use super::super::{
    App, AppCommand, EnrichSink, EnrichTarget, Toast, collection_state, failed_maps, ignored_maps,
    messages::{clear_app_message, set_loading_message},
    snapshots,
    update_source::{MissingBeatmapset, MissingStatus, ScanStatus, extract_collection_id},
};
use crate::{
    config::constants::CONCURRENT_REQUESTS,
    core::collection::{Collection, api_client},
    osu_db::{
        BeatmapReader, LazerReader, LocalBeatmapset, LocalCollection, Md5, OsuClient, StableReader,
        checksum,
    },
};
use futures_util::{StreamExt, stream};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use tokio::sync::mpsc;
use tracing::{debug, info, trace};

type DatabaseReadResult = (Vec<LocalCollection>, Vec<LocalBeatmapset>, Vec<Md5>);

#[derive(Debug, Clone)]
pub enum UpdatesEvent {
    DatabaseRead {
        generation: u64,
        collections: Vec<LocalCollection>,
        beatmapsets: Vec<LocalBeatmapset>,
        all_checksums: Vec<Md5>,
    },
    Progress {
        generation: u64,
        message: String,
    },
    ScanComplete {
        generation: u64,
        missing: Vec<MissingBeatmapset>,
        collection_seen: HashMap<u32, Vec<u32>>,
        /// Number of local-snapshot checksums absent from upstream, per collection_id.
        collection_removed_counts: HashMap<u32, usize>,
        /// The payloads the scan fetched to compute the diff, parked in the session
        /// collection cache so a selective download reuses them.
        collections: Vec<Collection>,
        manually_added_count: usize,
        hidden_failed_count: usize,
    },
    FailedMapRecheckProgress {
        generation: u64,
        checked: usize,
        total: usize,
    },
    FailedMapRecheckComplete {
        generation: u64,
        available: HashSet<u32>,
        unavailable: HashSet<u32>,
    },
    Error(String),
}

/// Fold an updates event into the app. Returns a follow-up command — the
/// auto-fetch of the missing-set enrichment's first page after a scan lands — for
/// the runtime loop to dispatch; every other arm returns `None`.
pub(super) fn handle_updates_event(
    event: UpdatesEvent,
    app: &mut App,
    updates_tx: &mpsc::UnboundedSender<UpdatesEvent>,
) -> Option<AppCommand> {
    match event {
        UpdatesEvent::DatabaseRead {
            generation,
            collections,
            beatmapsets,
            all_checksums,
        } => {
            // Ignore stale results from previous scan
            if generation != app.home.update.scan.scan_generation {
                debug!(
                    expected = app.home.update.scan.scan_generation,
                    got = generation,
                    "Ignoring stale DatabaseRead event"
                );
                return None;
            }

            app.home.update.set_collections(collections);
            app.home.update.set_local_beatmapsets(beatmapsets);
            app.home.update.set_all_checksums(all_checksums);
            // Surface what was actually read so a misconfigured path shows up as
            // a low/zero set count instead of a silent all-missing result.
            let local_set_count = app.home.update.scan.local_beatmapsets.len();
            let scan_path = app.library.osu_path.value.clone();
            app.home.update.scan.scan_status = ScanStatus::FetchingCollection;
            set_loading_message(
                &mut app.home.update.message,
                format!(
                    "read {local_set_count} local mapsets from {scan_path} · fetching collections…"
                ),
            );

            let selected_ids = app.home.update.selected_collection_ids();
            if selected_ids.is_empty() {
                app.home.update.scan.scan_status = ScanStatus::Ready;
                clear_app_message(&mut app.home.update.message);
                app.toast_info("no collections with ids found to compare");
                return None;
            }

            spawn_fetch_task(app, selected_ids, updates_tx.clone());
            None
        }
        UpdatesEvent::Progress {
            generation,
            message,
        } => {
            if generation == app.home.update.scan.scan_generation {
                set_loading_message(&mut app.home.update.message, message);
            }
            None
        }
        UpdatesEvent::ScanComplete {
            generation,
            missing,
            collection_seen,
            collection_removed_counts,
            collections,
            manually_added_count,
            hidden_failed_count,
        } => {
            // Ignore stale results from previous scan
            if generation != app.home.update.scan.scan_generation {
                debug!(
                    expected = app.home.update.scan.scan_generation,
                    got = generation,
                    "Ignoring stale ScanComplete event"
                );
                return None;
            }

            let local_ids: HashSet<u32> = app
                .home
                .update
                .scan
                .local_beatmapsets
                .iter()
                .map(|bs| bs.id)
                .collect();
            let local_snapshot: Vec<u32> = local_ids.iter().copied().collect();
            // The selective download of these results resolves the very same
            // collections; keeping the scan's payloads spares it a verbatim refetch.
            for collection in collections {
                app.home.collection_cache.insert(collection.id, collection);
            }
            // Seeds the missing-set enrichment pager against the session cache; the
            // follow-up below kicks its first page so titles load before the browse
            // opens (disjoint field paths — `update` vs `meta_cache`).
            app.home
                .update
                .set_missing_beatmaps(missing, &app.home.meta_cache);
            // Re-home any manually-marked-installed rows from the ignore file so
            // they stay visible and reversible after a scan (full data when held
            // in memory, id-only placeholder otherwise).
            if let Some(path) = ignored_maps::ignored_maps_path() {
                let still_ignored = ignored_maps::load(&path).ids();
                app.home.update.sync_marked_installed(&still_ignored);
            }
            app.home
                .update
                .set_removed_counts(&collection_removed_counts);
            app.home
                .update
                .set_failed_beatmapset_count(hidden_failed_count);
            app.home.update.scan.scan_status = ScanStatus::Ready;

            // Read the counts back off the app rather than off `missing`: the
            // toast then quotes the same derivation the form badge and the run's
            // id list do, so the three cannot drift.
            let count = app.home.update.total_new_count();
            let held_back = app.home.update.held_back_count();
            let (title, detail) =
                build_scan_summary(count, held_back, manually_added_count, hidden_failed_count);
            clear_app_message(&mut app.home.update.message);
            let mut toast = if count == 0 && held_back == 0 {
                Toast::success(title)
            } else {
                Toast::info(title)
            };
            if let Some(detail) = detail {
                toast = toast.with_detail(detail);
            }
            app.push_toast(toast);

            for (collection_id, ids) in collection_seen {
                let installed_ids: Vec<u32> = ids
                    .iter()
                    .copied()
                    .filter(|id| local_ids.contains(id))
                    .collect();
                app.collection_state.update(
                    collection_id,
                    ids,
                    installed_ids,
                    local_snapshot.clone(),
                );
            }
            if let Some(path) = app.collection_state_path.clone() {
                let state = app.collection_state.clone();
                tokio::task::spawn_blocking(move || collection_state::save(&state, &path));
            }
            // Kick the missing-set enrichment's first page if the scan-land seed
            // left anything to fetch (a cache-only result needs none).
            app.home
                .update
                .has_more_enrichment()
                .then_some(AppCommand::LoadEnrichment {
                    target: EnrichTarget::Update,
                })
        }
        UpdatesEvent::FailedMapRecheckProgress {
            generation,
            checked,
            total,
        } => {
            if generation == app.home.update.scan.scan_generation {
                set_loading_message(
                    &mut app.home.update.message,
                    format!("rechecking known bad mapsets {checked}/{total}…"),
                );
            }
            None
        }
        UpdatesEvent::FailedMapRecheckComplete {
            generation,
            available,
            unavailable,
        } => {
            if generation != app.home.update.scan.scan_generation {
                return None;
            }
            clear_app_message(&mut app.home.update.message);
            let mut toast = if available.is_empty() {
                Toast::info("no bad mapsets recovered")
            } else {
                Toast::success(format!(
                    "{} mapset{} now downloadable",
                    available.len(),
                    if available.len() == 1 { "" } else { "s" }
                ))
            };
            if !unavailable.is_empty() {
                toast = toast.with_detail(format!("{} still unavailable", unavailable.len()));
            }
            app.push_toast(toast);
            app.home.update.scan.scan_generation =
                app.home.update.scan.scan_generation.wrapping_add(1);
            spawn_scan_task(app, updates_tx.clone());
            None
        }
        UpdatesEvent::Error(msg) => {
            app.report_scan_error(msg);
            None
        }
    }
}

/// Post-scan toast copy: a short title (what an update run would fetch) plus an
/// optional detail line carrying the secondary counts, ` · `-separated per the
/// toast convention. `count` excludes the held-back sets, so a zero with
/// `held_back > 0` means "nothing to fetch", not "nothing missing" — the two
/// zero cases get different titles. The re-include key lives on the missing
/// list, not in this ephemeral toast.
fn build_scan_summary(
    count: usize,
    held_back: usize,
    manually_added: usize,
    hidden_failed: usize,
) -> (String, Option<String>) {
    let title = match (count, held_back) {
        (0, 0) => "no missing mapsets".to_string(),
        (0, _) => "nothing to fetch".to_string(),
        _ => format!(
            "{count} missing mapset{}",
            if count == 1 { "" } else { "s" }
        ),
    };

    let mut parts = Vec::new();
    if held_back > 0 {
        parts.push(format!("{held_back} previously deleted, held back"));
    }
    if manually_added > 0 {
        parts.push(format!("{manually_added} added since last scan"));
    }
    if hidden_failed > 0 {
        parts.push(format!("{hidden_failed} known bad"));
    }
    let detail = (!parts.is_empty()).then(|| parts.join(" · "));
    (title, detail)
}

pub(super) fn spawn_scan_task(app: &mut App, tx: mpsc::UnboundedSender<UpdatesEvent>) {
    if let Some(h) = app.scan_handle.take() {
        h.abort();
    }

    let client_type = app.library.client_type;
    let osu_path = PathBuf::from(app.library.osu_path());
    let generation = app.home.update.scan.scan_generation;

    app.home.update.scan.scan_status = ScanStatus::ReadingDatabase;
    clear_app_message(&mut app.home.update.message);
    set_loading_message(&mut app.home.update.message, "reading database…");

    let handle = tokio::spawn(async move {
        let result =
            tokio::task::spawn_blocking(move || read_local_database(client_type, osu_path))
                .await
                .map_err(|e| format!("scan task panicked: {e}"))
                .and_then(|r| r);

        match result {
            Ok((collections, beatmapsets, all_checksums)) => {
                let _ = tx.send(UpdatesEvent::DatabaseRead {
                    generation,
                    collections,
                    beatmapsets,
                    all_checksums,
                });
            }
            Err(err) => {
                let _ = tx.send(UpdatesEvent::Error(err));
            }
        }
    });
    app.scan_handle = Some(handle);
}

pub fn read_local_database(
    client_type: OsuClient,
    path: PathBuf,
) -> Result<DatabaseReadResult, String> {
    match client_type {
        OsuClient::Stable => {
            let reader = StableReader::new(path);
            let collections = reader.list_collections()?;
            let beatmapsets = reader.list_beatmapsets()?;
            let all_checksums = beatmapsets
                .iter()
                .flat_map(|bs| bs.beatmaps.iter().map(|b| b.checksum))
                .collect();
            Ok((collections, beatmapsets, all_checksums))
        }
        OsuClient::Lazer => {
            // Open realm once; calling list_collections/list_beatmapsets/list_all_checksums
            // individually would open the 167MB client.realm file three separate times.
            let reader = LazerReader::new(path);
            reader.read_all()
        }
    }
}

/// Beatmapset ids present in the selected client's library. Blocking (opens the
/// realm / parses `osu!.db`); callers wrap it in `spawn_blocking`. Used by the
/// Get-Maps pre-skip of already-imported sets, memoized in `library_cache`.
pub fn owned_beatmapset_ids(client_type: OsuClient, path: PathBuf) -> Result<HashSet<u32>, String> {
    read_local_database(client_type, path)
        .map(|(_, beatmapsets, _)| beatmapsets.into_iter().map(|bs| bs.id).collect())
}

pub fn collection_ids_for_scan(selected_ids: Vec<u64>) -> Vec<u32> {
    selected_ids
        .into_iter()
        .filter_map(|id| u32::try_from(id).ok())
        .collect()
}

pub fn snapshot_diffs_for_scan(
    snapshot_dir: &std::path::Path,
    selected_collection_ids: &[u32],
    current_snapshots: &HashMap<u32, snapshots::CollectionSnapshotFile>,
) -> HashMap<u32, snapshots::SnapshotDiff> {
    selected_collection_ids
        .iter()
        .filter_map(|&collection_id| {
            let current = current_snapshots.get(&collection_id)?;
            let path = snapshots::snapshot_path(snapshot_dir, collection_id);
            let previous = snapshots::load(&path);
            let diff = snapshots::diff_snapshot(
                previous.as_ref().map(|snapshot| &snapshot.snapshot),
                &current.snapshot,
            );
            Some((collection_id, diff))
        })
        .collect()
}

pub(super) fn spawn_failed_map_recheck_task(
    app: &mut App,
    tx: mpsc::UnboundedSender<UpdatesEvent>,
) {
    if let Some(h) = app.scan_handle.take() {
        h.abort();
    }

    let generation = app.home.update.scan.scan_generation;
    let Some(path) = failed_maps::failed_maps_path() else {
        app.toast_info("no known bad mapsets to recheck");
        return;
    };
    let ids: Vec<u32> = failed_maps::load(&path).beatmapset_ids;
    if ids.is_empty() {
        app.toast_info("no known bad mapsets to recheck");
        return;
    }

    app.home.update.scan.scan_status = ScanStatus::CheckingFailedMaps;
    set_loading_message(
        &mut app.home.update.message,
        format!("rechecking known bad mapsets 0/{}…", ids.len()),
    );

    let handle = tokio::spawn(async move {
        let fetcher = osu_downloader::size::SizeFetcher::new();
        let progress_tx = tx.clone();
        // Availability is an anonymous probe, so drop auth-gated mirrors (osu!
        // official) — they'd 403 without a token and waste requests.
        let mirrors: Vec<_> = osu_downloader::Mirror::builtins()
            .into_iter()
            .filter(|mirror| !mirror.kind().requires_auth())
            .collect();
        let result = fetcher
            .check_availability(&ids, &mirrors, |checked, total| {
                let _ = progress_tx.send(UpdatesEvent::FailedMapRecheckProgress {
                    generation,
                    checked,
                    total,
                });
            })
            .await;
        failed_maps::remove_available(&path, &result.available);
        let _ = tx.send(UpdatesEvent::FailedMapRecheckComplete {
            generation,
            available: result.available,
            unavailable: result.unavailable,
        });
    });
    app.scan_handle = Some(handle);
}

fn spawn_fetch_task(
    app: &mut App,
    selected_ids: Vec<u64>,
    tx: mpsc::UnboundedSender<UpdatesEvent>,
) {
    if let Some(h) = app.scan_handle.take() {
        h.abort();
    }

    let selected_collection_ids = collection_ids_for_scan(selected_ids);
    let local_set_ids: HashSet<u32> = app
        .home
        .update
        .scan
        .local_beatmapsets
        .iter()
        .map(|bs| bs.id)
        .collect();
    let all_local_checksums = std::mem::take(&mut app.home.update.scan.all_local_checksums);
    let local_collections_raw = app.home.update.scan.local_collections_raw.clone();
    let generation = app.home.update.scan.scan_generation;
    let client_type = app.library.client_type;
    let current_snapshots = snapshots::current_snapshots(
        client_type,
        &app.home.update.scan.local_collections_raw,
        app.home.update.scan.local_beatmapsets.iter(),
        |name| extract_collection_id(name).and_then(|id| u32::try_from(id).ok()),
    );
    let snapshot_dir = snapshots::snapshots_dir();
    let snapshot_diffs = snapshot_dir
        .as_deref()
        .map(|dir| snapshot_diffs_for_scan(dir, &selected_collection_ids, &current_snapshots))
        .unwrap_or_default();
    let added_count = snapshot_diffs
        .values()
        .map(|diff| diff.manually_added.len())
        .sum();
    let failed_beatmapset_ids = failed_maps::failed_maps_path()
        .as_deref()
        .map(failed_maps::load)
        .map(|failed_maps| failed_maps.ids())
        .unwrap_or_default();
    // Drop any manually-ignored id that is now genuinely installed, then hide
    // the rest from this scan.
    let ignored_beatmapset_ids = ignored_maps::ignored_maps_path()
        .map(|path| ignored_maps::reconcile_installed(&path, &local_set_ids))
        .unwrap_or_default();

    app.home.update.scan.scan_status = ScanStatus::FetchingCollection;

    let handle = tokio::spawn(async move {
        let result = fetch_missing_beatmapsets(
            client_type,
            selected_collection_ids,
            local_set_ids,
            all_local_checksums,
            &local_collections_raw,
            snapshot_diffs,
            FetchCompareSettings {
                hidden_failed_beatmapset_ids: failed_beatmapset_ids,
                ignored_beatmapset_ids,
            },
        )
        .await;

        match result {
            Ok(res) => {
                let _ = tx.send(UpdatesEvent::ScanComplete {
                    generation,
                    missing: res.missing,
                    collection_seen: res.collection_seen,
                    collection_removed_counts: res.collection_removed_counts,
                    collections: res.collections,
                    manually_added_count: added_count,
                    hidden_failed_count: res.hidden_failed_count,
                });
            }
            Err(err) => {
                let _ = tx.send(UpdatesEvent::Error(err));
            }
        }
    });
    app.scan_handle = Some(handle);
}

#[derive(Debug, Clone, Default)]
pub struct FetchCompareSettings {
    /// Known-bad maps (auto-recorded download failures), cleared by a recheck.
    pub hidden_failed_beatmapset_ids: HashSet<u32>,
    /// Maps the user manually marked as installed, cleared when a later scan
    /// detects a genuine install.
    pub ignored_beatmapset_ids: HashSet<u32>,
}

/// Whether a beatmapset is dismissed noise the scan must not surface as missing:
/// either a known-bad map or one the user manually marked as installed.
pub fn should_hide_failed_beatmapset(settings: &FetchCompareSettings, beatmapset_id: u32) -> bool {
    settings
        .hidden_failed_beatmapset_ids
        .contains(&beatmapset_id)
        || settings.ignored_beatmapset_ids.contains(&beatmapset_id)
}

/// What a collection beatmapset resolves to once the by-id local-install check
/// and the per-collection dedupe have already passed. Kept a pure, network-free
/// step so this per-beatmapset verdict can be unit-tested apart from the API
/// fetch it's normally embedded in — `Hidden` covers both known-bad and
/// manually-ignored ids, but only the former joins the scan's counted
/// suppression set (see [`scan_collection_candidates`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BeatmapsetVerdict {
    /// All of its checksums already exist locally under a different online id.
    LocallyPresent,
    /// Known-bad or manually-ignored; suppressed rather than surfaced as missing.
    Hidden,
    /// Genuinely missing and fetchable.
    Candidate,
}

pub(crate) fn classify_beatmapset(
    beatmapset_id: u32,
    api_checksums: &[Md5],
    local_checksums: &HashSet<Md5>,
    settings: &FetchCompareSettings,
) -> BeatmapsetVerdict {
    if !api_checksums.is_empty() && api_checksums.iter().all(|cs| local_checksums.contains(cs)) {
        return BeatmapsetVerdict::LocallyPresent;
    }
    if should_hide_failed_beatmapset(settings, beatmapset_id) {
        return BeatmapsetVerdict::Hidden;
    }
    BeatmapsetVerdict::Candidate
}

/// One fetched collection's contribution to the scan's candidate list, deduped
/// by beatmapset id within the collection, plus the ids of its beatmapsets this
/// scan suppressed via the failed-maps store specifically. A manually-ignored
/// id is suppressed from candidates the same way (see [`classify_beatmapset`]'s
/// `Hidden` verdict) but is NOT counted here: the "known bad" figure and the
/// recheck flow (`spawn_failed_map_recheck_task`) only ever read the
/// failed-maps store, so folding the ignore store in would report ids recheck
/// can't act on. Pure and network-free, so the suppression set — the figure
/// defect B miscounted — is unit-testable apart from the API fetch it's
/// normally embedded in.
pub(crate) fn scan_collection_candidates(
    collection: &Collection,
    collection_id: u32,
    local_set_ids: &HashSet<u32>,
    local_checksums: &HashSet<Md5>,
    settings: &FetchCompareSettings,
) -> (Vec<(CollectionBeatmapset, u32, String)>, HashSet<u32>) {
    // Dedupe within this collection only — the same beatmapset can appear
    // in multiple collections and must be tracked per collection_id.
    let mut seen_in_collection: HashSet<u32> = HashSet::new();
    let mut candidates = Vec::new();
    let mut hidden_failed_ids: HashSet<u32> = HashSet::new();

    for beatmapset in &collection.beatmapsets {
        if !seen_in_collection.insert(beatmapset.id) {
            continue;
        }

        // Skip if beatmapset exists locally (by ID)
        if local_set_ids.contains(&beatmapset.id) {
            trace!(beatmapset_id = beatmapset.id, "Found by ID, skipping");
            continue;
        }

        // ID not found - check if ALL checksums exist locally (handles beatmapsets with invalid OnlineID)
        let api_checksums: Vec<Md5> = beatmapset
            .beatmaps
            .iter()
            .filter(|bm| !bm.checksum.is_empty())
            .filter_map(|bm| checksum::parse_hex(&bm.checksum))
            .collect();

        match classify_beatmapset(beatmapset.id, &api_checksums, local_checksums, settings) {
            BeatmapsetVerdict::LocallyPresent => {
                trace!(
                    beatmapset_id = beatmapset.id,
                    "ID not found but all checksums exist locally, skipping"
                );
            }
            BeatmapsetVerdict::Hidden => {
                trace!(beatmapset_id = beatmapset.id, "skipping failed beatmapset");
                if settings
                    .hidden_failed_beatmapset_ids
                    .contains(&beatmapset.id)
                {
                    hidden_failed_ids.insert(beatmapset.id);
                }
            }
            BeatmapsetVerdict::Candidate => {
                trace!(
                    beatmapset_id = beatmapset.id,
                    "not installed, adding to candidates"
                );
                candidates.push((
                    CollectionBeatmapset {
                        id: beatmapset.id,
                        checksums: beatmapset
                            .beatmaps
                            .iter()
                            .filter_map(|beatmap| checksum::parse_hex(&beatmap.checksum))
                            .collect(),
                        enrich_diff_id: beatmapset.beatmaps.first().map(|beatmap| beatmap.id),
                    },
                    collection_id,
                    collection.name.to_string(),
                ));
            }
        }
    }

    (candidates, hidden_failed_ids)
}

/// Every already-fetched collection's contribution, combined: the full
/// candidate list, and the SET of beatmapset ids this scan hid via the
/// failed-maps store, unioned across every scanned collection. A union, not a
/// running sum of per-collection counts — the same id can be hidden in more
/// than one scanned collection and must count once (the double-count defect A's
/// fix re-introduced in this counter). Pure and network-free, so a
/// multi-collection fixture drives it directly instead of the API fetch that
/// normally produces `fetched`.
pub(crate) fn scan_fetched_collections(
    fetched: &[(u32, Collection)],
    local_set_ids: &HashSet<u32>,
    local_checksums: &HashSet<Md5>,
    settings: &FetchCompareSettings,
) -> (Vec<(CollectionBeatmapset, u32, String)>, HashSet<u32>) {
    let mut candidates_to_check = Vec::new();
    let mut hidden_failed_ids: HashSet<u32> = HashSet::new();

    for (collection_id, collection) in fetched {
        let (candidates, collection_hidden_ids) = scan_collection_candidates(
            collection,
            *collection_id,
            local_set_ids,
            local_checksums,
            settings,
        );
        candidates_to_check.extend(candidates);
        hidden_failed_ids.extend(collection_hidden_ids);
    }

    (candidates_to_check, hidden_failed_ids)
}

#[derive(Debug, Clone)]
pub(crate) struct CollectionBeatmapset {
    pub(crate) id: u32,
    pub(crate) checksums: Vec<Md5>,
    /// One diff (beatmap) id from this set's upstream listing, forwarded to the
    /// missing-set enrichment pager (the osu-batch keys on diff ids).
    pub(crate) enrich_diff_id: Option<u32>,
}

impl CollectionBeatmapset {
    pub(crate) fn is_in_snapshot(
        &self,
        client_type: OsuClient,
        snapshot: &snapshots::CollectionSnapshot,
    ) -> bool {
        match client_type {
            OsuClient::Stable => {
                // stable_hashes are persisted as hex strings; parse once for the lookup
                let deleted_hashes: HashSet<Md5> = snapshot
                    .stable_hashes
                    .iter()
                    .filter_map(|h| checksum::parse_hex(h))
                    .collect();
                self.checksums
                    .iter()
                    .any(|cksum| !checksum::is_empty(cksum) && deleted_hashes.contains(cksum))
            }
            OsuClient::Lazer => snapshot.lazer_ids.contains(&u64::from(self.id)),
        }
    }
}

/// Result of `fetch_missing_beatmapsets`.
pub struct FetchMissingResult {
    pub missing: Vec<MissingBeatmapset>,
    /// Upstream beatmapset IDs seen per collection.
    pub collection_seen: HashMap<u32, Vec<u32>>,
    /// Per-collection count of local checksums absent from the upstream collection.
    pub collection_removed_counts: HashMap<u32, usize>,
    /// The fetched upstream payloads, in fetch order. The TUI parks them in its
    /// session collection cache; the headless CLI ignores them.
    pub collections: Vec<Collection>,
    /// Ids this scan itself suppressed via the failed-maps store specifically
    /// (manually-ignored ids are also suppressed from `missing` but are not
    /// "known bad" — see [`scan_collection_candidates`]). Deduped across every
    /// scanned collection, and scoped to what this scan actually walked — not
    /// the size of the failed-maps store, which can hold ids outside the
    /// scanned collections entirely.
    pub hidden_failed_count: usize,
}

pub async fn fetch_missing_beatmapsets(
    client_type: OsuClient,
    collection_ids: Vec<u32>,
    local_set_ids: HashSet<u32>,
    local_checksums: HashSet<Md5>,
    local_collections_raw: &[LocalCollection],
    snapshot_diffs: HashMap<u32, snapshots::SnapshotDiff>,
    settings: FetchCompareSettings,
) -> Result<FetchMissingResult, String> {
    let client = osu_downloader::collection::CollectionClient::new();
    let mut collection_seen: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut collection_removed_counts: HashMap<u32, usize> = HashMap::new();

    debug!(
        local_beatmapset_count = local_set_ids.len(),
        local_checksums_count = local_checksums.len(),
        "Starting fetch_and_compare"
    );

    // Build a fast lookup: collection_id → local checksums for that collection
    let local_collection_checksums: HashMap<u32, HashSet<Md5>> = local_collections_raw
        .iter()
        .filter_map(|c| {
            let id = extract_collection_id(&c.name).and_then(|id| u32::try_from(id).ok())?;
            let set: HashSet<Md5> = c
                .beatmap_checksums
                .iter()
                .copied()
                .filter(|cs| !checksum::is_empty(cs))
                .collect();
            Some((id, set))
        })
        .collect();

    let t_api = std::time::Instant::now();

    // Fetch all collections concurrently, then process results sequentially
    let fetched: Vec<Result<(u32, Collection), String>> = stream::iter(collection_ids)
        .map(|collection_id| {
            let client = client.clone();
            async move {
                api_client::fetch_collection(&client, collection_id)
                    .await
                    .map(|c| (collection_id, c))
                    .map_err(|e| e.to_string())
            }
        })
        .buffer_unordered(CONCURRENT_REQUESTS)
        .collect()
        .await;

    info!(
        elapsed_ms = t_api.elapsed().as_millis(),
        "phase: API fetch collections"
    );

    // Errors abort the whole scan regardless of when they're noticed (the
    // function returns `Result<FetchMissingResult, _>`, all-or-nothing), so
    // resolving every fetch up front — rather than mid-loop — changes nothing
    // observable and lets the per-collection candidate scan run as a separate,
    // pure, testable pass below.
    let mut resolved: Vec<(u32, Collection)> = Vec::with_capacity(fetched.len());
    for fetch_result in fetched {
        resolved.push(fetch_result?);
    }

    for (collection_id, collection) in &resolved {
        let collection_id = *collection_id;

        debug!(
            collection_id,
            collection_name = %collection.name,
            beatmapset_count = collection.beatmapsets.len(),
            "Fetched collection from API"
        );

        let api_ids: Vec<u32> = collection.beatmapsets.iter().map(|b| b.id).collect();
        collection_seen.insert(collection_id, api_ids);

        // Compute removed count: local checksums for this collection absent from upstream.
        // Both sides use the same Md5 ([u8;16]) type; upstream hex strings are parsed via
        // checksum::parse_hex, same as the stable reader does when building local_collections_raw.
        let upstream_checksums: HashSet<Md5> = collection
            .beatmapsets
            .iter()
            .flat_map(|bs| bs.beatmaps.iter())
            .filter(|bm| !bm.checksum.is_empty())
            .filter_map(|bm| checksum::parse_hex(&bm.checksum))
            .filter(|cs| !checksum::is_empty(cs))
            .collect();

        let removed = local_collection_checksums
            .get(&collection_id)
            .map(|local| local.difference(&upstream_checksums).count())
            .unwrap_or(0);

        if removed > 0 {
            collection_removed_counts.insert(collection_id, removed);
        }
    }

    let (candidates_to_check, hidden_failed_ids) =
        scan_fetched_collections(&resolved, &local_set_ids, &local_checksums, &settings);
    let fetched_collections: Vec<Collection> = resolved.into_iter().map(|(_, c)| c).collect();

    debug!(
        candidates = candidates_to_check.len(),
        "finished scanning collections"
    );

    let mut all_missing: Vec<MissingBeatmapset> = Vec::new();

    for (beatmapset, collection_id, collection_name) in candidates_to_check {
        all_missing.push(missing_from_candidate(
            &beatmapset,
            collection_id,
            collection_name,
            client_type,
            &snapshot_diffs,
        ));
    }

    all_missing.sort_by(|a, b| {
        a.collection_id
            .cmp(&b.collection_id)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(FetchMissingResult {
        missing: all_missing,
        collection_seen,
        collection_removed_counts,
        collections: fetched_collections,
        hidden_failed_count: hidden_failed_ids.len(),
    })
}

/// Rebuild every scanned collection's snapshot so the sets this scan held back
/// stay recorded as deleted in it.
///
/// Both snapshot writers go through here — the TUI's completed-run persist
/// (`request_selective_download` → `persist_snapshots_if_complete`) and the
/// headless `update-collections` report — because both build their baseline from
/// [`snapshots::current_snapshots`], which reads the LOCAL library and therefore
/// omits exactly the sets the user deleted. Without this, finishing a run (or
/// merely running the report) resets the baseline and the next scan concludes
/// nothing was ever deleted.
pub fn retain_held_back_in_snapshots(
    snapshot_files: &mut HashMap<u32, snapshots::CollectionSnapshotFile>,
    client_type: OsuClient,
    missing: &[MissingBeatmapset],
) {
    // Holding nothing back is the overwhelmingly common scan, so settle it once
    // instead of re-walking `missing` and allocating a vec per collection to
    // discover the same emptiness N times.
    let held_back: Vec<&MissingBeatmapset> = missing.iter().filter(|set| !set.included).collect();
    if held_back.is_empty() {
        return;
    }
    for (collection_id, file) in snapshot_files.iter_mut() {
        let for_collection: Vec<(u32, &[Md5])> = held_back
            .iter()
            .filter(|set| set.collection_id == *collection_id)
            .map(|set| (set.id, set.checksums.as_ref()))
            .collect();
        if for_collection.is_empty() {
            continue;
        }
        snapshots::retain_held_back(&mut file.snapshot, client_type, for_collection);
    }
}

/// Turn one not-installed upstream set into a missing-list row, deciding whether
/// the user deleted it on purpose (it sits in this collection's
/// `manually_deleted` snapshot diff) and therefore whether an update run enqueues
/// it. This is the sole writer of both flags.
pub(crate) fn missing_from_candidate(
    beatmapset: &CollectionBeatmapset,
    collection_id: u32,
    collection_name: String,
    client_type: OsuClient,
    snapshot_diffs: &HashMap<u32, snapshots::SnapshotDiff>,
) -> MissingBeatmapset {
    let previously_deleted = snapshot_diffs
        .get(&collection_id)
        .map(|diff| beatmapset.is_in_snapshot(client_type, &diff.manually_deleted))
        .unwrap_or(false);

    if previously_deleted {
        trace!(
            beatmapset_id = beatmapset.id,
            "marking as previously deleted"
        );
    }

    MissingBeatmapset {
        id: beatmapset.id,
        status: MissingStatus::NotInstalled,
        collection_id,
        collection_name,
        // A set the user deleted on purpose is held back from the run until they
        // re-include it in the browse.
        included: !previously_deleted,
        previously_deleted,
        // Only a held-back set's checksums are ever read (to keep it recorded in
        // the stable snapshot), so a scan does not pay an allocation per missing
        // set for a field nothing will look at — and the lazer arm never reads
        // them at all.
        //
        // This must keep reading the SAME slice `is_in_snapshot` just read above:
        // that shared source is the only reason a set flagged deleted is
        // guaranteed to carry a hash the rebuild can re-express it with. Sourcing
        // it elsewhere fails open and silently (`snapshots::retain_held_back`).
        checksums: if previously_deleted {
            beatmapset.checksums.clone().into_boxed_slice()
        } else {
            Box::new([])
        },
        enrich_diff_id: beatmapset.enrich_diff_id,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/runtime_scan.rs"]
mod tests;
