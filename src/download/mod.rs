pub mod collection_db;
pub mod error;
pub mod events;
pub mod lock;
mod pipeline;
mod precheck;
mod session;

pub use collection_db::create_selective_collection_db;
pub use error::DownloadError;
pub use events::{Tally, translate_event};
pub use lock::ActiveDownloadRegistry;
pub use pipeline::{
    spawn_download, spawn_search_download, spawn_selective_download, try_remove_empty_output_dir,
};

pub use crate::config::constants::status;
pub use osu_downloader::ArchiveValidation;

use crate::app::collection::FailureReason;
use crate::mirrors::Mirror;
use crate::osu_db::OsuClient;
use fs2::available_space;
use osu_downloader::size::SizeFetcher;
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::{sync::watch, task::JoinHandle};
use tracing::{debug, warn};

use crate::utils::is_low_disk_space;

pub type DownloadId = u64;

/// Borrow-only emit reference used throughout pipeline/event code.
pub type Emit<'a> = &'a (dyn Fn(DownloadEvent) + Send + Sync);

/// Handle to a running download task.
pub struct DownloadHandle {
    cancel: watch::Sender<bool>,
    /// Generation counter: each bump asks the running session to defer (requeue)
    /// whatever maps are sitting on a rate-limit cooldown right now. A counter
    /// (not a bool) so repeated presses each register as a distinct `changed()`.
    defer: watch::Sender<u64>,
    /// Generation counter: each bump asks the running session to hard-drop
    /// whatever maps are sitting on a rate-limit cooldown right now. A counter
    /// (not a bool) so repeated presses each register as a distinct `changed()`.
    skip: watch::Sender<u64>,
    join: JoinHandle<()>,
}

impl DownloadHandle {
    pub(crate) fn new(
        cancel: watch::Sender<bool>,
        defer: watch::Sender<u64>,
        skip: watch::Sender<u64>,
        join: JoinHandle<()>,
    ) -> Self {
        Self {
            cancel,
            defer,
            skip,
            join,
        }
    }

    pub fn request_shutdown(&self) {
        let _ = self.cancel.send(true);
    }

    /// Ask the running session to defer (requeue) every map currently waiting on
    /// a mirror rate-limit cooldown, so it retries once a mirror frees rather
    /// than being dropped. No-op if the task has already finished.
    pub fn defer_rate_limited(&self) {
        self.defer.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Ask the running session to hard-drop every map currently waiting on a
    /// mirror rate-limit cooldown. No-op if the task has already finished.
    pub fn skip_rate_limited(&self) {
        self.skip.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub async fn wait(self) {
        let _ = self.join.await;
    }
}

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub directory: String,
    pub mirrors: Vec<Mirror>,
    pub concurrent: u8,
    pub archive_validation: ArchiveValidation,
    pub auto_skip_rate_limited: bool,
    pub rate_limit_skip_secs: u32,
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub collection_input: String,
    pub config: DownloadConfig,
    pub auto_overwrite: bool,
    /// Whether beatmaps that failed in a previous run for this collection
    /// should be retried as part of this download. Resolved by the
    /// pre-download retry prompt (see `RetryFailedOnDownload`).
    pub include_previously_failed: bool,
    /// Pre-skip beatmapsets already in the osu! library before downloading
    /// (they still land in `collection.db`). The owned-id set is resolved off
    /// the UI thread in the pipeline task; `osu_client` + `osu_path` are the
    /// cheap inputs read synchronously at request build.
    pub skip_already_imported: bool,
    pub osu_client: OsuClient,
    pub osu_path: String,
}

#[derive(Debug, Clone)]
pub struct SelectiveDownloadCollection {
    pub id: u32,
    pub name: String,
    pub beatmapset_ids: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct SelectiveDownloadRequest {
    pub collection_ids: Vec<u32>,
    pub beatmapset_ids: Vec<u32>,
    pub collections: Vec<SelectiveDownloadCollection>,
    pub config: DownloadConfig,
    pub snapshot_dir: Option<std::path::PathBuf>,
    pub snapshots: Vec<crate::app::snapshots::CollectionSnapshotFile>,
}

/// A fetch-skipping download of raw beatmapset ids from a search. Unlike
/// [`DownloadRequest`] / [`SelectiveDownloadRequest`] there is no collection to
/// resolve — search results already carry the ids, so the pipeline skips the
/// osu!collector fetch and the `collection.db` write. `label` names the run: it
/// derives the page title and the per-run output subdir (`search-<label>`, so
/// different query texts land in different dirs).
#[derive(Debug, Clone)]
pub struct SearchDownloadRequest {
    pub beatmapset_ids: Vec<u32>,
    pub label: String,
    pub config: DownloadConfig,
    pub auto_overwrite: bool,
    /// Pre-skip beatmapsets already in the osu! library (they still count toward
    /// the run tally). The owned-id set is resolved off the UI thread in the
    /// pipeline task, exactly like [`DownloadRequest`].
    pub skip_already_imported: bool,
    pub osu_client: OsuClient,
    pub osu_path: String,
}

/// A beatmapset that failed during a download run. Carried both in the
/// `FailedMaps` event and rendered in `CollectionPage::failed_maps`.
#[derive(Debug, Clone)]
pub struct FailedMap {
    pub beatmapset_id: u32,
    /// Beatmapset title, when the library was able to resolve it. `None` for
    /// failures that occurred before metadata was fetched.
    pub title: Option<String>,
    pub reason: FailureReason,
}

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    CollectionReady {
        id: DownloadId,
        collection_name: String,
        uploader: String,
        total_maps: usize,
        output_dir: String,
    },
    ResolveProgress {
        id: DownloadId,
        current: u32,
        total: u32,
    },
    CollectionSizeResolved {
        id: DownloadId,
        total_bytes: u64,
    },
    LowDiskSpace {
        id: DownloadId,
        available_bytes: u64,
    },
    VerifiedMapSizes {
        id: DownloadId,
        total_bytes: u64,
    },
    BeatmapProgress {
        id: DownloadId,
        beatmapset_id: u32,
        downloaded: u64,
        total: u64,
    },
    DownloadTarget {
        id: DownloadId,
        remaining: usize,
    },
    BeatmapStatus {
        id: DownloadId,
        beatmapset_id: u32,
        stage: BeatmapStage,
        message: String,
        rate_limited: bool,
        /// Instant at which the rate-limit cooldown expires. `Some` only when
        /// `rate_limited` is true; `None` for all other statuses.
        cooldown_until: Option<Instant>,
    },
    OverallProgress {
        id: DownloadId,
        downloaded: u32,
        skipped: u32,
        failed: u32,
        unverified: u32,
    },
    StageChanged {
        id: DownloadId,
        stage: DownloadStage,
    },
    FailedMaps {
        id: DownloadId,
        failures: Vec<FailedMap>,
    },
    /// Beatmapsets pre-skipped because they are already in the osu! library.
    /// Surfaced as a one-shot toast; the count is also folded into the run's
    /// skipped tally.
    SkippedImported {
        id: DownloadId,
        count: usize,
    },
    BeatmapVerified {
        id: DownloadId,
        duration_us: u64,
    },
    /// A beatmapset was deferred by the library: every candidate mirror was
    /// rate-limited past the inline-wait threshold, so it returned to the queue
    /// tail instead of parking a worker. The active slot frees and the map
    /// counts as queued again (never failed / skipped); it retries after
    /// `retry_in`. Toastless.
    BeatmapDeferred {
        id: DownloadId,
        beatmapset_id: u32,
        /// Rate-limit deferral count so far. A pure request-spacing requeue keeps
        /// the current count (may be 0); only cooling defers increment it.
        pass: u32,
        /// Time until the earliest candidate mirror frees.
        retry_in: Duration,
    },
    Finished {
        id: DownloadId,
        summary: DownloadSummary,
    },
    Failed {
        id: DownloadId,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatmapStage {
    Pending,
    Downloading,
    /// archive bytes done; lib is hashing/zip-validating/finalizing before emitting a terminal stage.
    Verifying,
    Success,
    Skipped,
    Failed,
    Aborted,
}

impl BeatmapStage {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Success | Self::Skipped | Self::Failed | Self::Aborted
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStage {
    Pending,
    Resolving,
    Rechecking,
    Downloading,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct DownloadSummary {
    pub downloaded: u32,
    pub skipped: u32,
    pub failed: u32,
    pub unverified: u32,
}

pub(crate) fn warn_low_disk_space(id: DownloadId, output_dir: &Path, emit: Emit<'_>) {
    if is_low_disk_space(output_dir)
        && let Ok(available) = available_space(output_dir)
    {
        warn!(
            available_bytes = available,
            output_dir = %output_dir.display(),
            "low disk space detected"
        );
        emit(DownloadEvent::LowDiskSpace {
            id,
            available_bytes: available,
        });
    }
}

pub(crate) async fn fetch_collection_sizes(id: DownloadId, beatmapset_ids: &[u32], emit: Emit<'_>) {
    let fetcher = SizeFetcher::new();
    let result = fetcher.fetch_sizes(beatmapset_ids).await;
    emit(DownloadEvent::CollectionSizeResolved {
        id,
        total_bytes: result.total_bytes,
    });
    if result.missing_count > 0 {
        debug!(
            missing = result.missing_count,
            "size info unavailable for some beatmapsets"
        );
    }
}
