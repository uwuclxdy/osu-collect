use super::{
    CleanupTracker, DownloadEvent, DownloadHandle, DownloadId, DownloadRequest, DownloadStage,
    DownloadSummary, OutstandingTracker, SelectiveDownloadRequest, VerifiedRegistry,
    create_download_client,
    integrity::ExpectationIndex,
    passes::{DownloadPassArgs, download_pass},
    precheck::{PrecheckReport, verify_existing_beatmapsets},
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
    worker::MirrorPool,
};
use std::{
    collections::HashSet,
    future::Future,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{fs, sync::mpsc::UnboundedSender};
use tracing::Instrument;
use tracing::{debug, error, info, info_span, warn};

pub(crate) trait EventSender {
    fn send_log(&self, id: DownloadId, message: impl Into<String>);
    fn send_stage(&self, id: DownloadId, stage: DownloadStage);
    fn send_failed(&self, id: DownloadId, message: impl Into<String>);
    fn send_finished(&self, id: DownloadId, summary: DownloadSummary);
    fn send_target(&self, id: DownloadId, remaining: usize);
    fn send_progress(
        &self,
        id: DownloadId,
        downloaded: u16,
        skipped: u16,
        failed: u16,
        unverified: u16,
    );
}

impl EventSender for UnboundedSender<DownloadEvent> {
    fn send_log(&self, id: DownloadId, message: impl Into<String>) {
        let _ = self.send(DownloadEvent::Log {
            id,
            message: message.into(),
        });
    }

    fn send_stage(&self, id: DownloadId, stage: DownloadStage) {
        let _ = self.send(DownloadEvent::StageChanged { id, stage });
    }

    fn send_failed(&self, id: DownloadId, message: impl Into<String>) {
        let _ = self.send(DownloadEvent::Failed {
            id,
            message: message.into(),
        });
    }

    fn send_finished(&self, id: DownloadId, summary: DownloadSummary) {
        let _ = self.send(DownloadEvent::Finished { id, summary });
    }

    fn send_target(&self, id: DownloadId, remaining: usize) {
        let _ = self.send(DownloadEvent::DownloadTarget { id, remaining });
    }

    fn send_progress(
        &self,
        id: DownloadId,
        downloaded: u16,
        skipped: u16,
        failed: u16,
        unverified: u16,
    ) {
        let _ = self.send(DownloadEvent::OverallProgress {
            id,
            downloaded,
            skipped,
            failed,
            unverified,
        });
    }
}

fn spawn_download_task<F, Fut>(
    id: DownloadId,
    span: tracing::Span,
    tx: UnboundedSender<DownloadEvent>,
    runner: F,
) -> DownloadHandle
where
    F: FnOnce(Arc<AtomicBool>, UnboundedSender<DownloadEvent>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), String>> + Send,
{
    let tx_clone = tx.clone();
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_worker = shutdown.clone();

    let join_handle = tokio::spawn(
        async move {
            info!("Download task started");
            match runner(shutdown_worker, tx).await {
                Ok(()) => info!("Download task completed"),
                Err(err) => {
                    error!(error = %err, "Download task failed");
                    let _ = tx_clone.send(DownloadEvent::Failed { id, message: err });
                }
            }
        }
        .instrument(span),
    );

    DownloadHandle {
        shutdown,
        join_handle,
    }
}

pub fn spawn_download(
    id: DownloadId,
    request: DownloadRequest,
    tx: UnboundedSender<DownloadEvent>,
) -> DownloadHandle {
    let mirror_count = request.mirrors.len();
    let concurrent = request.concurrent;
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
            target_directory = %request.directory,
            skip_existing = request.skip_existing,
            auto_overwrite = request.auto_overwrite,
            "Spawning download task"
        );
    }

    spawn_download_task(id, span, tx, move |shutdown, tx| async move {
        run_download(id, request, shutdown, &tx).await
    })
}

pub fn spawn_selective_download(
    id: DownloadId,
    request: SelectiveDownloadRequest,
    tx: UnboundedSender<DownloadEvent>,
) -> DownloadHandle {
    let mirror_count = request.mirrors.len();
    let concurrent = request.concurrent;
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
            target_directory = %request.directory,
            collection_count = request.collection_ids.len(),
            "Spawning selective download task"
        );
    }

    spawn_download_task(id, span, tx, move |shutdown, tx| async move {
        run_selective_download(id, request, shutdown, &tx).await
    })
}

struct CollectionResolution {
    collection: Collection,
    beatmap_ids: Vec<u32>,
    expectation_index: Arc<ExpectationIndex>,
}

struct OutputPreparation {
    output_dir: PathBuf,
    display: String,
}

async fn prepare_output_dir_common(
    base_path: &str,
    folder_name: &str,
) -> Result<OutputPreparation, String> {
    let normalized = {
        let trimmed = base_path.trim();
        if trimmed.is_empty() { "." } else { trimmed }
    };

    let base_dir = validate_and_prepare_directory(normalized)
        .await
        .map_err(|e| format!("{}", e))?;
    debug!(base = %base_dir.display(), "Validated base download directory");

    let output_dir = base_dir.join(folder_name);
    fs::create_dir_all(&output_dir)
        .await
        .map_err(|e| format!("{}", AppError::from(e)))?;
    let output_dir_display = output_dir.to_string_lossy().to_string();
    info!(output_dir = %output_dir_display, "Prepared output directory");

    Ok(OutputPreparation {
        output_dir,
        display: output_dir_display,
    })
}

struct FailureTracker {
    failed: Vec<u32>,
    seen: HashSet<u32>,
}

impl FailureTracker {
    fn new() -> Self {
        Self {
            failed: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn record(&mut self, beatmap_ids: Vec<u32>) {
        for id in beatmap_ids {
            if self.seen.insert(id) {
                self.failed.push(id);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.failed.is_empty()
    }

    fn len(&self) -> usize {
        self.failed.len()
    }

    fn as_slice(&self) -> &[u32] {
        &self.failed
    }
}

struct DownloadPassContext {
    id: DownloadId,
    thread_count: usize,
    skip_existing: bool,
    auto_overwrite: bool,
    shutdown: Arc<AtomicBool>,
    client: reqwest::Client,
    mirror_pool: MirrorPool,
    output_dir: Arc<PathBuf>,
    expectations: Arc<ExpectationIndex>,
    verified: VerifiedRegistry,
    outstanding: OutstandingTracker,
    cleanup_tracker: CleanupTracker,
    tx: UnboundedSender<DownloadEvent>,
}

struct DownloadContextInputs {
    id: DownloadId,
    thread_count: usize,
    skip_existing: bool,
    auto_overwrite: bool,
    shutdown: Arc<AtomicBool>,
    mirrors: Vec<MirrorEndpoint>,
    expectations: Arc<ExpectationIndex>,
    verified: VerifiedRegistry,
    outstanding: OutstandingTracker,
    output_dir: PathBuf,
    tx: UnboundedSender<DownloadEvent>,
}

struct DownloadLoopResult {
    failure_tracker: FailureTracker,
}

fn build_download_context(inputs: DownloadContextInputs) -> Result<DownloadPassContext, String> {
    validate_mirrors(&inputs.mirrors)?;
    let client = create_download_client().map_err(|e| format!("{}", e))?;

    Ok(DownloadPassContext {
        id: inputs.id,
        thread_count: inputs.thread_count,
        skip_existing: inputs.skip_existing,
        auto_overwrite: inputs.auto_overwrite,
        shutdown: inputs.shutdown,
        client,
        mirror_pool: MirrorPool::new(inputs.mirrors),
        output_dir: Arc::new(inputs.output_dir),
        expectations: inputs.expectations,
        verified: inputs.verified,
        outstanding: inputs.outstanding,
        cleanup_tracker: CleanupTracker::new(),
        tx: inputs.tx,
    })
}

async fn run_download_loop(
    ctx: &DownloadPassContext,
    totals: &mut DownloadSummary,
    log_prefix: &str,
) -> DownloadLoopResult {
    let mut failure_tracker = FailureTracker::new();

    for (pass_number, is_retry) in [false, true].into_iter().enumerate() {
        if ctx.outstanding.is_empty().await || ctx.shutdown.load(Ordering::SeqCst) {
            break;
        }

        let remaining_targets = ctx.outstanding.len().await;
        if is_retry {
            ctx.tx.send_stage(ctx.id, DownloadStage::Rechecking);
            ctx.tx.send_log(
                ctx.id,
                format!(
                    "Starting retry pass {} ({} targets remaining)",
                    pass_number, remaining_targets
                ),
            );
            info!(
                attempt = pass_number,
                remaining = remaining_targets,
                "Starting retry pass"
            );
        } else {
            info!(
                remaining = remaining_targets,
                "{} primary download pass", log_prefix
            );
        }

        let targets = ctx.outstanding.snapshot().await;
        if targets.is_empty() {
            break;
        }

        let pass_result = {
            let args = DownloadPassArgs {
                id: ctx.id,
                beatmapset_ids: targets,
                thread_count: ctx.thread_count,
                skip_existing: ctx.skip_existing,
                auto_overwrite: ctx.auto_overwrite,
                shutdown: ctx.shutdown.clone(),
                client: ctx.client.clone(),
                mirror_pool: ctx.mirror_pool.clone(),
                output_dir: ctx.output_dir.clone(),
                expectations: ctx.expectations.clone(),
                verified: ctx.verified.clone(),
                outstanding: ctx.outstanding.clone(),
                cleanup_tracker: ctx.cleanup_tracker.clone(),
                retry_phase: is_retry,
                tx: ctx.tx.clone(),
            };
            download_pass(args, totals).await
        };

        failure_tracker.record(pass_result.failed_maps);

        if is_retry {
            ctx.tx.send_stage(ctx.id, DownloadStage::Downloading);
        }

        if pass_result.aborted {
            warn!("{} aborted during pass", log_prefix);
            break;
        }

        if is_retry && !ctx.outstanding.is_empty().await && !ctx.shutdown.load(Ordering::SeqCst) {
            let remaining = ctx.outstanding.len().await;
            warn!(
                remaining,
                "Reached maximum retry passes; outstanding beatmapsets remain"
            );
            ctx.tx.send_log(
                ctx.id,
                format!(
                    "Maximum retry passes reached with {} outstanding beatmapsets",
                    remaining
                ),
            );
        }
    }

    DownloadLoopResult { failure_tracker }
}

struct AbortContext<'a> {
    id: DownloadId,
    shutdown: &'a Arc<AtomicBool>,
    ctx: &'a DownloadPassContext,
    failures: &'a FailureTracker,
    tx: &'a UnboundedSender<DownloadEvent>,
}

async fn abort_if_shutdown(abort: AbortContext<'_>, log_message: Option<&str>) -> bool {
    if !abort.shutdown.load(Ordering::SeqCst) {
        return false;
    }

    handle_shutdown_cleanup(
        abort.id,
        abort.failures,
        &abort.ctx.cleanup_tracker,
        &abort.ctx.output_dir,
        abort.tx,
    )
    .await;
    if let Some(message) = log_message {
        abort.tx.send_log(abort.id, message.to_string());
    }
    abort.tx.send_failed(abort.id, "Download aborted by user");
    true
}

fn summarize_failed_maps(
    id: DownloadId,
    tracker: &FailureTracker,
    tx: &UnboundedSender<DownloadEvent>,
    summary_message: &str,
) {
    if tracker.is_empty() {
        return;
    }

    emit_failed_maps(tx, id, tracker);
    warn!(count = tracker.len(), "{}", summary_message);
}

async fn handle_shutdown_cleanup(
    id: DownloadId,
    failure_tracker: &FailureTracker,
    cleanup_tracker: &CleanupTracker,
    output_dir: &Path,
    tx: &UnboundedSender<DownloadEvent>,
) {
    emit_failed_maps(tx, id, failure_tracker);
    let cleanup_outcome = cleanup_tracker.cleanup_incomplete().await;
    if cleanup_outcome.removed > 0 {
        info!(
            removed = cleanup_outcome.removed,
            "Removed incomplete beatmap archives"
        );
        tx.send_log(
            id,
            format!(
                "Cleaned up {} incomplete beatmap archives",
                cleanup_outcome.removed
            ),
        );
    }
    for (path, message) in &cleanup_outcome.failures {
        warn!(target = %path.display(), error = %message, "Failed to cleanup file");
        tx.send_log(
            id,
            format!("Cleanup warning for {}: {}", path.display(), message),
        );
    }

    match try_remove_empty_output_dir(output_dir).await {
        Ok(()) => {
            info!(dir = %output_dir.display(), "Removed empty output directory");
            tx.send_log(
                id,
                format!("Removed empty directory {}", output_dir.display()),
            );
        }
        Err(err) if err == "Directory is not empty" => {}
        Err(err) => {
            warn!(dir = %output_dir.display(), error = %err, "Failed to remove output directory");
        }
    }
}

async fn try_remove_empty_output_dir(output_dir: &Path) -> Result<(), String> {
    let mut entries = fs::read_dir(output_dir).await.map_err(|e| e.to_string())?;

    if entries
        .next_entry()
        .await
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("Directory is not empty".to_string());
    }

    fs::remove_dir(output_dir).await.map_err(|e| e.to_string())
}

fn validate_mirrors(mirrors: &[MirrorEndpoint]) -> Result<(), String> {
    if mirrors.is_empty() {
        warn!("Download request did not include any mirrors");
        return Err("Select at least one mirror".to_string());
    }
    Ok(())
}

fn check_and_warn_low_disk_space(
    id: DownloadId,
    output_dir: &Path,
    tx: &UnboundedSender<DownloadEvent>,
) {
    if is_low_disk_space(output_dir)
        && let Some(available) = check_available_space(output_dir)
    {
        warn!(
            available_bytes = available,
            output_dir = %output_dir.display(),
            "Low disk space detected"
        );
        let _ = tx.send(DownloadEvent::LowDiskSpace {
            id,
            available_bytes: available,
        });
    }
}

async fn run_download(
    id: DownloadId,
    request: DownloadRequest,
    shutdown: Arc<AtomicBool>,
    tx: &UnboundedSender<DownloadEvent>,
) -> Result<(), String> {
    let DownloadRequest {
        collection_input,
        directory,
        mirrors,
        concurrent,
        skip_existing,
        auto_overwrite,
    } = request;

    info!(
        collection_input = %collection_input,
        concurrent,
        mirror_count = mirrors.len(),
        skip_existing,
        auto_overwrite,
        "Running download pipeline"
    );

    let resolution = resolve_collection(&collection_input).await?;
    let output = prepare_output_directory(&directory, &resolution.collection).await?;

    announce_collection_ready(id, &resolution, &output, tx);

    tx.send_log(id, "Fetching collection size from Nekoha...");
    let size_result = super::size_fetcher::fetch_beatmapset_sizes(&resolution.beatmap_ids).await;
    let _ = tx.send(DownloadEvent::CollectionSizeResolved {
        id,
        total_bytes: size_result.total_bytes,
    });
    if size_result.missing_count > 0 {
        tx.send_log(
            id,
            format!(
                "Size info unavailable for {} beatmapsets",
                size_result.missing_count
            ),
        );
    }

    check_and_warn_low_disk_space(id, &output.output_dir, tx);

    let thread_count = concurrent.max(1) as usize;
    let precheck = perform_initial_precheck(
        id,
        &output.output_dir,
        resolution.expectation_index.clone(),
        thread_count,
        tx,
    )
    .await?;

    let PrecheckReport {
        satisfied: pre_verified,
        skipped: initial_skipped,
        unverified: pre_unverified,
        verified_bytes,
    } = precheck;

    if verified_bytes > 0 {
        let _ = tx.send(DownloadEvent::VerifiedMapSizes {
            id,
            total_bytes: verified_bytes,
        });
    }

    let tracker = OutstandingTracker::new(resolution.beatmap_ids.iter().copied().collect());
    let remaining_after_precheck = tracker.remove_all(pre_verified.iter().copied()).await;

    let verified_registry = VerifiedRegistry::new(pre_verified);

    tx.send_target(id, remaining_after_precheck);

    let pre_unverified_count = pre_unverified.len().min(u16::MAX as usize) as u16;
    let mut totals = DownloadSummary {
        downloaded: 0,
        skipped: initial_skipped,
        failed: 0,
        unverified: pre_unverified_count,
    };

    if totals.skipped > 0 {
        tx.send_log(
            id,
            format!("{} beatmapsets already verified locally", totals.skipped),
        );
        tx.send_progress(
            id,
            totals.downloaded,
            totals.skipped,
            totals.failed,
            totals.unverified,
        );
    }

    let ctx = build_download_context(DownloadContextInputs {
        id,
        thread_count,
        skip_existing,
        auto_overwrite,
        shutdown: shutdown.clone(),
        mirrors,
        expectations: resolution.expectation_index.clone(),
        verified: verified_registry,
        outstanding: tracker,
        output_dir: output.output_dir.clone(),
        tx: tx.clone(),
    })?;

    let loop_result = run_download_loop(&ctx, &mut totals, "Starting").await;

    if abort_if_shutdown(
        AbortContext {
            id,
            shutdown: &shutdown,
            ctx: &ctx,
            failures: &loop_result.failure_tracker,
            tx,
        },
        Some("Download aborted before completion"),
    )
    .await
    {
        warn!("Download aborted due to shutdown request");
        return Ok(());
    }

    match create_collection_database(&resolution.collection, &output.output_dir) {
        Ok(()) => {
            tx.send_log(id, "collection.db created successfully");
            info!("collection.db created successfully");
        }
        Err(e) => {
            tx.send_log(
                id,
                format!("Warning: Failed to create collection.db: {}", e),
            );
            warn!(error = %e, "Failed to create collection.db");
        }
    }

    summarize_failed_maps(
        id,
        &loop_result.failure_tracker,
        tx,
        "Download completed with failed beatmapsets",
    );

    tx.send_finished(id, totals);
    info!("Download pipeline finished and summary dispatched");
    Ok(())
}

async fn run_selective_download(
    id: DownloadId,
    request: SelectiveDownloadRequest,
    shutdown: Arc<AtomicBool>,
    tx: &UnboundedSender<DownloadEvent>,
) -> Result<(), String> {
    let SelectiveDownloadRequest {
        collection_ids,
        beatmapset_ids,
        directory,
        mirrors,
        concurrent,
    } = request;

    info!(
        collection_count = collection_ids.len(),
        beatmapset_count = beatmapset_ids.len(),
        concurrent,
        mirror_count = mirrors.len(),
        "Running selective download pipeline"
    );

    if beatmapset_ids.is_empty() {
        return Err("No beatmapsets selected for download".to_string());
    }

    let resolution = resolve_selective_collections(&collection_ids, &beatmapset_ids).await?;
    let output = prepare_selective_output_directory(&directory, &collection_ids).await?;

    announce_selective_ready(id, &resolution, &output, tx);

    tx.send_log(id, "Fetching collection size from nekoha...");
    let size_result = super::size_fetcher::fetch_beatmapset_sizes(&beatmapset_ids).await;
    let _ = tx.send(DownloadEvent::CollectionSizeResolved {
        id,
        total_bytes: size_result.total_bytes,
    });
    if size_result.missing_count > 0 {
        tx.send_log(
            id,
            format!(
                "Size info unavailable for {} beatmapsets",
                size_result.missing_count
            ),
        );
    }

    check_and_warn_low_disk_space(id, &output.output_dir, tx);

    let thread_count = concurrent.max(1) as usize;
    let tracker = OutstandingTracker::new(beatmapset_ids.iter().copied().collect());
    let verified_registry = VerifiedRegistry::new(HashSet::new());

    tx.send_target(id, beatmapset_ids.len());

    let mut totals = DownloadSummary {
        downloaded: 0,
        skipped: 0,
        failed: 0,
        unverified: 0,
    };

    let ctx = build_download_context(DownloadContextInputs {
        id,
        thread_count,
        skip_existing: false,
        auto_overwrite: true,
        shutdown: shutdown.clone(),
        mirrors,
        expectations: resolution.expectation_index.clone(),
        verified: verified_registry,
        outstanding: tracker,
        output_dir: output.output_dir.clone(),
        tx: tx.clone(),
    })?;

    let loop_result = run_download_loop(&ctx, &mut totals, "Starting selective").await;

    if abort_if_shutdown(
        AbortContext {
            id,
            shutdown: &shutdown,
            ctx: &ctx,
            failures: &loop_result.failure_tracker,
            tx,
        },
        None,
    )
    .await
    {
        return Ok(());
    }

    summarize_failed_maps(
        id,
        &loop_result.failure_tracker,
        tx,
        "Selective download completed with failed beatmapsets",
    );

    tx.send_finished(id, totals);
    info!("Selective download pipeline finished and summary dispatched");
    Ok(())
}

struct SelectiveResolution {
    collection_names: Vec<String>,
    expectation_index: Arc<ExpectationIndex>,
}

async fn resolve_selective_collections(
    collection_ids: &[u32],
    beatmapset_ids: &[u32],
) -> Result<SelectiveResolution, String> {
    let collection_service = HttpCollectionService::builder()
        .build()
        .map_err(|e| e.to_string())?;

    let mut all_beatmapsets = Vec::new();
    let mut collection_names = Vec::new();
    let target_set: HashSet<u32> = beatmapset_ids.iter().copied().collect();

    for &collection_id in collection_ids {
        let collection = collection_service
            .fetch_collection(collection_id)
            .await
            .map_err(|e| e.to_string())?;

        collection_names.push(collection.name.to_string());

        for beatmapset in collection.beatmapsets {
            if target_set.contains(&beatmapset.id) {
                all_beatmapsets.push(beatmapset);
            }
        }
    }

    info!(
        collection_count = collection_ids.len(),
        matched_beatmapsets = all_beatmapsets.len(),
        "Resolved selective collections"
    );

    let expectation_index = Arc::new(ExpectationIndex::new(&all_beatmapsets));

    Ok(SelectiveResolution {
        collection_names,
        expectation_index,
    })
}

async fn prepare_selective_output_directory(
    directory: &str,
    collection_ids: &[u32],
) -> Result<OutputPreparation, String> {
    let folder_name = if collection_ids.len() == 1 {
        format!("update-{}", collection_ids[0])
    } else {
        format!("update-{}-collections", collection_ids.len())
    };
    prepare_output_dir_common(directory, &folder_name).await
}

fn announce_selective_ready(
    id: DownloadId,
    resolution: &SelectiveResolution,
    output: &OutputPreparation,
    tx: &UnboundedSender<DownloadEvent>,
) {
    let collection_name = if resolution.collection_names.len() == 1 {
        format!("Update: {}", resolution.collection_names[0])
    } else {
        format!("Update: {} collections", resolution.collection_names.len())
    };

    let _ = tx.send(DownloadEvent::CollectionReady {
        id,
        collection_name,
        uploader: "Updates".to_string(),
        total_maps: 0,
        output_dir: output.display.clone(),
    });
    tx.send_log(id, format!("Downloading updates to {}", output.display));
}

fn announce_collection_ready(
    id: DownloadId,
    resolution: &CollectionResolution,
    output: &OutputPreparation,
    tx: &UnboundedSender<DownloadEvent>,
) {
    let collection = &resolution.collection;
    let _ = tx.send(DownloadEvent::CollectionReady {
        id,
        collection_name: collection.name.to_string(),
        uploader: collection.uploader.username.to_string(),
        total_maps: collection.beatmapsets.len(),
        output_dir: output.display.clone(),
    });
    let _ = tx.send(DownloadEvent::BeatmapsRegistered {
        id,
        beatmap_ids: resolution.beatmap_ids.clone(),
    });
    tx.send_log(id, format!("Downloading to {}", output.display));
}

async fn perform_initial_precheck(
    id: DownloadId,
    output_dir: &Path,
    expectations: Arc<ExpectationIndex>,
    thread_count: usize,
    tx: &UnboundedSender<DownloadEvent>,
) -> Result<PrecheckReport, String> {
    tx.send_log(id, "Verifying existing beatmapsets on disk");
    tx.send_stage(id, DownloadStage::Rechecking);
    info!("Starting disk precheck before downloads");
    let report =
        verify_existing_beatmapsets(id, output_dir, expectations, thread_count, true, tx).await?;
    info!(
        verified = report.satisfied.len(),
        skipped = report.skipped,
        "Finished initial disk precheck"
    );
    tx.send_stage(id, DownloadStage::Downloading);
    Ok(report)
}

fn emit_failed_maps(tx: &UnboundedSender<DownloadEvent>, id: DownloadId, tracker: &FailureTracker) {
    if tracker.is_empty() {
        return;
    }

    let _ = tx.send(DownloadEvent::FailedMaps {
        id,
        beatmapset_ids: tracker.as_slice().to_vec(),
    });
}

fn create_collection_database(collection: &Collection, output_dir: &Path) -> Result<(), AppError> {
    let db_collection_name = format!("{}-{}", collection.name, collection.id);
    create_collection_db(collection, &db_collection_name, output_dir)
}

async fn resolve_collection(collection_input: &str) -> Result<CollectionResolution, String> {
    let collection_id =
        utils::parse_collection_id(collection_input).map_err(|e| format!("{}", e))?;
    debug!(collection_input = %collection_input, collection_id, "Parsed collection identifier");

    let collection_service = HttpCollectionService::builder()
        .build()
        .map_err(|e| e.to_string())?;
    let collection = collection_service
        .fetch_collection(collection_id)
        .await
        .map_err(|e| e.to_string())?;

    info!(
        collection_id,
        collection_name = %collection.name,
        total_maps = collection.beatmapsets.len(),
        "Fetched collection metadata"
    );

    if collection.beatmapsets.is_empty() {
        warn!(collection_id, "Collection contained no beatmaps");
        return Err("Collection does not contain any beatmaps".to_string());
    }

    let beatmap_ids: Vec<u32> = collection
        .beatmapsets
        .iter()
        .map(|beatmap| beatmap.id)
        .collect();
    let expectation_index = Arc::new(ExpectationIndex::new(&collection.beatmapsets));

    Ok(CollectionResolution {
        collection,
        beatmap_ids,
        expectation_index,
    })
}

async fn prepare_output_directory(
    directory: &str,
    collection: &Collection,
) -> Result<OutputPreparation, String> {
    let folder_name = generate_collection_folder_name(collection);
    prepare_output_dir_common(directory, &folder_name).await
}
