use super::{
    CleanupTracker, DownloadEvent, DownloadHandle, DownloadId, DownloadRequest, DownloadStage,
    DownloadSummary, OutstandingTracker, VerifiedRegistry, create_download_client,
    integrity::ExpectationIndex,
    passes::{DownloadPassArgs, download_pass},
    precheck::{PrecheckReport, verify_existing_beatmapsets},
};
use crate::{
    core::collection::{
        CollectionService, HttpCollectionService, create_collection_db,
        generate_collection_folder_name, model::Collection,
    },
    utils::{self, AppError, validate_and_prepare_directory},
    worker::MirrorPool,
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{fs, sync::mpsc::UnboundedSender};
use tracing::Instrument;
use tracing::{debug, error, info, info_span, warn};

pub fn spawn_download(
    id: DownloadId,
    request: DownloadRequest,
    tx: UnboundedSender<DownloadEvent>,
) -> DownloadHandle {
    let tx_clone = tx.clone();
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_worker = shutdown.clone();

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

    let join_handle = tokio::spawn(
        async move {
            info!("Download task started");
            match run_download(id, request, shutdown_worker, &tx).await {
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

struct CollectionResolution {
    collection: Collection,
    beatmap_ids: Vec<u32>,
    expectation_index: Arc<ExpectationIndex>,
}

struct OutputPreparation {
    output_dir: PathBuf,
    display: String,
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
    } = precheck;

    let tracker = OutstandingTracker::new(resolution.beatmap_ids.iter().copied().collect());
    let remaining_after_precheck = tracker.remove_all(pre_verified.iter().copied()).await;

    let verified_registry = VerifiedRegistry::new(pre_verified);

    let _ = tx.send(DownloadEvent::DownloadTarget {
        id,
        remaining: remaining_after_precheck,
    });

    let pre_unverified_count = pre_unverified.len().min(u16::MAX as usize) as u16;
    let mut totals = DownloadSummary {
        downloaded: 0,
        skipped: initial_skipped,
        failed: 0,
        unverified: pre_unverified_count,
        unverified_sets: pre_unverified.clone(),
    };
    let mut failure_tracker = FailureTracker::new();
    let mut aborted = false;

    if totals.skipped > 0 {
        let _ = tx.send(DownloadEvent::Log {
            id,
            message: format!("{} beatmapsets already verified locally", totals.skipped),
        });
        let _ = tx.send(DownloadEvent::OverallProgress {
            id,
            downloaded: totals.downloaded,
            skipped: totals.skipped,
            failed: totals.failed,
            unverified: totals.unverified,
        });
    }

    let download_client = create_download_client().map_err(|e| format!("{}", e))?;
    if mirrors.is_empty() {
        warn!("Download request did not include any mirrors");
        return Err("Select at least one mirror".to_string());
    }
    let mirror_pool = MirrorPool::new(mirrors);
    let output_dir_arc = Arc::new(output.output_dir.clone());
    let cleanup_tracker = CleanupTracker::new();

    for (pass_number, is_retry) in [false, true].into_iter().enumerate() {
        if tracker.is_empty().await || aborted || shutdown.load(Ordering::SeqCst) {
            break;
        }

        if is_retry {
            let _ = tx.send(DownloadEvent::StageChanged {
                id,
                stage: DownloadStage::Rechecking,
            });
            let remaining_targets = tracker.len().await;
            let _ = tx.send(DownloadEvent::Log {
                id,
                message: format!(
                    "Starting retry pass {} ({} targets remaining)",
                    pass_number, remaining_targets
                ),
            });
            info!(
                attempt = pass_number,
                remaining = remaining_targets,
                "Starting retry pass"
            );
        } else {
            let remaining_targets = tracker.len().await;
            info!(
                remaining = remaining_targets,
                "Starting primary download pass"
            );
        }

        let targets = tracker.snapshot().await;
        if targets.is_empty() {
            break;
        }

        let pass_result = {
            let args = DownloadPassArgs {
                id,
                beatmapset_ids: targets,
                thread_count,
                skip_existing,
                auto_overwrite,
                shutdown: shutdown.clone(),
                client: download_client.clone(),
                mirror_pool: mirror_pool.clone(),
                output_dir: output_dir_arc.clone(),
                expectations: resolution.expectation_index.clone(),
                verified: verified_registry.clone(),
                outstanding: tracker.clone(),
                cleanup_tracker: cleanup_tracker.clone(),
                retry_phase: is_retry,
                tx: tx.clone(),
            };
            download_pass(args, &mut totals).await
        };

        failure_tracker.record(pass_result.failed_maps);
        aborted = pass_result.aborted;

        if is_retry {
            let _ = tx.send(DownloadEvent::StageChanged {
                id,
                stage: DownloadStage::Downloading,
            });
        }

        if aborted {
            warn!("Download aborted during pass");
            break;
        }

        if is_retry && !tracker.is_empty().await && !shutdown.load(Ordering::SeqCst) {
            let remaining = tracker.len().await;
            warn!(
                remaining,
                "Reached maximum retry passes; outstanding beatmapsets remain"
            );
            let _ = tx.send(DownloadEvent::Log {
                id,
                message: format!(
                    "Maximum retry passes reached with {} outstanding beatmapsets",
                    remaining
                ),
            });
        }
    }

    if shutdown.load(Ordering::SeqCst) {
        emit_failed_maps(tx, id, &failure_tracker);
        let cleanup_outcome = cleanup_tracker.cleanup_incomplete().await;
        if cleanup_outcome.removed > 0 {
            info!(
                removed = cleanup_outcome.removed,
                "Removed incomplete beatmap archives"
            );
            let _ = tx.send(DownloadEvent::Log {
                id,
                message: format!(
                    "Cleaned up {} incomplete beatmap archives",
                    cleanup_outcome.removed
                ),
            });
        }
        for (path, message) in cleanup_outcome.failures {
            warn!(target = %path.display(), error = %message, "Failed to cleanup file");
            let _ = tx.send(DownloadEvent::Log {
                id,
                message: format!("Cleanup warning for {}: {}", path.display(), message),
            });
        }
        let _ = tx.send(DownloadEvent::Log {
            id,
            message: "Download aborted before completion".to_string(),
        });
        let _ = tx.send(DownloadEvent::Failed {
            id,
            message: "Download aborted by user".to_string(),
        });
        warn!("Download aborted due to shutdown request");
        return Ok(());
    }

    match create_collection_database(&resolution.collection, &output.output_dir) {
        Ok(()) => {
            let _ = tx.send(DownloadEvent::Log {
                id,
                message: "collection.db created successfully".to_string(),
            });
            info!("collection.db created successfully");
        }
        Err(e) => {
            let _ = tx.send(DownloadEvent::Log {
                id,
                message: format!("Warning: Failed to create collection.db: {}", e),
            });
            warn!(error = %e, "Failed to create collection.db");
        }
    }

    if !failure_tracker.is_empty() {
        emit_failed_maps(tx, id, &failure_tracker);
        warn!(
            count = failure_tracker.len(),
            "Download completed with failed beatmapsets"
        );
    }

    let summary = totals.clone();

    let _ = tx.send(DownloadEvent::Finished { id, summary });
    info!("Download pipeline finished and summary dispatched");
    Ok(())
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
    let _ = tx.send(DownloadEvent::Log {
        id,
        message: format!("Downloading to {}", output.display),
    });
}

async fn perform_initial_precheck(
    id: DownloadId,
    output_dir: &Path,
    expectations: Arc<ExpectationIndex>,
    thread_count: usize,
    tx: &UnboundedSender<DownloadEvent>,
) -> Result<PrecheckReport, String> {
    let _ = tx.send(DownloadEvent::Log {
        id,
        message: "Verifying existing beatmapsets on disk".to_string(),
    });
    let _ = tx.send(DownloadEvent::StageChanged {
        id,
        stage: DownloadStage::Rechecking,
    });
    info!("Starting disk precheck before downloads");
    let report =
        verify_existing_beatmapsets(id, output_dir, expectations, thread_count, true, tx).await?;
    info!(
        verified = report.satisfied.len(),
        skipped = report.skipped,
        "Finished initial disk precheck"
    );

    let _ = tx.send(DownloadEvent::StageChanged {
        id,
        stage: DownloadStage::Downloading,
    });

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
    let normalized = {
        let trimmed = directory.trim();
        if trimmed.is_empty() { "." } else { trimmed }
    };

    let base_dir = validate_and_prepare_directory(normalized)
        .await
        .map_err(|e| format!("{}", e))?;
    debug!(base = %base_dir.display(), "Validated base download directory");

    let collection_folder_name = generate_collection_folder_name(collection);
    let output_dir = base_dir.join(&collection_folder_name);
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
