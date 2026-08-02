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
    spawn_download, spawn_ids_download, spawn_selective_download, try_remove_empty_output_dir,
};
pub(crate) use session::{ids_folder_name, selective_folder_name};

pub use crate::config::constants::status;
pub use osu_downloader::ArchiveValidation;

use crate::app::collection::FailureReason;
use crate::core::collection::Collection;
use crate::mirrors::Mirror;
use crate::osu_db::OsuClient;
use fs2::available_space;
use osu_downloader::size::SizeFetcher;
use std::collections::{HashMap, HashSet};
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
    /// Beatmapsets that failed a previous run for this collection and that the
    /// user chose *not* to retry, resolved by the pre-download retry prompt
    /// (see `RetryFailedOnDownload`). They are dropped from the run's target
    /// list before precheck, so the page's total, its gauge denominator and its
    /// queued count all describe only what the run enqueues. The resolved
    /// collection payload is left whole, so `collection.db` still records every
    /// set. Empty means retry them (the whole collection is targeted).
    pub previously_failed_skipped: HashSet<u32>,
    /// Pre-skip beatmapsets already in the osu! library before downloading
    /// (they still land in `collection.db`). The owned-id set is resolved off
    /// the UI thread in the pipeline task; `osu_client` + `osu_path` are the
    /// cheap inputs read synchronously at request build.
    pub skip_already_imported: bool,
    pub osu_client: OsuClient,
    pub osu_path: String,
    /// The collection payload the resolve already fetched for display, when it is
    /// still fresh. `None` makes the pipeline fetch it itself.
    pub prefetched: Option<Collection>,
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
    /// Pre-skip beatmapsets already in the osu! library, on the same toggle and
    /// through the same off-thread resolve as [`DownloadRequest`] — a part-picked
    /// collection honors it exactly as the whole one does. A pre-skipped set is
    /// folded into the run's satisfied set, so it still reaches the selective
    /// `collection.db` and still counts toward the snapshot gate: the user has
    /// the map, which is the only thing either asks about.
    pub skip_already_imported: bool,
    pub osu_client: OsuClient,
    pub osu_path: String,
    /// Collection payloads an update scan / collection resolve already fetched,
    /// keyed by collection id. A hit short-circuits that collection's fetch during
    /// resolve; a miss fetches as before.
    pub prefetched: HashMap<u32, Collection>,
}

/// Which Get Maps source produced a raw-ids run. Decides the run's uploader
/// label and its output-subdir prefix (`search-*` / `filter-*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdsRunSource {
    Search,
    Filter,
}

impl IdsRunSource {
    /// Uploader shown on the run's page (raw-ids runs have no collection owner).
    pub(crate) fn uploader(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Filter => "filter",
        }
    }

    /// Prefix of the per-run output subdir (`<prefix>-<folder_tag>`).
    pub(crate) fn folder_prefix(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Filter => "filter",
        }
    }
}

impl From<crate::app::FindBackend> for IdsRunSource {
    fn from(backend: crate::app::FindBackend) -> Self {
        match backend {
            crate::app::FindBackend::Nzbasic => Self::Filter,
            crate::app::FindBackend::Osu => Self::Search,
        }
    }
}

/// A fetch-skipping download of raw beatmapset ids from a search or filter run.
/// Unlike [`DownloadRequest`] / [`SelectiveDownloadRequest`] there is no
/// collection to resolve — the results already carry the ids, so the pipeline
/// skips the osu!collector fetch and the `collection.db` write. `label` names
/// the run (the page title / `CollectionReady` name); `folder_tag` derives the
/// per-run output subdir (`<source>-<folder_tag>`), so different queries land
/// in different dirs.
#[derive(Debug, Clone)]
pub struct IdsDownloadRequest {
    pub beatmapset_ids: Vec<u32>,
    pub label: String,
    pub folder_tag: String,
    pub source: IdsRunSource,
    pub config: DownloadConfig,
    pub auto_overwrite: bool,
    /// Pre-skip beatmapsets already in the osu! library (they still count toward
    /// the run tally). The owned-id set is resolved off the UI thread in the
    /// pipeline task, exactly like [`DownloadRequest`].
    pub skip_already_imported: bool,
    pub osu_client: OsuClient,
    pub osu_path: String,
    /// Sizes already known for these ids at request time (the osu route's lazy
    /// nekoha probe cache, the nzbasic route's free per-set `SizeMap`). Seeds
    /// the run's size estimate so a fully-cached selection needs no probe.
    pub known_sizes: HashMap<u32, u64>,
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
    /// Some requested collections could not be fetched at resolve time, so
    /// their beatmapsets download without collection membership in
    /// `collection.db`. Surfaced as a one-shot warning toast.
    CollectionsUnresolved {
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

/// Cap on how many still-unknown ids get an actual nekoha probe; the rest are
/// estimated by scaling the sample's mean-per-set, so a huge run (e.g. a
/// 2000-map collection) never fires one GET per beatmapset at a mirror that
/// is also in the download pool.
const SIZE_SAMPLE_CAP: usize = 48;

/// The estimate is a cosmetic total for the `X/Y` bytes line and the ETA, not
/// a download precondition, so it never blocks the run: ids already known
/// (`known_sizes`, seeded from the nzbasic fetch or the osu size-probe cache)
/// cost nothing, and the rest are sampled rather than probed one-for-one.
pub(crate) async fn fetch_collection_sizes(
    id: DownloadId,
    beatmapset_ids: &[u32],
    known_sizes: &HashMap<u32, u64>,
    emit: Emit<'_>,
) {
    let known_sum: u64 = beatmapset_ids
        .iter()
        .filter_map(|bid| known_sizes.get(bid))
        .sum();
    let unknown: Vec<u32> = beatmapset_ids
        .iter()
        .copied()
        .filter(|bid| !known_sizes.contains_key(bid))
        .collect();

    if unknown.is_empty() {
        emit(DownloadEvent::CollectionSizeResolved {
            id,
            total_bytes: known_sum,
        });
        return;
    }

    let sample = sample_unknown(&unknown);
    let sample_len = sample.len();
    let fetcher = SizeFetcher::new();
    let result = fetcher.fetch_sizes(&sample).await;
    let total_bytes =
        estimate_total_bytes(known_sum, result.total_bytes, sample_len, unknown.len());

    emit(DownloadEvent::CollectionSizeResolved { id, total_bytes });
    if result.missing_count > 0 {
        debug!(
            missing = result.missing_count,
            sampled = sample_len,
            unknown = unknown.len(),
            "size info unavailable for some sampled beatmapsets"
        );
    }
}

/// Up to `SIZE_SAMPLE_CAP` ids spread evenly across `unknown` rather than a
/// low-id prefix: ids sort ascending and newer (higher-id) maps trend larger, so
/// the first N would bias the scaled estimate downward. `step` of 1 keeps every
/// id when the set already fits the cap.
fn sample_unknown(unknown: &[u32]) -> Vec<u32> {
    let sample_len = unknown.len().min(SIZE_SAMPLE_CAP);
    let step = (unknown.len() / SIZE_SAMPLE_CAP).max(1);
    unknown
        .iter()
        .copied()
        .step_by(step)
        .take(sample_len)
        .collect()
}

/// Pure arithmetic behind the estimate. `sample_bytes`/`sample_len` describe a
/// fetch over a capped sample of the still-unknown ids (already
/// mirror-extrapolated for any of THOSE the probe itself missed);
/// `unknown_len` is the full unprobed-id count the sample stands in for. A
/// sample covering every unknown id is used verbatim; a partial sample is
/// scaled by its per-set mean. Guards div-by-zero for an empty sample, mirroring
/// `SizeFetcher::fetch_sizes`' own fail-soft shape.
fn estimate_total_bytes(
    known_sum: u64,
    sample_bytes: u64,
    sample_len: usize,
    unknown_len: usize,
) -> u64 {
    if unknown_len == 0 || sample_len == 0 {
        return known_sum;
    }
    if sample_len >= unknown_len {
        return known_sum.saturating_add(sample_bytes);
    }
    let mean = sample_bytes / sample_len as u64;
    known_sum.saturating_add(mean.saturating_mul(unknown_len as u64))
}

#[cfg(test)]
#[path = "../../tests/unit/download_size.rs"]
mod tests;
