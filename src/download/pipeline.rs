use super::{
    DownloadConfig, DownloadError, DownloadEvent, DownloadHandle, DownloadId, DownloadRequest,
    DownloadStage, Emit, IdsDownloadRequest, SelectiveDownloadRequest,
    collection_db::{write_collection_db, write_selective_collection_db},
    events::{Tally, emit_finish, translate_event},
    fetch_collection_sizes,
    lock::ActiveDownloadRegistry,
    session::{DownloadSession, PrepareParams, PrepareTarget},
    warn_low_disk_space,
};
use crate::{
    app::{failed_maps, library_cache, snapshots},
    config::constants::{DEFAULT_PROGRESS_WATCHDOG_SECS, NETWORK_RETRY_CAP},
    osu_db::OsuClient,
};
use futures_util::StreamExt;
use osu_downloader::{
    Downloader, Event as LibEvent, Mirror, OnExists, Session as LibDownloadSession,
};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::{sync::mpsc::UnboundedSender, sync::watch};
use tracing::{Instrument, error, info, info_span, warn};

static DOWNLOAD_REGISTRY: LazyLock<ActiveDownloadRegistry> =
    LazyLock::new(ActiveDownloadRegistry::new);

const ABORTED_FAIL: &str = "download aborted by user";

pub fn spawn_download(
    id: DownloadId,
    request: DownloadRequest,
    tx: UnboundedSender<DownloadEvent>,
) -> DownloadHandle {
    let span = info_span!(
        "download_task",
        download_id = id,
        mirror_count = request.config.mirrors.len(),
        concurrent = request.config.concurrent
    );
    spawn(
        id,
        span,
        tx,
        move |cancel_rx, defer_rx, skip_rx, emit| async move {
            run_collection(id, request, cancel_rx, defer_rx, skip_rx, emit).await
        },
    )
}

pub fn spawn_selective_download(
    id: DownloadId,
    request: SelectiveDownloadRequest,
    tx: UnboundedSender<DownloadEvent>,
) -> DownloadHandle {
    let span = info_span!(
        "selective_download_task",
        download_id = id,
        mirror_count = request.config.mirrors.len(),
        concurrent = request.config.concurrent,
        beatmapset_count = request.beatmapset_ids.len()
    );
    spawn(
        id,
        span,
        tx,
        move |cancel_rx, defer_rx, skip_rx, emit| async move {
            run_selective(id, request, cancel_rx, defer_rx, skip_rx, emit).await
        },
    )
}

pub fn spawn_ids_download(
    id: DownloadId,
    request: IdsDownloadRequest,
    tx: UnboundedSender<DownloadEvent>,
) -> DownloadHandle {
    let span = info_span!(
        "ids_download_task",
        download_id = id,
        source = ?request.source,
        mirror_count = request.config.mirrors.len(),
        concurrent = request.config.concurrent,
        beatmapset_count = request.beatmapset_ids.len()
    );
    spawn(
        id,
        span,
        tx,
        move |cancel_rx, defer_rx, skip_rx, emit| async move {
            run_ids(id, request, cancel_rx, defer_rx, skip_rx, emit).await
        },
    )
}

type EmitArc = Arc<dyn Fn(DownloadEvent) + Send + Sync>;

fn spawn<F, Fut>(
    id: DownloadId,
    span: tracing::Span,
    tx: UnboundedSender<DownloadEvent>,
    runner: F,
) -> DownloadHandle
where
    F: FnOnce(watch::Receiver<bool>, watch::Receiver<u64>, watch::Receiver<u64>, EmitArc) -> Fut
        + Send
        + 'static,
    Fut: std::future::Future<Output = Result<(), DownloadError>> + Send,
{
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (defer_tx, defer_rx) = watch::channel(0u64);
    let (skip_tx, skip_rx) = watch::channel(0u64);
    let failure_tx = tx.clone();
    let emit: EmitArc = Arc::new(move |event: DownloadEvent| {
        let _ = tx.send(event);
    });

    let join = tokio::spawn(
        async move {
            info!("download task started");
            if let Err(err) = runner(cancel_rx, defer_rx, skip_rx, emit).await {
                error!(error = %err, "download task failed");
                let _ = failure_tx.send(DownloadEvent::Failed {
                    id,
                    message: err.to_string(),
                });
            } else {
                info!("download task completed");
            }
        }
        .instrument(span),
    );

    DownloadHandle::new(cancel_tx, defer_tx, skip_tx, join)
}

fn emit_resolving(id: DownloadId, emit: Emit<'_>) {
    emit(DownloadEvent::StageChanged {
        id,
        stage: DownloadStage::Resolving,
    });
}

async fn run_collection(
    id: DownloadId,
    request: DownloadRequest,
    cancel_rx: watch::Receiver<bool>,
    defer_rx: watch::Receiver<u64>,
    skip_rx: watch::Receiver<u64>,
    emit: EmitArc,
) -> Result<(), DownloadError> {
    let DownloadRequest {
        collection_input,
        config,
        auto_overwrite,
        // Carried into the pipeline for future use (e.g. logging the user's
        // pre-download retry decision). The library re-downloads the whole
        // collection either way, so no branching is required here.
        include_previously_failed: _,
        skip_already_imported,
        osu_client,
        osu_path,
        prefetched,
    } = request;

    emit_resolving(id, emit.as_ref());

    // Resolve the already-imported library set here, off the UI thread, while the
    // tab shows its preparing state. Best-effort: any read error or panic leaves
    // the set empty (download proceeds unfiltered), never aborts the run.
    let owned_ids = resolve_owned_ids(skip_already_imported, osu_client, osu_path).await;

    let Some(session) = DownloadSession::prepare(PrepareParams {
        id,
        cancel_rx: cancel_rx.clone(),
        config: &config,
        registry: &DOWNLOAD_REGISTRY,
        emit: emit.as_ref(),
        target: PrepareTarget::Collection {
            collection_input: &collection_input,
            prefetched,
        },
        overwrite: auto_overwrite,
        owned_ids,
    })
    .await?
    else {
        return Ok(());
    };

    if session.skipped_owned > 0 {
        emit(DownloadEvent::SkippedImported {
            id,
            count: session.skipped_owned as usize,
        });
    }

    let collection = session.target.collection().cloned();
    let output_dir = session.output.output_dir.clone();

    let Some(tally) = run_pipeline_core(
        id,
        &session,
        &config,
        auto_overwrite,
        cancel_rx,
        defer_rx,
        skip_rx,
        &emit,
    )
    .await?
    else {
        drop(session);
        try_remove_empty_output_dir(&output_dir).await;
        return Ok(());
    };

    // collection.db reflects the full collection regardless of partial failures so that
    // saved state matches the user's intent even when some maps couldn't be downloaded.
    // Always `Some` here (a collection run), but the accessor is `Option` since a
    // search run has no collection.
    if let Some(collection) = collection {
        let db_collection_name = format!("{}-{}", collection.name, collection.id);
        write_collection_db(collection, db_collection_name, output_dir).await?;
    }

    // Clear any now-on-disk ids from the persisted failed-maps file (so a
    // successful re-download stops showing as previously failed) and record this
    // run's fresh failures — both in one pass.
    let resolved: HashSet<u32> = session
        .initial_satisfied
        .iter()
        .copied()
        .chain(tally.successful.iter().copied())
        .collect();
    reconcile_failed_maps(resolved, failure_ids(&tally)).await;

    emit_finish(id, emit.as_ref(), tally.to_summary());
    Ok(())
}

/// Resolve the user's already-imported library set off the UI thread. Best-effort:
/// disabled, a read error, or a join panic all yield an empty set (the download
/// then proceeds without pre-skipping) and never abort the run.
async fn resolve_owned_ids(
    skip_already_imported: bool,
    osu_client: OsuClient,
    osu_path: String,
) -> HashSet<u32> {
    if !skip_already_imported {
        return HashSet::new();
    }
    let install_dir = PathBuf::from(osu_path);
    match tokio::task::spawn_blocking(move || {
        library_cache::owned_ids_cached(osu_client, install_dir)
    })
    .await
    {
        Ok(Ok(owned)) => owned,
        Ok(Err(err)) => {
            warn!(error = %err, "skip-already-imported: library read failed; not pre-skipping");
            HashSet::new()
        }
        Err(err) => {
            warn!(error = %err, "skip-already-imported: library task panicked; not pre-skipping");
            HashSet::new()
        }
    }
}

async fn run_selective(
    id: DownloadId,
    request: SelectiveDownloadRequest,
    cancel_rx: watch::Receiver<bool>,
    defer_rx: watch::Receiver<u64>,
    skip_rx: watch::Receiver<u64>,
    emit: EmitArc,
) -> Result<(), DownloadError> {
    let SelectiveDownloadRequest {
        collection_ids,
        beatmapset_ids,
        collections,
        config,
        snapshot_dir,
        snapshots: snapshot_files,
        prefetched,
    } = request;

    if beatmapset_ids.is_empty() {
        return Err(DownloadError::NoBeatmapsets);
    }

    emit_resolving(id, emit.as_ref());

    let Some(session) = DownloadSession::prepare(PrepareParams {
        id,
        cancel_rx: cancel_rx.clone(),
        config: &config,
        registry: &DOWNLOAD_REGISTRY,
        emit: emit.as_ref(),
        target: PrepareTarget::Selective {
            collection_ids: &collection_ids,
            collections,
            beatmapset_ids: &beatmapset_ids,
            prefetched,
        },
        overwrite: false,
        // The selective/retry path never pre-skips owned maps — it must not
        // perturb the `all_targets_satisfied` snapshot gating.
        owned_ids: HashSet::new(),
    })
    .await?
    else {
        return Ok(());
    };

    // Always `Some` here (a selective run); the accessor is `Option` for search.
    let collection = session.target.collection().cloned();
    let selective_collections = session
        .target
        .selective_collections()
        .map(<[_]>::to_vec)
        .unwrap_or_default();
    let output_dir = session.output.output_dir.clone();
    let initial_satisfied = session.initial_satisfied.clone();
    let target_ids = session.beatmapset_ids.clone();

    let Some(tally) = run_pipeline_core(
        id, &session, &config, false, cancel_rx, defer_rx, skip_rx, &emit,
    )
    .await?
    else {
        drop(session);
        try_remove_empty_output_dir(&output_dir).await;
        return Ok(());
    };

    // every target that is verifiably on disk now: pre-existing + newly downloaded.
    let verified_now: HashSet<u32> = initial_satisfied
        .iter()
        .copied()
        .chain(tally.successful.iter().copied())
        .collect();

    if !verified_now.is_empty()
        && let Some(collection) = collection
    {
        write_selective_collection_db(
            collection,
            selective_collections,
            verified_now.clone(),
            output_dir.clone(),
        )
        .await?;
    }

    let all_targets_satisfied = target_ids.iter().all(|id| verified_now.contains(id));
    if all_targets_satisfied && let Some(snapshot_dir) = snapshot_dir {
        persist_snapshots(snapshot_dir, snapshot_files).await?;
    }

    // A retry / selective run is exactly where stale previously-failed entries get
    // cleared: drop every id now on disk and record this run's fresh failures.
    reconcile_failed_maps(verified_now, failure_ids(&tally)).await;

    emit_finish(id, emit.as_ref(), tally.to_summary());
    Ok(())
}

async fn run_ids(
    id: DownloadId,
    request: IdsDownloadRequest,
    cancel_rx: watch::Receiver<bool>,
    defer_rx: watch::Receiver<u64>,
    skip_rx: watch::Receiver<u64>,
    emit: EmitArc,
) -> Result<(), DownloadError> {
    let IdsDownloadRequest {
        beatmapset_ids,
        label,
        folder_tag,
        source,
        config,
        auto_overwrite,
        skip_already_imported,
        osu_client,
        osu_path,
        known_sizes,
    } = request;

    if beatmapset_ids.is_empty() {
        return Err(DownloadError::NoBeatmapsets);
    }

    emit_resolving(id, emit.as_ref());

    // Same off-thread owned-library resolve as `run_collection`; best-effort.
    let owned_ids = resolve_owned_ids(skip_already_imported, osu_client, osu_path).await;

    let Some(session) = DownloadSession::prepare(PrepareParams {
        id,
        cancel_rx: cancel_rx.clone(),
        config: &config,
        registry: &DOWNLOAD_REGISTRY,
        emit: emit.as_ref(),
        target: PrepareTarget::Ids {
            beatmapset_ids: &beatmapset_ids,
            label: &label,
            folder_tag: &folder_tag,
            source,
            known_sizes,
        },
        overwrite: auto_overwrite,
        owned_ids,
    })
    .await?
    else {
        return Ok(());
    };

    if session.skipped_owned > 0 {
        emit(DownloadEvent::SkippedImported {
            id,
            count: session.skipped_owned as usize,
        });
    }

    let output_dir = session.output.output_dir.clone();

    let Some(tally) = run_pipeline_core(
        id,
        &session,
        &config,
        auto_overwrite,
        cancel_rx,
        defer_rx,
        skip_rx,
        &emit,
    )
    .await?
    else {
        drop(session);
        try_remove_empty_output_dir(&output_dir).await;
        return Ok(());
    };

    // A raw-ids run has no collection identity, so it writes no `collection.db`.
    // The persisted failed-maps file is still reconciled: a re-download clears
    // stale failures and this run's fresh failures are recorded.
    let resolved: HashSet<u32> = session
        .initial_satisfied
        .iter()
        .copied()
        .chain(tally.successful.iter().copied())
        .collect();
    reconcile_failed_maps(resolved, failure_ids(&tally)).await;

    emit_finish(id, emit.as_ref(), tally.to_summary());
    Ok(())
}

/// Beatmapset ids that failed this run, for persisting to the failed-maps file.
fn failure_ids(tally: &Tally) -> Vec<u32> {
    tally.failures.iter().map(|f| f.beatmapset_id).collect()
}

/// Reconcile the persisted failed-maps file with one run's outcome off-thread:
/// remove `resolved` (now on disk) and add `failures`. A missing data path or an
/// IO error is non-fatal — persistence is best-effort, never blocks the run.
async fn reconcile_failed_maps(resolved: HashSet<u32>, failures: Vec<u32>) {
    if resolved.is_empty() && failures.is_empty() {
        return;
    }
    let Some(path) = failed_maps::failed_maps_path() else {
        warn!("failed maps path unavailable; skipping reconcile");
        return;
    };
    let _ = tokio::task::spawn_blocking(move || {
        failed_maps::reconcile(&path, &resolved, failures);
    })
    .await;
}

async fn persist_snapshots(
    snapshot_dir: PathBuf,
    snapshot_files: Vec<snapshots::CollectionSnapshotFile>,
) -> Result<(), DownloadError> {
    tokio::task::spawn_blocking(move || {
        for snapshot in snapshot_files {
            let Ok(collection_id) = snapshot.collection_id.parse() else {
                continue;
            };
            snapshots::save(
                &snapshot,
                &snapshots::snapshot_path(&snapshot_dir, collection_id),
            );
        }
    })
    .await
    .map_err(|err| DownloadError::internal(format!("snapshot save task panicked: {err}")))
}

/// Drives the [`Downloader`] for the prepared session. Returns `None` if cancelled.
#[allow(clippy::too_many_arguments)]
async fn run_pipeline_core(
    id: DownloadId,
    session: &DownloadSession,
    config: &DownloadConfig,
    auto_overwrite: bool,
    cancel_rx: watch::Receiver<bool>,
    defer_rx: watch::Receiver<u64>,
    skip_rx: watch::Receiver<u64>,
    emit: &EmitArc,
) -> Result<Option<Tally>, DownloadError> {
    if config.mirrors.is_empty() {
        return Err(DownloadError::NoMirrors);
    }

    warn_low_disk_space(id, &session.output.output_dir, emit.as_ref());

    let mut tally = Tally {
        // Pre-existing on-disk maps plus library-owned maps both count as skipped.
        skipped: session.skipped_existing + session.skipped_owned,
        unverified: session.initial_unverified.len() as u32,
        ..Tally::default()
    };
    super::events::emit_overall_progress(id, &tally, emit.as_ref());

    if session.pending_ids.is_empty() {
        return Ok(Some(tally));
    }

    // Spawned rather than awaited: even a capped sample of still-unknown ids
    // (0 for a fully-cached find run) shouldn't stall the download start for a
    // cosmetic estimate. Raced against cancel so a stopped run drops the probe
    // instead of hammering nekoha; a late CollectionSizeResolved on a finished
    // run is a no-op (`App::page_mut` returns `None`).
    let size_emit = Arc::clone(emit);
    let size_ids = session.beatmapset_ids.clone();
    let known_sizes = session.known_sizes.clone();
    let mut size_cancel = cancel_rx.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = fetch_collection_sizes(id, &size_ids, &known_sizes, size_emit.as_ref()) => {}
            _ = size_cancel.wait_for(|c| *c) => {}
        }
    });

    let on_exists = if auto_overwrite {
        OnExists::Overwrite
    } else {
        OnExists::Skip
    };

    // Attach the osu! API bearer token to any auth-requiring mirror. Auth-gated
    // mirrors are dropped when there is no valid login, which can empty the list.
    let mirrors = inject_mirror_auth(&config.mirrors).await;
    if mirrors.is_empty() {
        return Err(DownloadError::NoMirrors);
    }

    let downloader = Downloader::builder()
        .mirrors(mirrors.iter().cloned())
        .concurrent_downloads(config.concurrent.max(1) as usize)
        .archive_validation(config.archive_validation)
        .progress_timeout(Duration::from_secs(DEFAULT_PROGRESS_WATCHDOG_SECS))
        .network_retry_attempts(NETWORK_RETRY_CAP as usize)
        .on_exists(on_exists)
        .rate_limit_skip_after(
            config
                .auto_skip_rate_limited
                .then(|| Duration::from_secs(config.rate_limit_skip_secs.max(1) as u64)),
        )
        .build()
        .map_err(|err| DownloadError::internal(err.to_string()))?;

    let mut session_handle = downloader.download_many(
        session.pending_ids.iter().copied(),
        &session.output.output_dir,
    );
    let mut events = session_handle
        .events()
        .expect("events() called once per session");
    let mut cancel_signal = cancel_rx;
    let mut defer_signal = defer_rx;
    let mut skip_signal = skip_rx;

    let cancelled = drive_session(
        &mut session_handle,
        &mut events,
        &mut cancel_signal,
        &mut defer_signal,
        &mut skip_signal,
        |lib_event| translate_event(id, lib_event, &mut tally, emit.as_ref()),
    )
    .await;

    let _ = session_handle.wait().await;

    if cancelled {
        emit.as_ref()(DownloadEvent::Failed {
            id,
            message: ABORTED_FAIL.into(),
        });
        return Ok(None);
    }

    if !tally.failures.is_empty() {
        emit.as_ref()(DownloadEvent::FailedMaps {
            id,
            failures: tally.failures.clone(),
        });
        warn!(
            count = tally.failures.len(),
            "download completed with failures"
        );
    }

    Ok(Some(tally))
}

async fn drive_session<F, S>(
    session_handle: &mut LibDownloadSession,
    events: &mut S,
    cancel_signal: &mut watch::Receiver<bool>,
    defer_signal: &mut watch::Receiver<u64>,
    skip_signal: &mut watch::Receiver<u64>,
    mut on_event: F,
) -> bool
where
    F: FnMut(LibEvent),
    S: futures_util::Stream<Item = LibEvent> + Unpin,
{
    loop {
        tokio::select! {
            biased;
            changed = cancel_signal.changed() => {
                if changed.is_err() { return false; }
                if *cancel_signal.borrow() {
                    session_handle.cancel();
                    return true;
                }
            }
            // Each bump of the defer counter forwards to the library, which
            // requeues every map currently parked on a rate-limit cooldown.
            changed = defer_signal.changed() => {
                if changed.is_ok() {
                    session_handle.defer_rate_limited();
                }
            }
            // Each bump of the skip counter forwards to the library, which drops
            // every map currently parked on a rate-limit cooldown.
            changed = skip_signal.changed() => {
                if changed.is_ok() {
                    session_handle.skip_rate_limited();
                }
            }
            event = events.next() => match event {
                Some(lib_event) => on_event(lib_event),
                None => return false,
            },
        }
    }
}

/// Attach the osu! API bearer token to any auth-requiring mirror.
///
/// [`osu_downloader::MirrorKind::OsuApi`] downloads need an `Authorization:
/// Bearer` header carrying a `*` (lazer-tier) user token plus an
/// `x-api-version` header. We resolve (and refresh) the stored login here, at
/// download time, and attach both. Auth-requiring mirrors are dropped when no
/// valid token is available, so the rest of the run proceeds on the anonymous
/// mirrors.
async fn inject_mirror_auth(mirrors: &[Mirror]) -> Vec<Mirror> {
    if !mirrors.iter().any(|mirror| mirror.kind().requires_auth()) {
        return mirrors.to_vec();
    }

    // Build the bearer header once and reuse it for every auth-gated mirror.
    let auth_headers = build_osu_auth_headers().await;
    if auth_headers.is_none() {
        warn!("osu! official mirror selected but no valid login; skipping it");
    }

    mirrors
        .iter()
        .filter_map(|mirror| {
            if !mirror.kind().requires_auth() {
                return Some(mirror.clone());
            }
            let headers = auth_headers.clone()?;
            Some(mirror.clone().with_headers(headers))
        })
        .collect()
}

/// Resolve the osu! token and wrap it in the `Authorization: Bearer` +
/// `x-api-version` header map every api v2 download needs. Returns `None` (with
/// a logged reason) when there is no valid login or the token can't form a
/// header value.
async fn build_osu_auth_headers() -> Option<HeaderMap> {
    let token = resolve_osu_bearer().await?;
    let bearer = match HeaderValue::from_str(&format!("Bearer {token}")) {
        Ok(value) => value,
        Err(err) => {
            warn!(error = %err, "osu! token is not a valid header value; skipping the official mirror");
            return None;
        }
    };
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, bearer);
    // The api v2 download endpoint rejects requests without `x-api-version`.
    headers.insert(
        HeaderName::from_static("x-api-version"),
        HeaderValue::from_static(crate::auth::X_API_VERSION),
    );
    Some(headers)
}

/// Load, scope-check, and refresh the stored osu! token for the official mirror.
/// Returns `None` (with a logged reason) when the user is not logged in, the
/// token lacks the `*` (lazer-tier) scope, or a refresh fails.
async fn resolve_osu_bearer() -> Option<String> {
    let mut auth = crate::auth::load()?;
    if !auth.has_lazer_scope() {
        warn!("stored osu! token lacks the '*' scope; re-login to enable the official mirror");
        return None;
    }
    let client = reqwest::Client::new();
    if let Err(err) = crate::auth::ensure_valid(&client, &mut auth).await {
        warn!(error = %err, "failed to refresh osu! token for the official mirror");
        return None;
    }
    Some(auth.bearer_token().to_string())
}

pub async fn try_remove_empty_output_dir(output_dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(output_dir).await else {
        return;
    };
    if entries.next_entry().await.ok().flatten().is_some() {
        return;
    }
    if let Err(err) = tokio::fs::remove_dir(output_dir).await {
        warn!(error = %err, path = %output_dir.display(), "failed to remove empty output directory");
    }
}

#[cfg(test)]
#[path = "../../tests/unit/download_pipeline.rs"]
mod tests;
