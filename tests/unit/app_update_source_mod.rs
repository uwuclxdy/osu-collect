use super::{
    BeatmapSort, CollectionSort, MissingBeatmapset, MissingStatus, ScanCta, ScanStatus,
    UpdateSource, scroll_list,
};
use crate::app::EnrichSink;
use crate::osu_db::LocalCollection;
use osu_downloader::search::BeatmapSetMeta;
use std::collections::HashMap;

fn local_col(name: &str, count: usize) -> LocalCollection {
    LocalCollection {
        name: name.to_string(),
        beatmap_checksums: vec![Default::default(); count].into_boxed_slice(),
    }
}

fn missing(id: u32, collection_id: u32, previously_deleted: bool) -> MissingBeatmapset {
    MissingBeatmapset {
        id,
        status: MissingStatus::NotInstalled,
        collection_id,
        collection_name: format!("col {collection_id}"),
        selected: true,
        previously_deleted,
        enrich_diff_id: None,
    }
}

fn missing_with_diff(id: u32, collection_id: u32, diff: Option<u32>) -> MissingBeatmapset {
    MissingBeatmapset {
        enrich_diff_id: diff,
        ..missing(id, collection_id, false)
    }
}

fn meta_val(id: u32, title: &str) -> BeatmapSetMeta {
    BeatmapSetMeta {
        id,
        title: title.to_string(),
        artist: "artist".to_string(),
        creator: "mapper".to_string(),
        status: "ranked".to_string(),
        favourite_count: 0,
        play_count: 0,
        nsfw: false,
        video: false,
    }
}

#[test]
fn enrichment_seeds_one_deduped_diff_per_missing_set() {
    let mut tab = UpdateSource::new();
    tab.set_missing_beatmaps(vec![
        missing_with_diff(10, 100, Some(1000)),
        missing_with_diff(10, 200, Some(1000)), // same set, other collection → deduped
        missing_with_diff(20, 100, Some(2000)),
        missing_with_diff(30, 100, None), // no diff id → contributes nothing
    ]);
    tab.descend(); // seeds the enrichment pager off the missing sets

    let page = tab
        .next_enrich_page()
        .expect("a page of diff ids to enrich");
    assert_eq!(
        page,
        vec![1000, 2000],
        "one diff per unique set, in set order, holes skipped"
    );
    assert!(
        tab.next_enrich_page().is_none(),
        "the pager dries up after the seeded ids"
    );
}

#[test]
fn fold_meta_backfills_the_missing_set_cache() {
    let mut tab = UpdateSource::new();
    tab.set_missing_beatmaps(vec![missing_with_diff(10, 100, Some(1000))]);
    tab.descend();
    assert!(
        tab.set_meta(10).is_none(),
        "no metadata before a page lands"
    );

    let mut folded = HashMap::new();
    folded.insert(10, meta_val(10, "Song A"));
    folded.insert(99, meta_val(99, "unlisted")); // a set not previewed is harmless
    tab.fold_meta(folded);
    assert_eq!(tab.set_meta(10).map(|m| m.title.as_str()), Some("Song A"));

    // A re-descend reseeds and clears the cache (a fresh scan is a new identity).
    tab.descend();
    assert!(
        tab.set_meta(10).is_none(),
        "re-descend clears stale metadata"
    );
}

/// Two collections (ids 100 / 200) with three missing sets: two in 100, one in 200.
fn seeded() -> UpdateSource {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        local_col("Alpha - 100", 3),
        local_col("Beta - 200", 1),
    ]);
    tab.set_missing_beatmaps(vec![
        missing(1, 100, false),
        missing(2, 100, false),
        missing(3, 200, false),
    ]);
    tab
}

#[test]
fn needs_initial_scan_reflects_cache_state() {
    let mut tab = UpdateSource::new();
    assert!(tab.needs_initial_scan(), "idle tab needs a scan");

    tab.scan.scan_status = ScanStatus::ReadingDatabase;
    assert!(
        !tab.needs_initial_scan(),
        "in-flight scan should not restart"
    );

    tab.scan.scan_status = ScanStatus::Ready;
    assert!(!tab.needs_initial_scan(), "cached results should be reused");

    tab.scan.scan_status = ScanStatus::Error;
    assert!(tab.needs_initial_scan(), "errored scans retry");
}

#[test]
fn scroll_list_wraps_at_both_ends() {
    let mut state = Some(0);
    scroll_list(&mut state, 3, -1);
    assert_eq!(state, Some(2));
    scroll_list(&mut state, 3, 1);
    assert_eq!(state, Some(0));
    scroll_list(&mut state, 3, 10);
    assert_eq!(state, Some(1));
}

#[test]
fn scroll_list_empty_leaves_state() {
    let mut state: Option<usize> = None;
    scroll_list(&mut state, 0, 1);
    assert_eq!(state, None);
}

// ── sorts ─────────────────────────────────────────────────────────────────────

#[test]
fn collection_sort_cycles_through_all_modes() {
    let sort = CollectionSort::Default.next();
    assert_eq!(sort, CollectionSort::Name);
    assert_eq!(sort.next(), CollectionSort::Size);
    assert_eq!(sort.next().next(), CollectionSort::Default);
}

#[test]
fn beatmap_sort_cycles_through_all_modes() {
    let sort = BeatmapSort::Default.next();
    assert_eq!(sort, BeatmapSort::Name);
    assert_eq!(sort.next(), BeatmapSort::Status);
    assert_eq!(sort.next().next(), BeatmapSort::Default);
}

#[test]
fn collection_sort_name_orders_case_insensitively() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        local_col("Zebra Maps - 11111", 5),
        local_col("alpha Maps - 22222", 2),
        local_col("Beta Maps - 33333", 8),
    ]);
    tab.cycle_collection_sort(); // Default → Name
    let names: Vec<&str> = tab
        .selection
        .local_collections
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "alpha Maps - 22222",
            "Beta Maps - 33333",
            "Zebra Maps - 11111"
        ]
    );
}

#[test]
fn collection_sort_size_orders_largest_first() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        local_col("Small - 11111", 2),
        local_col("Large - 22222", 10),
        local_col("Medium - 33333", 5),
    ]);
    tab.cycle_collection_sort();
    tab.cycle_collection_sort(); // → Size
    let counts: Vec<usize> = tab
        .selection
        .local_collections
        .iter()
        .map(|c| c.beatmap_count)
        .collect();
    assert_eq!(counts, [10, 5, 2]);
}

#[test]
fn collection_sort_default_restores_insertion_order() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        local_col("Zebra Maps - 11111", 5),
        local_col("Alpha Maps - 22222", 2),
        local_col("Beta Maps - 33333", 8),
    ]);
    let original: Vec<String> = tab
        .selection
        .local_collections
        .iter()
        .map(|c| c.name.clone())
        .collect();
    tab.cycle_collection_sort();
    tab.cycle_collection_sort();
    tab.cycle_collection_sort(); // back to Default
    let restored: Vec<String> = tab
        .selection
        .local_collections
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(original, restored);
}

// ── removed count ─────────────────────────────────────────────────────────────

#[test]
fn set_removed_counts_applies_to_matching_collection() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![local_col("Pack - 11111", 3)]);
    let mut counts = HashMap::new();
    counts.insert(11111u32, 7usize);
    tab.set_removed_counts(&counts);
    assert_eq!(tab.selection.local_collections[0].removed_count, 7);
}

#[test]
fn set_removed_counts_leaves_unmatched_at_zero() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        local_col("Alpha - 11111", 2),
        local_col("Beta - 22222", 4),
    ]);
    let mut counts = HashMap::new();
    counts.insert(11111u32, 3usize);
    tab.set_removed_counts(&counts);
    let removed: Vec<usize> = tab
        .selection
        .local_collections
        .iter()
        .map(|e| e.removed_count)
        .collect();
    assert_eq!(removed, [3, 0]);
}

#[test]
fn set_removed_counts_survives_sort_round_trip() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        local_col("Alpha - 11111", 2),
        local_col("Beta - 22222", 4),
    ]);
    let mut counts = HashMap::new();
    counts.insert(11111u32, 5usize);
    tab.set_removed_counts(&counts);
    tab.cycle_collection_sort();
    tab.cycle_collection_sort();
    tab.cycle_collection_sort(); // back to Default from the snapshot
    let entry = tab
        .selection
        .local_collections
        .iter()
        .find(|e| e.collection_id == Some(11111))
        .expect("entry for 11111 must exist");
    assert_eq!(entry.removed_count, 5);
}

#[test]
fn selection_survives_sort_round_trip() {
    let mut tab = seeded();
    // Deselect the collection under the cursor (whole-collection selection is
    // the sole download determinant now).
    tab.toggle_selected_collection();
    let deselected_id = tab.selection.local_collections[0].collection_id;
    assert!(!tab.selection.local_collections[0].selected);

    // Cycle Default → Name → Size → Default.
    tab.cycle_collection_sort();
    tab.cycle_collection_sort();
    tab.cycle_collection_sort();

    let entry = tab
        .selection
        .local_collections
        .iter()
        .find(|e| e.collection_id == deselected_id)
        .expect("the deselected collection must still exist after a sort round-trip");
    assert!(
        !entry.selected,
        "returning to Default sort must not re-select a deselected collection"
    );
}

// ── descend / ascend / pane focus ─────────────────────────────────────────────

#[test]
fn descend_homes_cursors_and_focuses_list() {
    let mut tab = seeded();
    assert!(!tab.is_browsing());
    tab.descend();
    assert!(tab.is_browsing());
    assert!(!tab.preview_focused());
    assert_eq!(tab.selection.collections_cursor, Some(0));
    assert_eq!(tab.selection.preview_cursor, Some(0));
}

#[test]
fn ascend_steps_preview_then_form() {
    let mut tab = seeded();
    tab.descend();
    tab.focus_preview(); // Alpha (100) has 2 missing → preview focuses
    assert!(tab.preview_focused());
    assert!(tab.ascend(), "preview → list consumes a step");
    assert!(!tab.preview_focused());
    assert!(tab.is_browsing());
    assert!(tab.ascend(), "list → form consumes a step");
    assert!(!tab.is_browsing());
    assert!(!tab.ascend(), "nothing left to ascend");
}

#[test]
fn focus_preview_noop_without_missing() {
    // No collections → nothing highlighted → nothing to preview.
    let mut tab = UpdateSource::new();
    tab.descend();
    tab.focus_preview();
    assert!(
        !tab.preview_focused(),
        "focus_preview is a no-op when there's no highlighted collection to preview"
    );
}

// ── whole-collection selection ────────────────────────────────────────────────

#[test]
fn selected_beatmapset_ids_covers_selected_collections_only() {
    let tab = seeded();
    let mut ids = tab.selected_beatmapset_ids();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3], "all selected by default");
    assert_eq!(tab.selected_new_count(), 3);
}

#[test]
fn deselecting_a_collection_drops_its_maps() {
    let mut tab = seeded();
    // Deselect collection 100 (index 0 after default sort).
    tab.selection.collections_cursor = Some(0);
    tab.toggle_selected_collection();
    assert!(!tab.selection.local_collections[0].selected);
    let ids = tab.selected_beatmapset_ids();
    assert_eq!(ids, vec![3], "only collection 200's map survives");
    assert_eq!(tab.selected_new_count(), 1);
}

#[test]
fn set_all_collections_selected_toggles_everything() {
    let mut tab = seeded();
    tab.set_all_collections_selected(false);
    assert!(tab.selected_beatmapset_ids().is_empty());
    tab.set_all_collections_selected(true);
    let mut ids = tab.selected_beatmapset_ids();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3]);
}

// ── no-update collections are inert (unselectable) + sunk to the bottom ────────

/// Three collections; only 100 and 300 have updates. After the scan, 200 sinks
/// below the rest and is force-deselected.
fn seeded_with_empty() -> UpdateSource {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        local_col("Has - 100", 3),
        local_col("Empty - 200", 2),
        local_col("Also - 300", 1),
    ]);
    tab.set_missing_beatmaps(vec![missing(1, 100, false), missing(2, 300, false)]);
    tab
}

#[test]
fn no_update_collections_sink_to_bottom() {
    let tab = seeded_with_empty();
    let ids: Vec<Option<u64>> = tab
        .selection
        .local_collections
        .iter()
        .map(|c| c.collection_id)
        .collect();
    // 100 and 300 (with updates) keep their insertion order on top; 200 sinks.
    assert_eq!(ids, vec![Some(100), Some(300), Some(200)]);
}

#[test]
fn no_update_collection_is_force_deselected() {
    let tab = seeded_with_empty();
    let empty = tab
        .selection
        .local_collections
        .iter()
        .find(|c| c.collection_id == Some(200))
        .expect("the empty collection is still listed");
    assert!(
        !empty.selected,
        "a no-update collection is force-deselected"
    );
    assert!(
        tab.selection
            .local_collections
            .iter()
            .filter(|c| c.collection_id != Some(200))
            .all(|c| c.selected),
        "collections with updates stay selected"
    );
}

#[test]
fn toggle_is_inert_on_no_update_collection() {
    let mut tab = seeded_with_empty();
    let empty_idx = tab
        .selection
        .local_collections
        .iter()
        .position(|c| c.collection_id == Some(200))
        .expect("the empty collection is still listed");
    tab.selection.collections_cursor = Some(empty_idx);
    tab.toggle_selected_collection();
    assert!(
        !tab.selection.local_collections[empty_idx].selected,
        "toggling a no-update collection must be a no-op"
    );
}

#[test]
fn select_all_skips_no_update_collections() {
    let mut tab = seeded_with_empty();
    tab.set_all_collections_selected(false);
    tab.set_all_collections_selected(true);
    let selected: Vec<Option<u64>> = tab
        .selection
        .local_collections
        .iter()
        .filter(|c| c.selected)
        .map(|c| c.collection_id)
        .collect();
    assert_eq!(
        selected,
        vec![Some(100), Some(300)],
        "select-all ticks only collections with updates"
    );
}

#[test]
fn no_update_collections_stay_sunk_across_sorts() {
    let mut tab = seeded_with_empty();
    tab.cycle_collection_sort(); // → Name
    let last = tab
        .selection
        .local_collections
        .last()
        .expect("list is non-empty");
    assert_eq!(
        last.collection_id,
        Some(200),
        "a no-update collection stays at the bottom under the Name sort too"
    );
}

#[test]
fn counts_track_missing_per_collection() {
    let tab = seeded();
    assert_eq!(tab.total_new_count(), 3);
    assert_eq!(tab.new_count_for(100), 2);
    assert_eq!(tab.new_count_for(200), 1);
    assert_eq!(tab.new_count_for(999), 0);
    assert_eq!(tab.collections_with_new_count(), 2);
}

// ── preview derivation ────────────────────────────────────────────────────────

#[test]
fn preview_shows_only_highlighted_collection() {
    let mut tab = seeded();
    tab.descend();
    tab.selection.collections_cursor = Some(0); // Alpha (100)
    let indices = tab.preview_missing_indices();
    let ids: Vec<u32> = indices
        .iter()
        .map(|&i| tab.selection.cached_missing_sets[i].id)
        .collect();
    assert_eq!(
        ids,
        vec![1, 2],
        "preview lists collection 100's missing sets"
    );
    assert_eq!(tab.preview_len(), 2);

    tab.selection.collections_cursor = Some(1); // Beta (200)
    assert_eq!(tab.preview_len(), 1);
}

#[test]
fn mark_installed_ids_read_the_preview() {
    let mut tab = seeded();
    tab.descend();
    tab.selection.collections_cursor = Some(0);
    tab.selection.preview_cursor = Some(1); // second row of collection 100
    assert_eq!(tab.preview_focused_id(), vec![2]);
    let mut all = tab.highlighted_collection_missing_ids();
    all.sort_unstable();
    assert_eq!(all, vec![1, 2]);
}

#[test]
fn hide_missing_drops_ids_from_cache() {
    let mut tab = seeded();
    tab.hide_missing(&std::collections::HashSet::from([1, 3]));
    assert_eq!(tab.total_new_count(), 1);
    assert_eq!(tab.new_count_for(100), 1);
    assert_eq!(tab.new_count_for(200), 0);
}

// ── scan CTA state machine ────────────────────────────────────────────────────

#[test]
fn scan_cta_label_follows_state_machine() {
    let mut tab = UpdateSource::new();
    // Idle: first scan.
    assert_eq!(tab.scan_cta(), ScanCta::Scan);
    assert_eq!(tab.scan_cta_label(), "scan for updates");

    // In flight: inert.
    tab.scan.scan_status = ScanStatus::FetchingCollection;
    assert_eq!(tab.scan_cta(), ScanCta::Busy);
    assert_eq!(tab.scan_cta_label(), "scanning…");

    // Ready with nothing found: re-scan invitation.
    tab.scan.scan_status = ScanStatus::Ready;
    assert_eq!(tab.scan_cta(), ScanCta::Scan);
    assert_eq!(tab.scan_cta_label(), "rescan");

    // Ready with updates: the scan button still only scans (opening the browse
    // is the separate view button's job), so it reads `rescan`.
    let mut ready = seeded();
    ready.scan.scan_status = ScanStatus::Ready;
    assert_eq!(ready.scan_cta(), ScanCta::Scan);
    assert_eq!(ready.scan_cta_label(), "rescan");
}

#[test]
fn client_switch_reset_clears_scan_without_auto_scanning() {
    let mut tab = seeded();
    tab.descend();
    tab.scan.scan_status = ScanStatus::Ready;
    tab.reset_for_client_switch();
    assert!(tab.selection.local_collections.is_empty());
    assert!(tab.selection.cached_missing_sets.is_empty());
    assert!(!tab.is_browsing());
    // Cleared back to Idle — the user scans manually; no auto-scan on switch.
    assert_eq!(tab.scan.scan_status, ScanStatus::Idle);
}
