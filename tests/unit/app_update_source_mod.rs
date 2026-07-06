use super::{
    BeatmapSort, CollectionSort, MissingBeatmapset, MissingStatus, ScanCta, ScanStatus,
    UpdateSource, scroll_list,
};
use crate::osu_db::LocalCollection;
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
    }
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

    // Ready with updates: descend.
    let mut ready = seeded();
    ready.scan.scan_status = ScanStatus::Ready;
    assert_eq!(ready.scan_cta(), ScanCta::Descend);
    assert_eq!(ready.scan_cta_label(), "view 3 updates");
}

#[test]
fn client_switch_reset_clears_scan() {
    let mut tab = seeded();
    tab.descend();
    tab.scan.scan_status = ScanStatus::Ready;
    let action = tab.reset_for_client_switch();
    assert_eq!(action, super::UpdateAction::RefreshAll);
    assert!(tab.selection.local_collections.is_empty());
    assert!(tab.selection.cached_missing_sets.is_empty());
    assert!(!tab.is_browsing());
    assert_eq!(tab.scan.scan_status, ScanStatus::Idle);
}
