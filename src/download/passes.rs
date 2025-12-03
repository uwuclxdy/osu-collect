use super::{
    BeatmapStage, DownloadEvent, DownloadId, DownloadResult, DownloadSummary, OutstandingTracker,
    VerifiedRegistry, download_beatmap,
    integrity::{ExpectationIndex, verify_download_integrity},
};
use crate::{
    download::CleanupTracker,
    worker::{DownloadContext, MirrorPool},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc::UnboundedSender},
    task::JoinSet,
    time::sleep,
};
use tracing::{debug, info, trace, warn};

struct SlotLimiter {
    semaphore: Arc<Semaphore>,
    slots: Mutex<Vec<usize>>,
}

impl SlotLimiter {
    fn new(count: usize) -> Self {
        let mut slots: Vec<usize> = (0..count).collect();
        slots.reverse();
        Self {
            semaphore: Arc::new(Semaphore::new(count)),
            slots: Mutex::new(slots),
        }
    }

    async fn acquire(self: Arc<Self>) -> SlotLease {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("slot semaphore closed unexpectedly");
        let slot = {
            let mut guard = self.slots.lock().expect("slot pool poisoned");
            guard.pop().expect("no slot available despite permit")
        };
        SlotLease {
            slot,
            limiter: self.clone(),
            permit: Some(permit),
        }
    }

    fn release(&self, slot: usize) {
        let mut guard = self.slots.lock().expect("slot pool poisoned");
        guard.push(slot);
    }
}

struct SlotLease {
    slot: usize,
    limiter: Arc<SlotLimiter>,
    permit: Option<OwnedSemaphorePermit>,
}

impl SlotLease {
    fn slot(&self) -> usize {
        self.slot
    }
}

impl Drop for SlotLease {
    fn drop(&mut self) {
        self.limiter.release(self.slot);
        if let Some(permit) = self.permit.take() {
            drop(permit);
        }
    }
}

pub(crate) struct DownloadPassResult {
    pub(crate) failed_maps: Vec<u32>,
    pub(crate) aborted: bool,
}

pub(crate) struct DownloadPassArgs {
    pub id: DownloadId,
    pub beatmapset_ids: Vec<u32>,
    pub thread_count: usize,
    pub skip_existing: bool,
    pub auto_overwrite: bool,
    pub shutdown: Arc<AtomicBool>,
    pub client: reqwest::Client,
    pub mirror_pool: MirrorPool,
    pub output_dir: Arc<PathBuf>,
    pub expectations: Arc<ExpectationIndex>,
    pub verified: VerifiedRegistry,
    pub outstanding: OutstandingTracker,
    pub cleanup_tracker: CleanupTracker,
    pub retry_phase: bool,
    pub tx: UnboundedSender<DownloadEvent>,
}

pub(crate) async fn download_pass(
    args: DownloadPassArgs,
    totals: &mut DownloadSummary,
) -> DownloadPassResult {
    if args.beatmapset_ids.is_empty() {
        debug!(
            download_id = args.id,
            "download pass invoked with no targets"
        );
        return DownloadPassResult {
            failed_maps: Vec::new(),
            aborted: false,
        };
    }

    info!(
        download_id = args.id,
        queued = args.beatmapset_ids.len(),
        thread_count = args.thread_count,
        retry_phase = args.retry_phase,
        "Starting download pass"
    );
    let mut failed_maps: Vec<u32> = Vec::new();
    let mut beatmap_queue: VecDeque<u32> = args.beatmapset_ids.into_iter().collect();
    let concurrency = args.thread_count.max(1);
    let slot_limiter = Arc::new(SlotLimiter::new(concurrency));
    let mut join_set: JoinSet<_> = JoinSet::new();
    let mut aborted = false;

    while !beatmap_queue.is_empty() || !join_set.is_empty() {
        while join_set.len() < concurrency {
            if args.shutdown.load(Ordering::SeqCst) {
                aborted = true;
                break;
            }

            let Some(beatmapset_id) = beatmap_queue.pop_front() else {
                break;
            };

            let mirror_pool_for_task = args.mirror_pool.clone();
            let output_dir = args.output_dir.clone();
            let shutdown_inner = args.shutdown.clone();
            let download_context = DownloadContext::new(
                args.client.clone(),
                output_dir.clone(),
                args.skip_existing,
                args.auto_overwrite,
                shutdown_inner.clone(),
                Some(args.verified.clone()),
                args.cleanup_tracker.clone(),
            );
            let tx_progress = args.tx.clone();
            let limiter = slot_limiter.clone();
            let retry_phase_for_task = args.retry_phase;

            join_set.spawn(async move {
                let lease = limiter.acquire().await;
                let slot = lease.slot();
                trace!(
                    download_id = args.id,
                    beatmapset_id, slot, "Dispatching beatmap download task"
                );
                let status_sender = tx_progress.clone();
                let start_label = if retry_phase_for_task {
                    "Rechecking"
                } else {
                    "Starting download"
                };
                let _ = status_sender.send(DownloadEvent::BeatmapStatus {
                    id: args.id,
                    beatmapset_id,
                    stage: BeatmapStage::Downloading,
                    message: format!("{} {}", start_label, beatmapset_id),
                });

                let progress_callback = {
                    let tx_inner = tx_progress.clone();
                    Arc::new(move |downloaded: u64, total: u64| {
                        let _ = tx_inner.send(DownloadEvent::BeatmapProgress {
                            id: args.id,
                            beatmapset_id,
                            thread_index: slot,
                            downloaded,
                            total,
                        });
                    })
                };

                let thread_status_sender = tx_progress.clone();
                let status_callback = Arc::new(move |msg: &str| {
                    let message = msg.to_string();
                    let rate_limited = message.starts_with("Rate limited");

                    if !message.starts_with("Contacting") {
                        let _ = thread_status_sender.send(DownloadEvent::ThreadStatus {
                            id: args.id,
                            thread_index: slot,
                            message,
                            rate_limited,
                        });
                    }
                });

                let result = loop {
                    if shutdown_inner.load(Ordering::SeqCst) {
                        warn!(
                            download_id = args.id,
                            beatmapset_id, "Download task aborted due to shutdown signal"
                        );
                        break Ok(DownloadResult::Aborted);
                    }

                    if let Some((mirror_info, wait_for)) =
                        mirror_pool_for_task.single_mirror_cooldown()
                        && !wait_for.is_zero()
                    {
                        let wait_secs = wait_for.as_secs().max(1);
                        let wait_message = format!(
                            "Rate limited on {}, waiting {}s before retry",
                            mirror_info.display_name(),
                            wait_secs
                        );
                        let _ = tx_progress.send(DownloadEvent::ThreadStatus {
                            id: args.id,
                            thread_index: slot,
                            message: wait_message.clone(),
                            rate_limited: true,
                        });
                        sleep(wait_for).await;
                        continue;
                    }

                    let mirror_plan = mirror_pool_for_task.plan();
                    let first_mirror = mirror_plan
                        .first()
                        .map(|mirror| mirror.display_name())
                        .unwrap_or("selected mirror");

                    let activity_label = if retry_phase_for_task {
                        "Rechecking"
                    } else {
                        "Downloading"
                    };
                    let _ = tx_progress.send(DownloadEvent::ThreadStatus {
                        id: args.id,
                        thread_index: slot,
                        message: format!(
                            "{} #{} from {}",
                            activity_label, beatmapset_id, first_mirror
                        ),
                        rate_limited: false,
                    });
                    trace!(
                        download_id = args.id,
                        beatmapset_id,
                        slot,
                        mirror = first_mirror,
                        "Starting mirror download"
                    );

                    let result = download_beatmap(
                        beatmapset_id,
                        mirror_plan.as_slice(),
                        &download_context,
                        Some(progress_callback.clone()),
                        Some(status_callback.clone()),
                        Some(mirror_pool_for_task.clone()),
                    )
                    .await;

                    let should_retry_single_mirror = matches!(
                        result,
                        Ok(DownloadResult::Failed(reason))
                            if reason == "Rate limited"
                                && mirror_pool_for_task.is_single_mirror()
                    );

                    if should_retry_single_mirror {
                        continue;
                    }

                    break result;
                };

                (slot, beatmapset_id, result)
            });
        }

        if aborted {
            break;
        }

        let Some(task_result) = join_set.join_next().await else {
            break;
        };

        let (slot, beatmapset_id, result) = match task_result {
            Ok(values) => values,
            Err(err) => {
                aborted = true;
                warn!(download_id = args.id, error = %err, "Download task panicked");
                break;
            }
        };

        match result {
            Ok(DownloadResult::Success(file)) => {
                args.mirror_pool.clear_penalty(file.mirror);
                let file_path = args.output_dir.join(file.filename.as_ref());
                let _ = args.tx.send(DownloadEvent::ThreadStatus {
                    id: args.id,
                    thread_index: slot,
                    message: format!("Verifying integrity for #{}", beatmapset_id),
                    rate_limited: false,
                });

                verify_download_integrity(
                    beatmapset_id,
                    file_path.clone(),
                    args.expectations.clone(),
                )
                .await;
                trace!(
                    download_id = args.id,
                    beatmapset_id, "Integrity verification succeeded"
                );
                totals.downloaded = totals.downloaded.saturating_add(1);
                args.verified.insert(beatmapset_id);
                if let Some(remaining) = args.outstanding.remove(beatmapset_id).await {
                    let _ = args.tx.send(DownloadEvent::DownloadTarget {
                        id: args.id,
                        remaining,
                    });
                }
                let mirror_label = file.mirror.label();
                let success_message = format!(
                    "{} (md5: {}) via {}",
                    file.filename, file.hash, mirror_label
                );
                let _ = args.tx.send(DownloadEvent::BeatmapStatus {
                    id: args.id,
                    beatmapset_id,
                    stage: BeatmapStage::Success,
                    message: success_message,
                });
                let _ = args.tx.send(DownloadEvent::ThreadStatus {
                    id: args.id,
                    thread_index: slot,
                    message: format!("Done via {}", mirror_label),
                    rate_limited: false,
                });
            }
            Ok(DownloadResult::Skipped(filename)) => {
                totals.skipped = totals.skipped.saturating_add(1);
                debug!(
                    download_id = args.id,
                    beatmapset_id, "Skipped beatmap download"
                );
                let _ = args.tx.send(DownloadEvent::BeatmapStatus {
                    id: args.id,
                    beatmapset_id,
                    stage: BeatmapStage::Skipped,
                    message: format!("Skipped: {}", filename),
                });
                if let Some(remaining) = args.outstanding.remove(beatmapset_id).await {
                    let _ = args.tx.send(DownloadEvent::DownloadTarget {
                        id: args.id,
                        remaining,
                    });
                }
            }
            Ok(DownloadResult::Failed(reason)) => {
                totals.failed = totals.failed.saturating_add(1);
                failed_maps.push(beatmapset_id);
                warn!(
                    download_id = args.id,
                    beatmapset_id,
                    error = %reason,
                    "Download failed"
                );
                let _ = args.tx.send(DownloadEvent::BeatmapStatus {
                    id: args.id,
                    beatmapset_id,
                    stage: BeatmapStage::Failed,
                    message: reason.to_string(),
                });
                if let Some(remaining) = args.outstanding.remove(beatmapset_id).await {
                    let _ = args.tx.send(DownloadEvent::DownloadTarget {
                        id: args.id,
                        remaining,
                    });
                }
            }
            Ok(DownloadResult::FailedDynamic(reason)) => {
                totals.failed = totals.failed.saturating_add(1);
                failed_maps.push(beatmapset_id);
                warn!(
                    download_id = args.id,
                    beatmapset_id,
                    error = %reason,
                    "Download failed with dynamic reason"
                );
                let message = reason.to_string();
                let _ = args.tx.send(DownloadEvent::BeatmapStatus {
                    id: args.id,
                    beatmapset_id,
                    stage: BeatmapStage::Failed,
                    message: message.clone(),
                });
                if let Some(remaining) = args.outstanding.remove(beatmapset_id).await {
                    let _ = args.tx.send(DownloadEvent::DownloadTarget {
                        id: args.id,
                        remaining,
                    });
                }
            }
            Ok(DownloadResult::Aborted) => {
                args.shutdown.store(true, Ordering::SeqCst);
                warn!(
                    download_id = args.id,
                    beatmapset_id, "Download aborted mid-pass"
                );
                let _ = args.tx.send(DownloadEvent::BeatmapStatus {
                    id: args.id,
                    beatmapset_id,
                    stage: BeatmapStage::Aborted,
                    message: "Aborted".to_string(),
                });
                aborted = true;
            }
            Err(err) => {
                totals.failed = totals.failed.saturating_add(1);
                failed_maps.push(beatmapset_id);
                let message = format!("{}", err);
                warn!(download_id = args.id, beatmapset_id, error = %message, "Download errored");
                let _ = args.tx.send(DownloadEvent::BeatmapStatus {
                    id: args.id,
                    beatmapset_id,
                    stage: BeatmapStage::Failed,
                    message: message.clone(),
                });
                if let Some(remaining) = args.outstanding.remove(beatmapset_id).await {
                    let _ = args.tx.send(DownloadEvent::DownloadTarget {
                        id: args.id,
                        remaining,
                    });
                }
            }
        }

        if aborted {
            break;
        }

        let _ = args.tx.send(DownloadEvent::OverallProgress {
            id: args.id,
            downloaded: totals.downloaded,
            skipped: totals.skipped,
            failed: totals.failed,
            unverified: totals.unverified,
        });
    }

    if aborted {
        warn!(download_id = args.id, "Download pass aborted; exiting loop");
    }

    DownloadPassResult {
        failed_maps,
        aborted,
    }
}
