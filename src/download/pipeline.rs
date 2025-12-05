use super::{
    BeatmapStage, BeatmapTracker, CleanupTracker, DownloadConfig, DownloadError, DownloadEvent,
    DownloadHandle, DownloadId, DownloadRequest, DownloadStage, DownloadSummary,
    SelectiveDownloadRequest, ShutdownToken,
    constants::DEFAULT_PROGRESS_WATCHDOG_SECS,
    http_client,
    integrity::ExpectationIndex,
    passes::{FailureReport, PassCoordinator},
    precheck::{PrecheckOptions, PrecheckReport, verify_existing_beatmapsets},
};
use crate::{
    core::collection::{
        CollectionService, HttpCollectionService, create_collection_db,
        generate_collection_folder_name, model::Collection,
    },
    mirrors::MirrorEndpoint,
    utils::{
        self, AppError, check_available_space, is_low_disk_space, validate_and_prepare_directory,
    },
    worker::{DownloadContext, MirrorPool, StatusSink},
};
use dashmap::DashSet;
use fs2::FileExt;
use std::{
    collections::{HashMap, HashSet},
    fs::{File as StdFile, OpenOptions},
    future::Future,
    io,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::{fs, sync::mpsc::UnboundedSender};
use tracing::Instrument;
use tracing::{debug, error, info, info_span, warn};

const DIRECTORY_LOCK_FILE: &str = ".osu-collect.lock";

/// Global tracker for active downloads to prevent concurrent downloads to the same directory
static ACTIVE_DOWNLOADS: LazyLock<DashSet<PathBuf>> = LazyLock::new(DashSet::new);

fn log_status(status: &StatusSink, id: DownloadId, message: impl Into<String>) {
    status.emit(DownloadEvent::Log {
        id,
        message: message.into(),
    });
}

fn stage_status(status: &StatusSink, id: DownloadId, stage: DownloadStage) {
    status.emit(DownloadEvent::StageChanged { id, stage });
}

fn fail_status(status: &StatusSink, id: DownloadId, message: impl Into<String>) {
    status.emit(DownloadEvent::Failed {
        id,
        message: message.into(),
    });
}

fn finished_status(status: &StatusSink, id: DownloadId, summary: &DownloadSummary) {
    status.emit(DownloadEvent::Finished {
        id,
        summary: summary.clone(),
    });
}

fn target_status(status: &StatusSink, id: DownloadId, remaining: usize) {
    status.emit(DownloadEvent::DownloadTarget { id, remaining });
}

fn progress_status(status: &StatusSink, id: DownloadId, summary: &DownloadSummary) {
    status.emit(DownloadEvent::OverallProgress {
        id,
        downloaded: summary.downloaded,
        skipped: summary.skipped,
        failed: summary.failed,
        unverified: summary.unverified,
    });
}

fn verified_sizes_status(status: &StatusSink, id: DownloadId, total_bytes: u64) {
    status.emit(DownloadEvent::VerifiedMapSizes { id, total_bytes });
}

fn low_disk_space_status(status: &StatusSink, id: DownloadId, available_bytes: u64) {
    status.emit(DownloadEvent::LowDiskSpace {
        id,
        available_bytes,
    });
}

struct DownloadLockGuard {
    path: PathBuf,
    lock_file_path: PathBuf,
    file: Option<StdFile>,
}

impl DownloadLockGuard {
    fn acquire(path: &Path) -> Result<Self, DownloadError> {
        let key = path.to_path_buf();
        if !ACTIVE_DOWNLOADS.insert(key.clone()) {
            return Err(DownloadError::ConcurrentDownload(
                key.to_string_lossy().into_owned(),
            ));
        }

        match Self::lock_directory(&key) {
            Ok((file, lock_file_path)) => Ok(Self {
                path: key,
                lock_file_path,
                file: Some(file),
            }),
            Err(err) => {
                ACTIVE_DOWNLOADS.remove(&key);
                Err(err)
            }
        }
    }

    fn lock_directory(path: &Path) -> Result<(StdFile, PathBuf), DownloadError> {
        let lock_file_path = path.join(DIRECTORY_LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_file_path)
            .map_err(DownloadError::from)?;

        if let Err(err) = file.try_lock_exclusive() {
            let kind = err.kind();
            drop(file);
            if kind == io::ErrorKind::WouldBlock {
                return Err(DownloadError::ConcurrentDownload(
                    path.to_string_lossy().into_owned(),
                ));
            }
            return Err(DownloadError::Io(err));
        }

        Ok((file, lock_file_path))
    }
}

impl Drop for DownloadLockGuard {
    fn drop(&mut self) {
        if let Some(file) = self.file.take()
            && let Err(err) = file.unlock()
        {
            warn!(
                directory = %self.path.display(),
                error = %err,
                "Failed to release directory lock"
            );
        }

        if let Err(err) = std::fs::remove_file(&self.lock_file_path)
            && err.kind() != io::ErrorKind::NotFound
        {
            warn!(
                file = %self.lock_file_path.display(),
                error = %err,
                "Failed to remove directory lock file"
            );
        }

        ACTIVE_DOWNLOADS.remove(&self.path);
    }
}

fn spawn_download_task<F, Fut>(
    id: DownloadId,
    span: tracing::Span,
    tx: UnboundedSender<DownloadEvent>,
    runner: F,
) -> DownloadHandle
where
    F: FnOnce(ShutdownToken, StatusSink) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), DownloadError>> + Send,
{
    let shutdown = ShutdownToken::new();
    let shutdown_worker = shutdown.clone();
    let handle_token = shutdown.clone();
    let status = StatusSink::from_sender(tx);
    let failure_status = status.clone();

    let join_handle = tokio::spawn(
        async move {
            info!("Download task started");
            match runner(shutdown_worker.clone(), status).await {
                Ok(()) => {
                    shutdown_worker.mark_completed();
                    info!("Download task completed");
                }
                Err(err) => {
                    shutdown_worker.mark_completed();
                    error!(error = %err, "Download task failed");
                    fail_status(&failure_status, id, err.to_string());
                }
            }
        }
        .instrument(span),
    );

    DownloadHandle {
        shutdown: handle_token,
        join_handle,
    }
}

pub fn spawn_download(
    id: DownloadId,
    request: DownloadRequest,
    tx: UnboundedSender<DownloadEvent>,
) -> DownloadHandle {
    let mirror_count = request.config.mirrors.len();
    let concurrent = request.config.concurrent;
    let span = info_span!(
        "download_task",
        download_id = id,
        mirror_count = mirror_count,
        concurrent = concurrent
    );
    {
        let _guard = span.enter();
        info!(
            collection_input = %request.collection_input,
            target_directory = %request.config.directory,
            skip_existing = request.skip_existing,
            auto_overwrite = request.auto_overwrite,
            "Spawning download task"
        );
    }

    spawn_download_task(id, span, tx, move |shutdown, status| async move {
        run_download(id, request, shutdown, status).await
    })
}

pub fn spawn_selective_download(
    id: DownloadId,
    request: SelectiveDownloadRequest,
    tx: UnboundedSender<DownloadEvent>,
) -> DownloadHandle {
    let mirror_count = request.config.mirrors.len();
    let concurrent = request.config.concurrent;
    let beatmapset_count = request.beatmapset_ids.len();
    let span = info_span!(
        "selective_download_task",
        download_id = id,
        mirror_count = mirror_count,
        concurrent = concurrent,
        beatmapset_count = beatmapset_count
    );
    {
        let _guard = span.enter();
        info!(
            target_directory = %request.config.directory,
            collection_count = request.collection_ids.len(),
            "Spawning selective download task"
        );
    }

    spawn_download_task(id, span, tx, move |shutdown, status| async move {
        run_selective_download(id, request, shutdown, status).await
    })
}

struct OutputPreparation {
    output_dir: PathBuf,
    display: String,
}
enum SessionTarget {
    Collection(Collection),
    Selective { collection_names: Vec<String> },
}

impl SessionTarget {
    fn expectation_index(&self, beatmapset_ids: &[u32]) -> Arc<ExpectationIndex> {
        match self {
            SessionTarget::Collection(collection) => {
                Arc::new(ExpectationIndex::new(&collection.beatmapsets))
            }
            SessionTarget::Selective { .. } => Arc::new(ExpectationIndex::from_ids(beatmapset_ids)),
        }
    }

    fn announce_ready(
        &self,
        status: &StatusSink,
        id: DownloadId,
        output: &OutputPreparation,
        beatmapset_ids: &[u32],
    ) {
        match self {
            SessionTarget::Collection(collection) => {
                status.emit(DownloadEvent::CollectionReady {
                    id,
                    collection_name: collection.name.to_string(),
                    uploader: collection.uploader.username.to_string(),
                    total_maps: collection.beatmapsets.len(),
                    output_dir: output.display.clone(),
                });
                log_status(status, id, format!("Downloading to {}", output.display));
            }
            SessionTarget::Selective { collection_names } => {
                let collection_name = if collection_names.len() == 1 {
                    format!("Update: {}", collection_names[0])
                } else {
                    format!("Update: {} collections", collection_names.len())
                };
                status.emit(DownloadEvent::CollectionReady {
                    id,
                    collection_name,
                    uploader: "Updates".to_string(),
                    total_maps: 0,
                    output_dir: output.display.clone(),
                });
                log_status(
                    status,
                    id,
                    format!("Downloading updates to {}", output.display),
                );
            }
        }

        status.emit(DownloadEvent::BeatmapsRegistered {
            id,
            beatmap_ids: beatmapset_ids.to_vec(),
        });
    }

    fn collection(&self) -> Option<&Collection> {
        match self {
            SessionTarget::Collection(collection) => Some(collection),
            SessionTarget::Selective { .. } => None,
        }
    }
}

struct DownloadSession {
    id: DownloadId,
    status: StatusSink,
    target: SessionTarget,
    beatmapset_ids: Vec<u32>,
    output: OutputPreparation,
    tracker: BeatmapTracker,
    totals: DownloadSummary,
    initial_unverified: Arc<DashSet<u32>>,
    _lock_guard: DownloadLockGuard,
}

impl DownloadSession {
    #[allow(clippy::too_many_arguments)]
    async fn prepare_collection(
        id: DownloadId,
        status: StatusSink,
        shutdown: &ShutdownToken,
        directory: &str,
        collection_input: &str,
        thread_count: usize,
        verify_zip_eocd: bool,
        flavor: &PipelineFlavor,
    ) -> Result<Option<Self>, DownloadError> {
        let collection = resolve_collection(collection_input).await?;
        let beatmapset_ids: Vec<u32> = collection.beatmapsets.iter().map(|b| b.id).collect();
        let output = prepare_output_directory(directory, &collection).await?;
        let lock_guard = DownloadLockGuard::acquire(&output.output_dir)?;
        let target = SessionTarget::Collection(collection);
        target.announce_ready(&status, id, &output, &beatmapset_ids);

        Self::finalize(
            id,
            status,
            shutdown,
            target,
            beatmapset_ids,
            output,
            lock_guard,
            thread_count,
            verify_zip_eocd,
            flavor,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn prepare_selective(
        id: DownloadId,
        status: StatusSink,
        shutdown: &ShutdownToken,
        directory: &str,
        collection_ids: &[u32],
        beatmapset_ids: &[u32],
        thread_count: usize,
        verify_zip_eocd: bool,
        flavor: &PipelineFlavor,
    ) -> Result<Option<Self>, DownloadError> {
        let collection_names =
            resolve_selective_collections(collection_ids, beatmapset_ids).await?;
        let output = prepare_selective_output_directory(directory, collection_ids).await?;
        let lock_guard = DownloadLockGuard::acquire(&output.output_dir)?;
        let mut target_ids = beatmapset_ids.to_vec();
        target_ids.sort_unstable();
        let target = SessionTarget::Selective { collection_names };
        target.announce_ready(&status, id, &output, &target_ids);

        Self::finalize(
            id,
            status,
            shutdown,
            target,
            target_ids,
            output,
            lock_guard,
            thread_count,
            verify_zip_eocd,
            flavor,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn finalize(
        id: DownloadId,
        status: StatusSink,
        shutdown: &ShutdownToken,
        target: SessionTarget,
        beatmapset_ids: Vec<u32>,
        output: OutputPreparation,
        lock_guard: DownloadLockGuard,
        thread_count: usize,
        verify_zip_eocd: bool,
        flavor: &PipelineFlavor,
    ) -> Result<Option<Self>, DownloadError> {
        let expectations = target.expectation_index(&beatmapset_ids);
        let precheck = perform_initial_precheck(
            &status,
            id,
            &output.output_dir,
            expectations,
            thread_count,
            verify_zip_eocd,
            shutdown,
        )
        .await?;

        if precheck.aborted {
            log_status(&status, id, flavor.precheck_abort_log);
            fail_status(&status, id, "Download aborted by user");
            return Ok(None);
        }

        if precheck.files_changed {
            log_status(
                &status,
                id,
                "Files changed during precheck; rescheduling affected beatmapsets",
            );
        }

        let PrecheckReport {
            satisfied,
            skipped,
            unverified,
            verified_bytes,
            ..
        } = precheck;

        let initial_unverified: Arc<DashSet<u32>> =
            Arc::new(DashSet::with_capacity(unverified.len()));
        for id in &unverified {
            initial_unverified.insert(*id);
        }

        if verified_bytes > 0 {
            verified_sizes_status(&status, id, verified_bytes);
        }

        let pending_ids: HashSet<u32> = beatmapset_ids
            .iter()
            .copied()
            .filter(|beatmap_id| !satisfied.contains(beatmap_id))
            .collect();
        let tracker = BeatmapTracker::with_verified(pending_ids.clone(), satisfied);

        target_status(&status, id, pending_ids.len());

        let totals = DownloadSummary {
            downloaded: 0,
            skipped,
            failed: 0,
            unverified: initial_unverified.len() as u32,
        };

        if totals.skipped > 0 {
            log_status(
                &status,
                id,
                format!("{} beatmapsets already verified locally", totals.skipped),
            );
            progress_status(&status, id, &totals);
        }

        Ok(Some(DownloadSession {
            id,
            status,
            target,
            beatmapset_ids,
            output,
            tracker,
            totals,
            initial_unverified,
            _lock_guard: lock_guard,
        }))
    }
}

#[derive(Clone, Copy, Debug)]
struct PipelineFlavor {
    precheck_abort_log: &'static str,
    abort_log_message: Option<&'static str>,
    abort_warning: Option<&'static str>,
    log_prefix: &'static str,
    failure_summary: &'static str,
    completion_log: &'static str,
}

impl PipelineFlavor {
    const fn collection() -> Self {
        Self {
            precheck_abort_log: "Download aborted during precheck",
            abort_log_message: Some("Download aborted before completion"),
            abort_warning: Some("Download aborted due to shutdown request"),
            log_prefix: "Starting",
            failure_summary: "Download completed with failed beatmapsets",
            completion_log: "Download pipeline finished and summary dispatched",
        }
    }

    const fn selective() -> Self {
        Self {
            precheck_abort_log: "Selective download aborted during precheck",
            abort_log_message: None,
            abort_warning: None,
            log_prefix: "Starting selective",
            failure_summary: "Selective download completed with failed beatmapsets",
            completion_log: "Selective download pipeline finished and summary dispatched",
        }
    }
}

async fn prepare_output_dir_common(
    base_path: &str,
    folder_name: &str,
) -> Result<OutputPreparation, DownloadError> {
    let normalized = {
        let trimmed = base_path.trim();
        if trimmed.is_empty() { "." } else { trimmed }
    };

    let base_dir = validate_and_prepare_directory(normalized).await?;
    debug!(base = %base_dir.display(), "Validated base download directory");

    let output_dir = base_dir.join(folder_name);
    fs::create_dir_all(&output_dir).await?;
    let output_dir_display = output_dir.to_string_lossy().to_string();
    info!(output_dir = %output_dir_display, "Prepared output directory");

    Ok(OutputPreparation {
        output_dir,
        display: output_dir_display,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_download_context(
    id: DownloadId,
    thread_count: usize,
    max_retries: u8,
    skip_existing: bool,
    auto_overwrite: bool,
    verify_zip_eocd: bool,
    client: reqwest::Client,
    shutdown: ShutdownToken,
    mirrors: Vec<MirrorEndpoint>,
    tracker: BeatmapTracker,
    output_dir: PathBuf,
    initial_unverified: Arc<DashSet<u32>>,
    status: StatusSink,
) -> Result<DownloadContext, DownloadError> {
    validate_mirrors(&mirrors)?;
    let progress_watchdog = Duration::from_secs(DEFAULT_PROGRESS_WATCHDOG_SECS);
    Ok(DownloadContext::new(
        id,
        thread_count,
        skip_existing,
        auto_overwrite,
        verify_zip_eocd,
        max_retries,
        shutdown,
        client,
        MirrorPool::new(mirrors),
        output_dir,
        tracker,
        initial_unverified,
        status,
        progress_watchdog,
    ))
}

async fn run_download_loop(
    ctx: &DownloadContext,
    totals: &mut DownloadSummary,
    log_prefix: &str,
) -> FailureReport {
    let mut final_failures = FailureReport::default();
    let mut attempts: HashMap<u32, u8> = HashMap::new();
    let mut pass_index: u32 = 1;
    let max_attempts = ctx.max_retries.max(1);

    loop {
        if ctx.shutdown.is_cancelled() {
            break;
        }

        let pending = ctx.tracker.pending_snapshot();
        if pending.is_empty() {
            break;
        }

        info!(
            download_id = ctx.id,
            remaining = pending.len(),
            pass = pass_index,
            thread_count = ctx.thread_count,
            "{} download pass",
            log_prefix
        );

        let pass_outcome = PassCoordinator::new(ctx.clone(), totals).run(pending).await;

        if pass_outcome.aborted {
            warn!("{} aborted during pass", log_prefix);
            for (beatmapset_id, reason) in pass_outcome.failures.beatmaps() {
                final_failures.record_error(*beatmapset_id, reason.clone());
            }
            break;
        } else if let Some(summary) = pass_outcome.failures.describe_top_failure() {
            ctx.emit(DownloadEvent::Log {
                id: ctx.id,
                message: format!("Most common failure this pass: {}", summary),
            });
        }

        if pass_outcome.failures.is_empty() {
            pass_index = pass_index.saturating_add(1);
            continue;
        }

        for (beatmapset_id, reason) in pass_outcome.failures.beatmaps() {
            if ctx.shutdown.is_cancelled() {
                final_failures.record_error(*beatmapset_id, reason.clone());
                continue;
            }

            let attempt_entry = attempts.entry(*beatmapset_id).or_insert(1);
            let can_retry = *attempt_entry < max_attempts;

            if can_retry && ctx.tracker.mark_pending(*beatmapset_id) {
                *attempt_entry += 1;
                totals.failed = totals.failed.saturating_sub(1);
                let attempt_label = *attempt_entry;
                ctx.emit(DownloadEvent::BeatmapStatus {
                    id: ctx.id,
                    beatmapset_id: *beatmapset_id,
                    stage: BeatmapStage::Pending,
                    message: format!("Retrying download ({}/{})", attempt_label, max_attempts),
                });
                ctx.emit(DownloadEvent::Log {
                    id: ctx.id,
                    message: format!(
                        "Retrying #{} ({}/{}) after: {}",
                        beatmapset_id, attempt_label, max_attempts, reason
                    ),
                });
                continue;
            }

            final_failures.record_error(*beatmapset_id, reason.clone());
        }

        pass_index = pass_index.saturating_add(1);
    }

    final_failures
}

#[allow(clippy::too_many_arguments)]
async fn run_download_core(
    session: DownloadSession,
    shutdown: ShutdownToken,
    mirrors: Vec<MirrorEndpoint>,
    concurrent: u8,
    max_retries: u8,
    skip_existing: bool,
    auto_overwrite: bool,
    verify_zip_eocd: bool,
    flavor: PipelineFlavor,
) -> Result<(), DownloadError> {
    let DownloadSession {
        id,
        status,
        target,
        beatmapset_ids,
        output,
        tracker,
        mut totals,
        initial_unverified,
        _lock_guard,
    } = session;
    let _session_lock = _lock_guard;

    log_status(&status, id, "Fetching collection size from Nekoha...");
    let api_client = http_client::api_client()?;
    let size_result =
        super::size_fetcher::fetch_beatmapset_sizes(&api_client, &beatmapset_ids).await;
    status.emit(DownloadEvent::CollectionSizeResolved {
        id,
        total_bytes: size_result.total_bytes,
    });
    if size_result.missing_count > 0 {
        log_status(
            &status,
            id,
            format!(
                "Size info unavailable for {} beatmapsets",
                size_result.missing_count
            ),
        );
    }

    check_and_warn_low_disk_space(&status, id, &output.output_dir);

    let download_client = http_client::download_client()?;
    let thread_count = concurrent.max(1) as usize;
    let ctx = build_download_context(
        id,
        thread_count,
        max_retries,
        skip_existing,
        auto_overwrite,
        verify_zip_eocd,
        download_client,
        shutdown.clone(),
        mirrors,
        tracker,
        output.output_dir.clone(),
        initial_unverified,
        status.clone(),
    )?;

    let failure_report = run_download_loop(&ctx, &mut totals, flavor.log_prefix).await;
    ctx.tracker.clear_validation_cache();

    if abort_if_shutdown(
        &status,
        id,
        &shutdown,
        &ctx,
        &failure_report,
        flavor.abort_log_message,
    )
    .await
    {
        if let Some(warning) = flavor.abort_warning {
            warn!("{}", warning);
        }
        return Ok(());
    }

    if let Some(collection) = target.collection() {
        match create_collection_database(collection, ctx.output_dir.as_ref().as_path()) {
            Ok(()) => {
                log_status(&status, id, "collection.db created successfully");
                info!("collection.db created successfully");
            }
            Err(e) => {
                log_status(
                    &status,
                    id,
                    format!("Warning: Failed to create collection.db: {}", e),
                );
                warn!(error = %e, "Failed to create collection.db");
            }
        }
    }

    summarize_failed_maps(&status, id, &failure_report, flavor.failure_summary);

    finished_status(&status, id, &totals);
    stage_status(&status, id, DownloadStage::Completed);
    info!("{}", flavor.completion_log);
    Ok(())
}

async fn abort_if_shutdown(
    status: &StatusSink,
    id: DownloadId,
    shutdown: &ShutdownToken,
    ctx: &DownloadContext,
    failures: &FailureReport,
    log_message: Option<&str>,
) -> bool {
    if !shutdown.is_cancelled() {
        return false;
    }

    handle_shutdown_cleanup(
        status,
        id,
        failures,
        &ctx.cleanup_tracker,
        ctx.output_dir.as_ref().as_path(),
    )
    .await;
    if let Some(message) = log_message {
        log_status(status, id, message);
    }
    fail_status(status, id, "Download aborted by user");
    true
}

fn summarize_failed_maps(
    status: &StatusSink,
    id: DownloadId,
    failures: &FailureReport,
    summary_message: &str,
) {
    if failures.is_empty() {
        return;
    }

    emit_failed_maps(status, id, failures);
    warn!(count = failures.beatmaps().len(), "{}", summary_message);
}

async fn handle_shutdown_cleanup(
    status: &StatusSink,
    id: DownloadId,
    failures: &FailureReport,
    cleanup_tracker: &CleanupTracker,
    output_dir: &Path,
) {
    emit_failed_maps(status, id, failures);
    let cleanup_outcome = cleanup_tracker.cleanup_incomplete().await;
    if cleanup_outcome.removed > 0 {
        info!(
            removed = cleanup_outcome.removed,
            "Removed incomplete beatmap archives"
        );
        log_status(
            status,
            id,
            format!(
                "Cleaned up {} incomplete beatmap archives",
                cleanup_outcome.removed
            ),
        );
    }
    for (path, message) in &cleanup_outcome.failures {
        warn!(target = %path.display(), error = %message, "Failed to cleanup file");
        log_status(
            status,
            id,
            format!("Cleanup warning for {}: {}", path.display(), message),
        );
    }

    match try_remove_empty_output_dir(output_dir).await {
        Ok(()) => {
            info!(dir = %output_dir.display(), "Removed empty output directory");
            log_status(
                status,
                id,
                format!("Removed empty directory {}", output_dir.display()),
            );
        }
        Err(DownloadError::DirectoryNotEmpty) => {}
        Err(err) => {
            warn!(dir = %output_dir.display(), error = %err, "Failed to remove output directory");
        }
    }
}

async fn try_remove_empty_output_dir(output_dir: &Path) -> Result<(), DownloadError> {
    let mut entries = fs::read_dir(output_dir).await?;

    if entries.next_entry().await?.is_some() {
        return Err(DownloadError::DirectoryNotEmpty);
    }

    fs::remove_dir(output_dir).await?;
    Ok(())
}

fn validate_mirrors(mirrors: &[MirrorEndpoint]) -> Result<(), DownloadError> {
    if mirrors.is_empty() {
        warn!("Download request did not include any mirrors");
        return Err(DownloadError::NoMirrors);
    }
    Ok(())
}

fn check_and_warn_low_disk_space(status: &StatusSink, id: DownloadId, output_dir: &Path) {
    if is_low_disk_space(output_dir)
        && let Some(available) = check_available_space(output_dir)
    {
        warn!(
            available_bytes = available,
            output_dir = %output_dir.display(),
            "Low disk space detected"
        );
        low_disk_space_status(status, id, available);
    }
}

async fn run_download(
    id: DownloadId,
    request: DownloadRequest,
    shutdown: ShutdownToken,
    status: StatusSink,
) -> Result<(), DownloadError> {
    let DownloadRequest {
        collection_input,
        config,
        skip_existing,
        auto_overwrite,
    } = request;
    let DownloadConfig {
        directory,
        mirrors,
        concurrent,
        verify_zip_eocd,
        max_retries,
        ..
    } = config;

    info!(
        collection_input = %collection_input,
        concurrent,
        mirror_count = mirrors.len(),
        skip_existing,
        auto_overwrite,
        "Running download pipeline"
    );

    stage_status(&status, id, DownloadStage::Resolving);
    let flavor = PipelineFlavor::collection();
    let thread_count = concurrent.max(1) as usize;

    let session = DownloadSession::prepare_collection(
        id,
        status.clone(),
        &shutdown,
        &directory,
        &collection_input,
        thread_count,
        verify_zip_eocd,
        &flavor,
    )
    .await?;

    let Some(session) = session else {
        return Ok(());
    };

    run_download_core(
        session,
        shutdown,
        mirrors,
        concurrent,
        max_retries,
        skip_existing,
        auto_overwrite,
        verify_zip_eocd,
        flavor,
    )
    .await
}

async fn run_selective_download(
    id: DownloadId,
    request: SelectiveDownloadRequest,
    shutdown: ShutdownToken,
    status: StatusSink,
) -> Result<(), DownloadError> {
    let SelectiveDownloadRequest {
        collection_ids,
        beatmapset_ids,
        config,
    } = request;
    let DownloadConfig {
        directory,
        mirrors,
        concurrent,
        verify_zip_eocd,
        max_retries,
        ..
    } = config;

    info!(
        collection_count = collection_ids.len(),
        beatmapset_count = beatmapset_ids.len(),
        concurrent,
        mirror_count = mirrors.len(),
        "Running selective download pipeline"
    );

    if beatmapset_ids.is_empty() {
        return Err(DownloadError::NoBeatmapsets);
    }

    stage_status(&status, id, DownloadStage::Resolving);
    let flavor = PipelineFlavor::selective();
    let thread_count = concurrent.max(1) as usize;

    let session = DownloadSession::prepare_selective(
        id,
        status.clone(),
        &shutdown,
        &directory,
        &collection_ids,
        &beatmapset_ids,
        thread_count,
        verify_zip_eocd,
        &flavor,
    )
    .await?;

    let Some(session) = session else {
        return Ok(());
    };

    run_download_core(
        session,
        shutdown,
        mirrors,
        concurrent,
        max_retries,
        true,
        false,
        verify_zip_eocd,
        flavor,
    )
    .await
}

async fn resolve_selective_collections(
    collection_ids: &[u32],
    beatmapset_ids: &[u32],
) -> Result<Vec<String>, DownloadError> {
    let collection_service = HttpCollectionService::builder().build()?;

    let mut collection_names = Vec::new();
    let target_set: HashSet<u32> = beatmapset_ids.iter().copied().collect();
    let mut matched_count = 0;

    for &collection_id in collection_ids {
        match collection_service.fetch_collection(collection_id).await {
            Ok(collection) => {
                collection_names.push(collection.name.to_string());

                for beatmapset in &collection.beatmapsets {
                    if target_set.contains(&beatmapset.id) {
                        matched_count += 1;
                    }
                }
            }
            Err(err) => {
                warn!(
                    collection_id,
                    error = %err,
                    "Skipping missing/inaccessible collection in selective download"
                );
            }
        }
    }

    info!(
        collection_count = collection_ids.len(),
        resolved_count = collection_names.len(),
        matched_beatmapsets = matched_count,
        "Resolved selective collections"
    );

    Ok(collection_names)
}

async fn prepare_selective_output_directory(
    directory: &str,
    collection_ids: &[u32],
) -> Result<OutputPreparation, DownloadError> {
    let folder_name = if collection_ids.len() == 1 {
        format!("update-{}", collection_ids[0])
    } else {
        format!("update-{}-collections", collection_ids.len())
    };
    prepare_output_dir_common(directory, &folder_name).await
}

async fn perform_initial_precheck(
    status: &StatusSink,
    id: DownloadId,
    output_dir: &Path,
    expectations: Arc<ExpectationIndex>,
    thread_count: usize,
    verify_zip_eocd: bool,
    shutdown: &ShutdownToken,
) -> Result<PrecheckReport, DownloadError> {
    log_status(status, id, "Verifying existing beatmapsets on disk");
    stage_status(status, id, DownloadStage::Rechecking);
    info!("Starting disk precheck before downloads");
    let options = PrecheckOptions {
        verify_integrity: true,
        notify_verified: true,
        verify_zip_eocd,
    };
    let report = verify_existing_beatmapsets(
        id,
        output_dir,
        expectations,
        thread_count,
        options,
        shutdown,
        status,
    )
    .await?;
    if report.aborted {
        info!("Disk precheck aborted by shutdown");
    } else {
        info!(
            verified = report.satisfied.len(),
            skipped = report.skipped,
            "Finished initial disk precheck"
        );
    }
    stage_status(status, id, DownloadStage::Downloading);
    Ok(report)
}

fn emit_failed_maps(status: &StatusSink, id: DownloadId, failures: &FailureReport) {
    if failures.is_empty() {
        return;
    }

    status.emit(DownloadEvent::FailedMaps {
        id,
        failures: failures.beatmaps().to_vec(),
    });
}

fn create_collection_database(collection: &Collection, output_dir: &Path) -> Result<(), AppError> {
    let db_collection_name = format!("{}-{}", collection.name, collection.id);
    create_collection_db(collection, &db_collection_name, output_dir)
}

async fn resolve_collection(collection_input: &str) -> Result<Collection, DownloadError> {
    let collection_id = utils::parse_collection_id(collection_input)?;
    debug!(collection_input = %collection_input, collection_id, "Parsed collection identifier");

    let collection_service = HttpCollectionService::builder().build()?;
    let collection = collection_service.fetch_collection(collection_id).await?;

    info!(
        collection_id,
        collection_name = %collection.name,
        total_maps = collection.beatmapsets.len(),
        "Fetched collection metadata"
    );

    if collection.beatmapsets.is_empty() {
        warn!(collection_id, "Collection contained no beatmaps");
        return Err(DownloadError::EmptyCollection);
    }

    Ok(collection)
}

async fn prepare_output_directory(
    directory: &str,
    collection: &Collection,
) -> Result<OutputPreparation, DownloadError> {
    let folder_name = generate_collection_folder_name(collection);
    prepare_output_dir_common(directory, &folder_name).await
}
