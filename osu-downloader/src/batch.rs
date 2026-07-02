//! Batch download orchestration.
//!
//! The library entrypoint ([`Downloader::download_many`](crate::Downloader::download_many))
//! delegates here. Items live in a shared queue; a worker pool of `concurrent_downloads` tasks
//! leases from it. A map whose mirrors are all cooling for longer than the inline-wait threshold
//! is [`Deferred`](crate::download::BeatmapsetDownloadOutcome::Deferred) and pushed back onto the
//! queue tail, so it is retried once its earliest mirror frees instead of parking a worker. Only a
//! rate-limit-sourced defer increments the pass counter; after [`DEFERRAL_PASS_CAP`] such deferrals
//! the map is dropped as [`Skip::RateLimitSkipped`]. A pure request-spacing defer (a healthy map
//! waiting for a send token) requeues at the same pass, so it is never counted toward the cap. A
//! caller hard-drop
//! ([`Session::skip_rate_limited`](crate::Session::skip_rate_limited)) drains every
//! deferred-pending item out of the queue immediately, even while all workers are busy.

use crate::{
    Error, Event, Summary,
    config::{DEFERRAL_PASS_CAP, NETWORK_RETRY_BACKOFF},
    download::{self, BeatmapsetDownloadCallbacks, BeatmapsetDownloadOutcome, download_beatmapset},
    downloader::OnExists,
    event::{Skip, Status},
    mirrors::MirrorPool,
    validation::ArchiveValidation,
};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{Notify, mpsc, watch};
use tracing::{debug, info, warn};

/// Cap on how long an idle worker sleeps before re-checking the queue, so a
/// requeue or completion is never missed even if its wakeup is lost.
#[cfg(not(test))]
const IDLE_POLL_CAP: Duration = Duration::from_millis(250);
#[cfg(test)]
const IDLE_POLL_CAP: Duration = Duration::from_millis(20);

/// Spread added to a requeue-wait so workers waking from the same cooldown do
/// not convoy back onto the recovered mirror in lockstep.
#[cfg(not(test))]
const REQUEUE_JITTER_MAX: Duration = Duration::from_millis(25);
#[cfg(test)]
const REQUEUE_JITTER_MAX: Duration = Duration::from_millis(3);

#[derive(Clone)]
pub(crate) struct BatchConfig {
    pub(crate) concurrent_downloads: usize,
    pub(crate) archive_validation: ArchiveValidation,
    pub(crate) progress_timeout: Duration,
    pub(crate) network_retry_attempts: usize,
    pub(crate) sanitize_filenames: bool,
    pub(crate) on_exists: OnExists,
    pub(crate) rate_limit_skip_after: Option<Duration>,
    pub(crate) inline_wait_max: Duration,
    #[cfg(feature = "instrument")]
    pub(crate) attempt_observer: Option<crate::instrument::AttemptObserver>,
    #[cfg(feature = "instrument")]
    pub(crate) session_start: Instant,
}

struct QueueItem {
    id: u32,
    /// Deferrals so far; 0 for a fresh item.
    pass: u32,
    /// Earliest instant this item may be leased again.
    ready_at: Instant,
}

enum Lease {
    /// Process this item now.
    Work(QueueItem),
    /// Nothing is ready; wait until this instant, or indefinitely if `None`
    /// (queue empty but other workers still in flight).
    Idle(Option<Instant>),
    /// All items reached a terminal outcome.
    Done,
}

/// Shared work queue for the worker pool: FIFO among ready items, with deferred
/// items carrying a future `ready_at`.
struct BatchQueue {
    inner: Mutex<QueueInner>,
    ready: Notify,
}

struct QueueInner {
    items: VecDeque<QueueItem>,
    /// Items not yet terminal (queued plus in flight).
    outstanding: usize,
}

impl BatchQueue {
    fn new(ids: &[u32]) -> Self {
        let now = Instant::now();
        let items = ids
            .iter()
            .map(|&id| QueueItem {
                id,
                pass: 0,
                ready_at: now,
            })
            .collect();
        Self {
            inner: Mutex::new(QueueInner {
                items,
                outstanding: ids.len(),
            }),
            ready: Notify::new(),
        }
    }

    fn lease(&self, now: Instant) -> Lease {
        let mut inner = self.inner.lock().unwrap();
        if inner.outstanding == 0 {
            return Lease::Done;
        }
        let mut earliest: Option<Instant> = None;
        let mut ready_pos: Option<usize> = None;
        for (i, item) in inner.items.iter().enumerate() {
            if item.ready_at <= now {
                ready_pos = Some(i);
                break;
            }
            earliest = Some(earliest.map_or(item.ready_at, |e| e.min(item.ready_at)));
        }
        match ready_pos {
            Some(i) => Lease::Work(inner.items.remove(i).expect("index from enumerate")),
            None => Lease::Idle(earliest),
        }
    }

    fn requeue(&self, id: u32, pass: u32, ready_at: Instant) {
        let mut inner = self.inner.lock().unwrap();
        inner.items.push_back(QueueItem { id, pass, ready_at });
        drop(inner);
        self.ready.notify_waiters();
    }

    fn complete(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.outstanding -= 1;
        drop(inner);
        self.ready.notify_waiters();
    }

    /// Remove every deferred-pending item (pass >= 1) from the queue, returning
    /// their ids. Fresh pass-0 items that merely have not started stay. Removal
    /// under the lock guarantees exactly-once terminal handling: a drained item
    /// can no longer be leased, and a leased item is not in the queue to drain.
    /// Decrements `outstanding` and wakes idle workers so they observe `Done`.
    fn drain_deferred(&self) -> Vec<u32> {
        let mut inner = self.inner.lock().unwrap();
        let mut dropped = Vec::new();
        inner.items.retain(|item| {
            if item.pass >= 1 {
                dropped.push(item.id);
                false
            } else {
                true
            }
        });
        inner.outstanding -= dropped.len();
        drop(inner);
        if !dropped.is_empty() {
            self.ready.notify_waiters();
        }
        dropped
    }
}

fn requeue_jitter() -> Duration {
    let max = REQUEUE_JITTER_MAX.as_millis() as u64;
    if max == 0 {
        return Duration::ZERO;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    Duration::from_millis(nanos % max)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn download_batch(
    ids: Vec<u32>,
    output_dir: &Path,
    client: reqwest::Client,
    mirror_pool: Arc<MirrorPool>,
    config: BatchConfig,
    event_tx: mpsc::UnboundedSender<Event>,
    cancel_rx: watch::Receiver<bool>,
    defer: Arc<Notify>,
    drop_signal: Arc<Notify>,
) -> Summary {
    let start_time = Instant::now();
    let total = ids.len();
    let mut summary = Summary::new(total);
    let _ = event_tx.send(Event::SessionStarted { total });

    if ids.is_empty() {
        finalize(summary, &event_tx, start_time);
        return Summary::new(0);
    }

    let queue = Arc::new(BatchQueue::new(&ids));
    let worker_count = config.concurrent_downloads.max(1);
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<DownloadOutcome>();
    let mut worker_handles = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let queue = queue.clone();
        let result_tx = result_tx.clone();
        let client = client.clone();
        let mirror_pool = mirror_pool.clone();
        let config = config.clone();
        let event_tx = event_tx.clone();
        let cancel_rx = cancel_rx.clone();
        let defer = defer.clone();
        let drop_signal = drop_signal.clone();
        let output_dir = output_dir.to_path_buf();
        worker_handles.push(tokio::spawn(async move {
            worker_loop(
                queue,
                result_tx,
                client,
                mirror_pool,
                config,
                event_tx,
                cancel_rx,
                defer,
                drop_signal,
                output_dir,
            )
            .await;
        }));
    }
    drop(result_tx);

    // The drop signal must clear deferred-pending queue items at the instant of
    // the press, even while every worker is busy streaming, so this task (never
    // a worker) races it against result collection. The notified future is
    // pre-registered via `enable` and re-armed after each drain so a press
    // landing between drains is never lost.
    let mut dropped = std::pin::pin!(drop_signal.notified());
    dropped.as_mut().enable();
    loop {
        tokio::select! {
            outcome = result_rx.recv() => {
                let Some(outcome) = outcome else { break };
                match outcome {
                    DownloadOutcome::Success {
                        beatmapset_id,
                        size_bytes,
                    } => {
                        summary.downloaded.push(beatmapset_id);
                        summary.total_bytes += size_bytes;
                    }
                    DownloadOutcome::Skipped {
                        beatmapset_id,
                        reason,
                    } => summary.skipped.push((beatmapset_id, reason)),
                    DownloadOutcome::Failed {
                        beatmapset_id,
                        error,
                    } => summary.failed.push((beatmapset_id, error)),
                    DownloadOutcome::Aborted => {}
                }
            }
            _ = dropped.as_mut() => {
                for beatmapset_id in queue.drain_deferred() {
                    let _ = event_tx.send(Event::BeatmapsetSkipped {
                        beatmapset_id,
                        reason: Skip::RateLimitSkipped,
                    });
                    summary.skipped.push((beatmapset_id, Skip::RateLimitSkipped));
                }
                dropped.set(drop_signal.notified());
                dropped.as_mut().enable();
            }
        }
    }

    for handle in worker_handles {
        let _ = handle.await;
    }

    finalize(summary.clone(), &event_tx, start_time);

    summary.duration = start_time.elapsed();
    info!(
        downloaded = summary.downloaded.len(),
        skipped = summary.skipped.len(),
        failed = summary.failed.len(),
        total,
        "batch complete"
    );
    summary
}

fn finalize(mut summary: Summary, event_tx: &mpsc::UnboundedSender<Event>, start_time: Instant) {
    summary.duration = start_time.elapsed();
    let _ = event_tx.send(Event::SessionCompleted { summary });
}

enum DownloadOutcome {
    Success { beatmapset_id: u32, size_bytes: u64 },
    Skipped { beatmapset_id: u32, reason: Skip },
    Failed { beatmapset_id: u32, error: Error },
    Aborted,
}

#[allow(clippy::too_many_arguments)]
async fn worker_loop(
    queue: Arc<BatchQueue>,
    result_tx: mpsc::UnboundedSender<DownloadOutcome>,
    client: reqwest::Client,
    mirror_pool: Arc<MirrorPool>,
    config: BatchConfig,
    event_tx: mpsc::UnboundedSender<Event>,
    cancel_rx: watch::Receiver<bool>,
    defer: Arc<Notify>,
    drop_signal: Arc<Notify>,
    output_dir: std::path::PathBuf,
) {
    loop {
        if *cancel_rx.borrow() {
            break;
        }
        match queue.lease(Instant::now()) {
            Lease::Done => break,
            Lease::Idle(when) => {
                let notified = queue.ready.notified();
                let sleep_for = match when {
                    Some(t) => t
                        .saturating_duration_since(Instant::now())
                        .min(IDLE_POLL_CAP),
                    None => IDLE_POLL_CAP,
                };
                let mut cancel = cancel_rx.clone();
                tokio::select! {
                    biased;
                    _ = wait_until_cancelled(&mut cancel) => break,
                    _ = notified => {}
                    _ = tokio::time::sleep(sleep_for) => {}
                }
            }
            Lease::Work(item) => {
                let outcome = process_one(
                    item.id,
                    &output_dir,
                    &client,
                    &mirror_pool,
                    &config,
                    event_tx.clone(),
                    cancel_rx.clone(),
                    defer.clone(),
                    drop_signal.clone(),
                )
                .await;

                if let BeatmapsetDownloadOutcome::Deferred {
                    retry_in,
                    rate_limited,
                } = outcome
                {
                    // Only a rate-limit-sourced defer advances the pass counter and
                    // can hit the drop cap; a pure request-spacing defer requeues at
                    // the same pass, so a healthy map (never 429'd, pass 0) is never
                    // counted toward the cap nor drained as skipped. The future
                    // `ready_at` still bounds the requeue, so it cannot hot-loop.
                    let pass = if rate_limited {
                        item.pass + 1
                    } else {
                        item.pass
                    };
                    if rate_limited && pass >= DEFERRAL_PASS_CAP {
                        let _ = event_tx.send(Event::BeatmapsetSkipped {
                            beatmapset_id: item.id,
                            reason: Skip::RateLimitSkipped,
                        });
                        let _ = result_tx.send(DownloadOutcome::Skipped {
                            beatmapset_id: item.id,
                            reason: Skip::RateLimitSkipped,
                        });
                        queue.complete();
                    } else {
                        let _ = event_tx.send(Event::BeatmapsetDeferred {
                            beatmapset_id: item.id,
                            pass,
                            retry_in,
                        });
                        queue.requeue(item.id, pass, Instant::now() + retry_in + requeue_jitter());
                    }
                    continue;
                }

                let result = emit_terminal(item.id, outcome, &event_tx);
                queue.complete();
                if result_tx.send(result).is_err() {
                    break;
                }
            }
        }
    }
}

/// Emit the terminal event for a completed map and map it to a [`DownloadOutcome`].
fn emit_terminal(
    beatmapset_id: u32,
    outcome: BeatmapsetDownloadOutcome,
    event_tx: &mpsc::UnboundedSender<Event>,
) -> DownloadOutcome {
    match outcome {
        BeatmapsetDownloadOutcome::Success {
            filename,
            hash,
            mirror,
            size_bytes,
            verify_duration_us,
        } => {
            let _ = event_tx.send(Event::BeatmapsetCompleted {
                beatmapset_id,
                filename,
                size_bytes,
                md5_hash: Some(hash),
                mirror_used: mirror,
                verify_duration_us,
            });
            DownloadOutcome::Success {
                beatmapset_id,
                size_bytes,
            }
        }
        BeatmapsetDownloadOutcome::Skipped { reason } => {
            let _ = event_tx.send(Event::BeatmapsetSkipped {
                beatmapset_id,
                reason: reason.clone(),
            });
            DownloadOutcome::Skipped {
                beatmapset_id,
                reason,
            }
        }
        BeatmapsetDownloadOutcome::Failed { mirror, reason } => {
            let error = Error::validation(reason);
            let _ = event_tx.send(Event::BeatmapsetFailed {
                beatmapset_id,
                error: error.clone(),
                mirror,
            });
            DownloadOutcome::Failed {
                beatmapset_id,
                error,
            }
        }
        BeatmapsetDownloadOutcome::NetworkError { reason } => {
            let error = Error::network(reason);
            let _ = event_tx.send(Event::BeatmapsetFailed {
                beatmapset_id,
                error: error.clone(),
                mirror: None,
            });
            DownloadOutcome::Failed {
                beatmapset_id,
                error,
            }
        }
        BeatmapsetDownloadOutcome::Aborted => {
            warn!(beatmapset_id, "download aborted");
            DownloadOutcome::Aborted
        }
        BeatmapsetDownloadOutcome::Deferred { .. } => {
            // Handled by the caller before reaching here.
            unreachable!("deferred outcome is requeued in worker_loop, not finalized")
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_one(
    beatmapset_id: u32,
    output_dir: &Path,
    client: &reqwest::Client,
    mirror_pool: &MirrorPool,
    config: &BatchConfig,
    event_tx: mpsc::UnboundedSender<Event>,
    cancel_rx: watch::Receiver<bool>,
    defer: Arc<Notify>,
    drop_signal: Arc<Notify>,
) -> BeatmapsetDownloadOutcome {
    debug!(beatmapset_id, "starting download");

    let event_tx_progress = event_tx.clone();
    let progress_state = Arc::new(Mutex::new((0u64, Instant::now())));
    let progress_callback = Arc::new(move |downloaded: u64, total: u64| {
        let speed_bps = {
            let mut state = progress_state.lock().unwrap();
            let (last_bytes, last_time) = *state;
            let now = Instant::now();
            let elapsed = now.duration_since(last_time).as_secs_f64();
            let speed = if elapsed > 0.0 && downloaded > last_bytes {
                ((downloaded - last_bytes) as f64 / elapsed) as u64
            } else {
                0
            };
            *state = (downloaded, now);
            speed
        };
        let _ = event_tx_progress.send(Event::Progress {
            beatmapset_id,
            downloaded_bytes: downloaded,
            total_bytes: if total > 0 { Some(total) } else { None },
            speed_bps,
        });
    });

    let event_tx_status = event_tx.clone();
    let status_callback = Arc::new(move |status: Status| {
        let _ = event_tx_status.send(Event::BeatmapsetStatus {
            beatmapset_id,
            status,
        });
    });

    let mut outcome;
    let mut attempts_remaining = config.network_retry_attempts;
    loop {
        outcome = download_beatmapset(download::DownloadParams {
            beatmapset_id,
            output_dir,
            client,
            mirror_pool,
            archive_validation: config.archive_validation,
            progress_timeout: config.progress_timeout,
            sanitize_filenames: config.sanitize_filenames,
            on_exists: config.on_exists,
            callbacks: BeatmapsetDownloadCallbacks {
                progress: Some(progress_callback.clone()),
                status: Some(status_callback.clone()),
            },
            cancel_rx: cancel_rx.clone(),
            defer_signal: defer.clone(),
            drop_signal: drop_signal.clone(),
            inline_wait_max: config.inline_wait_max,
            rate_limit_skip_after: config.rate_limit_skip_after,
            #[cfg(feature = "instrument")]
            attempt_observer: config.attempt_observer.clone(),
            #[cfg(feature = "instrument")]
            session_start: config.session_start,
        })
        .await
        .0;

        if !matches!(outcome, BeatmapsetDownloadOutcome::NetworkError { .. })
            || attempts_remaining == 0
            || *cancel_rx.borrow()
        {
            break;
        }

        attempts_remaining -= 1;
        let cancelled = tokio::select! {
            _ = tokio::time::sleep(NETWORK_RETRY_BACKOFF) => false,
            changed = async {
                let mut rx = cancel_rx.clone();
                rx.changed().await
            } => changed.is_err() || *cancel_rx.borrow(),
        };
        if cancelled {
            break;
        }
    }

    outcome
}

async fn wait_until_cancelled(cancel_rx: &mut watch::Receiver<bool>) {
    loop {
        if *cancel_rx.borrow_and_update() {
            return;
        }
        if cancel_rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
#[path = "../tests/batch.rs"]
mod tests;
