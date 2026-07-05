use super::super::{
    App, Toast, collection_state, failed_maps, ignored_maps,
    messages::{clear_app_message, set_loading_message},
    snapshots,
    updates::{MissingBeatmapset, MissingStatus, ScanStatus, extract_collection_id},
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

pub(super) fn handle_updates_event(
    event: UpdatesEvent,
    app: &mut App,
    updates_tx: &mpsc::UnboundedSender<UpdatesEvent>,
) {
    match event {
        UpdatesEvent::DatabaseRead {
            generation,
            collections,
            beatmapsets,
            all_checksums,
        } => {
            // Ignore stale results from previous scan
            if generation != app.updates.scan.scan_generation {
                debug!(
                    expected = app.updates.scan.scan_generation,
                    got = generation,
                    "Ignoring stale DatabaseRead event"
                );
                return;
            }

            app.updates.set_collections(collections);
            app.updates.set_local_beatmapsets(beatmapsets);
            app.updates.set_all_checksums(all_checksums);
            // Surface what was actually read so a misconfigured path shows up as
            // a low/zero set count instead of a silent all-missing result.
            let local_set_count = app.updates.scan.local_beatmapsets.len();
            let scan_path = app.library.osu_path.value.clone();
            app.updates.scan.scan_status = ScanStatus::FetchingCollection;
            set_loading_message(
                &mut app.updates.message,
                format!(
                    "read {local_set_count} local sets from {scan_path} · fetching collections..."
                ),
            );

            let selected_ids = app.updates.selected_collection_ids();
            if selected_ids.is_empty() {
                app.updates.scan.scan_status = ScanStatus::Ready;
                clear_app_message(&mut app.updates.message);
                app.toast_info("no collections with ids found to compare");
                return;
            }

            spawn_fetch_task(app, selected_ids, updates_tx.clone());
        }
        UpdatesEvent::Progress {
            generation,
            message,
        } => {
            if generation == app.updates.scan.scan_generation {
                set_loading_message(&mut app.updates.message, message);
            }
        }
        UpdatesEvent::ScanComplete {
            generation,
            missing,
            collection_seen,
            collection_removed_counts,
            manually_added_count,
            hidden_failed_count,
        } => {
            // Ignore stale results from previous scan
            if generation != app.updates.scan.scan_generation {
                debug!(
                    expected = app.updates.scan.scan_generation,
                    got = generation,
                    "Ignoring stale ScanComplete event"
                );
                return;
            }

            let previously_deleted_count = missing.iter().filter(|m| m.previously_deleted).count();
            let local_ids: HashSet<u32> = app
                .updates
                .scan
                .local_beatmapsets
                .iter()
                .map(|bs| bs.id)
                .collect();
            let local_snapshot: Vec<u32> = local_ids.iter().copied().collect();
            let count = missing.len();
            app.updates.set_missing_beatmaps(missing);
            app.updates.set_removed_counts(&collection_removed_counts);
            app.updates.set_failed_beatmapset_count(hidden_failed_count);
            app.updates.scan.scan_status = ScanStatus::Ready;

            let (title, detail) = build_scan_summary(
                count,
                previously_deleted_count,
                manually_added_count,
                hidden_failed_count,
            );
            clear_app_message(&mut app.updates.message);
            let mut toast = if count == 0 {
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
        }
        UpdatesEvent::FailedMapRecheckProgress {
            generation,
            checked,
            total,
        } => {
            if generation == app.updates.scan.scan_generation {
                set_loading_message(
                    &mut app.updates.message,
                    format!("rechecking known bad maps {checked}/{total}..."),
                );
            }
        }
        UpdatesEvent::FailedMapRecheckComplete {
            generation,
            available,
            unavailable,
        } => {
            if generation != app.updates.scan.scan_generation {
                return;
            }
            clear_app_message(&mut app.updates.message);
            let mut toast = if available.is_empty() {
                Toast::info("no bad maps recovered")
            } else {
                Toast::success(format!("{} maps now downloadable", available.len()))
            };
            if !unavailable.is_empty() {
                toast = toast.with_detail(format!("{} still unavailable", unavailable.len()));
            }
            app.push_toast(toast);
            app.updates.scan.scan_generation = app.updates.scan.scan_generation.wrapping_add(1);
            spawn_scan_task(app, updates_tx.clone());
        }
        UpdatesEvent::Error(msg) => {
            app.report_scan_error(msg);
        }
    }
}

/// Post-scan toast copy: a short title (the missing count) plus an optional
/// detail line carrying the secondary counts, ` · `-separated per the toast
/// convention. The "re-select to download" hint for previously-deleted sets
/// lives on the missing list, not in this ephemeral toast.
fn build_scan_summary(
    count: usize,
    previously_deleted: usize,
    manually_added: usize,
    hidden_failed: usize,
) -> (String, Option<String>) {
    let title = if count == 0 {
        "no missing beatmapsets".to_string()
    } else {
        format!("{count} missing beatmapsets")
    };

    let mut parts = Vec::new();
    if previously_deleted > 0 {
        parts.push(format!("{previously_deleted} previously deleted"));
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
    let generation = app.updates.scan.scan_generation;

    app.updates.scan.scan_status = ScanStatus::ReadingDatabase;
    clear_app_message(&mut app.updates.message);
    set_loading_message(&mut app.updates.message, "Reading database...");

    let handle = tokio::spawn(async move {
        let result =
            tokio::task::spawn_blocking(move || read_local_database(client_type, osu_path))
                .await
                .map_err(|e| format!("Task panicked: {e}"))
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

    let generation = app.updates.scan.scan_generation;
    let Some(path) = failed_maps::failed_maps_path() else {
        app.toast_info("no known bad maps to recheck");
        return;
    };
    let ids: Vec<u32> = failed_maps::load(&path).beatmapset_ids;
    if ids.is_empty() {
        app.toast_info("no known bad maps to recheck");
        return;
    }

    app.updates.scan.scan_status = ScanStatus::CheckingFailedMaps;
    set_loading_message(
        &mut app.updates.message,
        format!("rechecking known bad maps 0/{}...", ids.len()),
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
        .updates
        .scan
        .local_beatmapsets
        .iter()
        .map(|bs| bs.id)
        .collect();
    let all_local_checksums = std::mem::take(&mut app.updates.scan.all_local_checksums);
    let local_collections_raw = app.updates.scan.local_collections_raw.clone();
    let generation = app.updates.scan.scan_generation;
    let client_type = app.library.client_type;
    let current_snapshots = snapshots::current_snapshots(
        client_type,
        &app.updates.scan.local_collections_raw,
        app.updates.scan.local_beatmapsets.iter(),
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
    let hidden_failed_count = failed_beatmapset_ids.len();
    // Drop any manually-ignored id that is now genuinely installed, then hide
    // the rest from this scan.
    let ignored_beatmapset_ids = ignored_maps::ignored_maps_path()
        .map(|path| ignored_maps::reconcile_installed(&path, &local_set_ids))
        .unwrap_or_default();

    app.updates.scan.scan_status = ScanStatus::FetchingCollection;

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
                    manually_added_count: added_count,
                    hidden_failed_count,
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

#[derive(Debug, Clone)]
struct CollectionBeatmapset {
    id: u32,
    checksums: Vec<Md5>,
}

impl CollectionBeatmapset {
    fn is_in_snapshot(
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
    let mut candidates_to_check: Vec<(CollectionBeatmapset, u32, String)> = Vec::new();
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

    for fetch_result in fetched {
        let (collection_id, collection) = fetch_result?;

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

        // Dedupe within this collection only — the same beatmapset can appear
        // in multiple collections and must be tracked per collection_id.
        let mut seen_in_collection: HashSet<u32> = HashSet::new();

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

            if !api_checksums.is_empty()
                && api_checksums.iter().all(|cs| local_checksums.contains(cs))
            {
                trace!(
                    beatmapset_id = beatmapset.id,
                    "ID not found but all checksums exist locally, skipping"
                );
                continue;
            }

            if should_hide_failed_beatmapset(&settings, beatmapset.id) {
                trace!(beatmapset_id = beatmapset.id, "skipping failed beatmapset");
                continue;
            }

            trace!(
                beatmapset_id = beatmapset.id,
                "not installed, adding to candidates"
            );
            candidates_to_check.push((
                CollectionBeatmapset {
                    id: beatmapset.id,
                    checksums: beatmapset
                        .beatmaps
                        .iter()
                        .filter_map(|beatmap| checksum::parse_hex(&beatmap.checksum))
                        .collect(),
                },
                collection_id,
                collection.name.to_string(),
            ));
        }
    }

    debug!(
        candidates = candidates_to_check.len(),
        "finished scanning collections"
    );

    let mut all_missing: Vec<MissingBeatmapset> = Vec::new();

    for (beatmapset, collection_id, collection_name) in candidates_to_check {
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

        all_missing.push(MissingBeatmapset {
            id: beatmapset.id,
            status: MissingStatus::NotInstalled,
            collection_id,
            collection_name,
            selected: !previously_deleted,
            previously_deleted,
        });
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
    })
}
