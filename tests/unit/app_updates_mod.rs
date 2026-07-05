use super::{
    BeatmapSort, CollectionSort, MissingBeatmapset, MissingStatus, ScanStatus, UpdateSource,
    scroll_list,
};
use crate::osu_db::{LocalCollection, checksum};
use std::collections::HashMap;

#[test]
fn needs_initial_scan_reflects_cache_state() {
    let mut tab = UpdateSource::new();
    assert!(tab.needs_initial_scan(), "idle tab needs a scan");

    tab.scan.scan_status = ScanStatus::ReadingDatabase;
    assert!(
        !tab.needs_initial_scan(),
        "in-flight scan should not restart"
    );

    tab.scan.scan_status = ScanStatus::FetchingCollection;
    assert!(!tab.needs_initial_scan());

    tab.scan.scan_status = ScanStatus::Ready;
    assert!(!tab.needs_initial_scan(), "cached results should be reused");

    tab.scan.scan_status = ScanStatus::Error;
    assert!(tab.needs_initial_scan(), "errored scans retry on tab entry");
}

#[test]
fn scroll_list_wraps_at_both_ends() {
    // Up past the first item wraps to the last.
    let mut state = Some(0);
    scroll_list(&mut state, 3, -1);
    assert_eq!(state, Some(2));
    // Down past the last item wraps to the first.
    scroll_list(&mut state, 3, 1);
    assert_eq!(state, Some(0));
    // A larger step wraps modulo the length.
    scroll_list(&mut state, 3, 10);
    assert_eq!(state, Some(1));
}

#[test]
fn scroll_list_empty_leaves_state() {
    let mut state: Option<usize> = None;
    scroll_list(&mut state, 0, 1);
    assert_eq!(state, None);
}

#[test]
fn scroll_list_none_starts_at_zero() {
    let mut state: Option<usize> = None;
    scroll_list(&mut state, 5, 1);
    assert_eq!(state, Some(1));
}

fn local_col(name: &str, count: usize) -> LocalCollection {
    LocalCollection {
        name: name.to_string(),
        beatmap_checksums: vec![Default::default(); count].into_boxed_slice(),
    }
}

fn md5(seed: u8) -> crate::osu_db::Md5 {
    let mut out = [0u8; 16];
    out[0] = seed;
    out
}

fn local_col_with_checksums(name: &str, checksums: &[crate::osu_db::Md5]) -> LocalCollection {
    LocalCollection {
        name: name.to_string(),
        beatmap_checksums: checksums.to_vec().into_boxed_slice(),
    }
}

fn missing_beatmap(
    id: u32,
    collection_id: u32,
    collection_name: &str,
    previously_deleted: bool,
) -> MissingBeatmapset {
    MissingBeatmapset {
        id,
        status: MissingStatus::NotInstalled,
        collection_id,
        collection_name: collection_name.to_string(),
        selected: true,
        previously_deleted,
    }
}

#[test]
fn collection_sort_cycles_through_all_modes() {
    let sort = CollectionSort::Default;
    let sort = sort.next();
    assert_eq!(sort, CollectionSort::Name);
    let sort = sort.next();
    assert_eq!(sort, CollectionSort::Size);
    let sort = sort.next();
    assert_eq!(sort, CollectionSort::Default);
}

#[test]
fn beatmap_sort_cycles_through_all_modes() {
    let sort = BeatmapSort::Default;
    let sort = sort.next();
    assert_eq!(sort, BeatmapSort::Name);
    let sort = sort.next();
    assert_eq!(sort, BeatmapSort::Status);
    let sort = sort.next();
    assert_eq!(sort, BeatmapSort::Default);
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
    tab.cycle_collection_sort(); // Default → Name
    tab.cycle_collection_sort(); // Name → Size
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
    let original_names: Vec<String> = tab
        .selection
        .local_collections
        .iter()
        .map(|c| c.name.clone())
        .collect();
    tab.cycle_collection_sort(); // Default → Name
    tab.cycle_collection_sort(); // Name → Size
    tab.cycle_collection_sort(); // Size → Default
    let restored_names: Vec<String> = tab
        .selection
        .local_collections
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(
        original_names, restored_names,
        "cycling back to Default restores insertion order"
    );
}

#[test]
fn beatmap_sort_name_orders_by_collection_name() {
    let mut tab = UpdateSource::new();
    // collection_id 11111 must match the ID extracted from the collection name
    tab.set_collections(vec![local_col("Maps - 11111", 3)]);
    tab.set_missing_beatmaps(vec![
        missing_beatmap(1, 11111, "Zebra pack", false),
        missing_beatmap(2, 11111, "Alpha pack", false),
        missing_beatmap(3, 11111, "Beta pack", false),
    ]);
    tab.filter_cached();
    tab.cycle_beatmap_sort(); // Default → Name
    let names: Vec<&str> = tab
        .selection
        .visible_missing
        .iter()
        .map(|&idx| {
            tab.selection.cached_missing_sets[idx]
                .collection_name
                .as_str()
        })
        .collect();
    assert_eq!(names, ["Alpha pack", "Beta pack", "Zebra pack"]);
}

#[test]
fn beatmap_sort_status_puts_previously_deleted_last() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![local_col("Collection - 11111", 4)]);
    tab.set_missing_beatmaps(vec![
        missing_beatmap(1, 11111, "col", true),
        missing_beatmap(2, 11111, "col", false),
        missing_beatmap(3, 11111, "col", true),
        missing_beatmap(4, 11111, "col", false),
    ]);
    tab.filter_cached();
    tab.cycle_beatmap_sort(); // Default → Name
    tab.cycle_beatmap_sort(); // Name → Status
    let deleted_flags: Vec<bool> = tab
        .selection
        .visible_missing
        .iter()
        .map(|&idx| tab.selection.cached_missing_sets[idx].previously_deleted)
        .collect();
    // previously_deleted=false entries first, then true
    assert!(
        deleted_flags.iter().take_while(|&&d| !d).count() >= 2,
        "non-deleted entries should appear before deleted ones"
    );
    assert!(
        deleted_flags.iter().rev().take_while(|&&d| d).count() >= 2,
        "deleted entries should appear at the end"
    );
}

#[test]
fn beatmap_sort_default_restores_filter_order() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![local_col("Collection - 11111", 3)]);
    tab.set_missing_beatmaps(vec![
        missing_beatmap(10, 11111, "Zebra", false),
        missing_beatmap(20, 11111, "Alpha", false),
        missing_beatmap(30, 11111, "Beta", false),
    ]);
    tab.filter_cached();
    let original: Vec<usize> = tab.selection.visible_missing.clone();
    tab.cycle_beatmap_sort(); // Default → Name (reorders)
    tab.cycle_beatmap_sort(); // Name → Status
    tab.cycle_beatmap_sort(); // Status → Default (restores)
    assert_eq!(
        tab.selection.visible_missing, original,
        "cycling back to Default restores filter order"
    );
}

#[test]
fn s_key_cycles_collection_sort_in_list() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![local_col("Test - 11111", 1)]);
    tab.selection.in_collection_list = true;
    assert_eq!(tab.selection.collection_sort, CollectionSort::Default);
    tab.handle_char('s');
    assert_eq!(tab.selection.collection_sort, CollectionSort::Name);
    tab.handle_char('s');
    assert_eq!(tab.selection.collection_sort, CollectionSort::Size);
    tab.handle_char('s');
    assert_eq!(tab.selection.collection_sort, CollectionSort::Default);
}

#[test]
fn s_key_does_not_cycle_sort_outside_list() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![local_col("Test - 11111", 1)]);
    // Not in any list — `handle_char` only drives list shortcuts, so 's' outside
    // a list is inert here (path typing routes through the app-global library).
    assert_eq!(tab.selection.collection_sort, CollectionSort::Default);
    tab.handle_char('s');
    assert_eq!(
        tab.selection.collection_sort,
        CollectionSort::Default,
        "'s' outside list must not advance sort"
    );
}

// --- removed count ---

#[test]
fn set_removed_counts_applies_to_matching_collection() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![local_col("Pack - 11111", 3)]);

    let mut counts = HashMap::new();
    counts.insert(11111u32, 7usize);
    tab.set_removed_counts(&counts);

    let entry = &tab.selection.local_collections[0];
    assert_eq!(entry.removed_count, 7);
}

#[test]
fn set_removed_counts_leaves_unmatched_at_zero() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        local_col("Alpha - 11111", 2),
        local_col("Beta - 22222", 4),
    ]);

    // Only set a count for 11111; 22222 gets nothing.
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
fn set_removed_counts_also_updates_default_order_snapshot() {
    // The default-order snapshot must be kept in sync so that cycling back to
    // Default sort restores the correct removed_count values.
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        local_col("Alpha - 11111", 2),
        local_col("Beta - 22222", 4),
    ]);

    let mut counts = HashMap::new();
    counts.insert(11111u32, 5usize);
    tab.set_removed_counts(&counts);

    // Cycle sort away from Default and back; the snapshot must carry the count.
    tab.cycle_collection_sort(); // Default → Name
    tab.cycle_collection_sort(); // Name → Size
    tab.cycle_collection_sort(); // Size → Default (restores from snapshot)

    let entry = tab
        .selection
        .local_collections
        .iter()
        .find(|e| e.collection_id == Some(11111))
        .expect("entry for 11111 must exist");
    assert_eq!(
        entry.removed_count, 5,
        "removed_count must survive sort round-trip"
    );
}

#[test]
fn removed_count_is_zero_when_no_counts_provided() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![local_col("Pack - 33333", 5)]);

    // Apply an empty map — no collection gets a removed_count.
    tab.set_removed_counts(&HashMap::new());

    let entry = &tab.selection.local_collections[0];
    assert_eq!(entry.removed_count, 0, "no counts → removed_count stays 0");
}

// --- checksum set diff correctness ---

#[test]
fn removed_count_reflects_local_minus_upstream_checksums() {
    // Simulate what fetch_missing_beatmapsets computes for removed_count:
    // local checksums for the collection that are absent from the upstream set.
    use std::collections::HashSet;

    let local = [md5(1), md5(2), md5(3), md5(4)];
    let upstream: HashSet<_> = [checksum::to_hex(md5(1)), checksum::to_hex(md5(3))]
        .iter()
        .filter_map(|h| checksum::parse_hex(h))
        .filter(|cs| !checksum::is_empty(cs))
        .collect();

    let local_set: HashSet<_> = local
        .iter()
        .copied()
        .filter(|cs| !checksum::is_empty(cs))
        .collect();

    let removed = local_set.difference(&upstream).count();
    // md5(2) and md5(4) are local but not upstream → removed = 2
    assert_eq!(removed, 2);
}

#[test]
fn removed_count_is_zero_when_all_local_checksums_present_upstream() {
    use std::collections::HashSet;

    let local = [md5(10), md5(20)];
    let upstream: HashSet<_> = [
        checksum::to_hex(md5(10)),
        checksum::to_hex(md5(20)),
        checksum::to_hex(md5(30)), // extra upstream, not in local
    ]
    .iter()
    .filter_map(|h| checksum::parse_hex(h))
    .filter(|cs| !checksum::is_empty(cs))
    .collect();

    let local_set: HashSet<_> = local
        .iter()
        .copied()
        .filter(|cs| !checksum::is_empty(cs))
        .collect();

    assert_eq!(local_set.difference(&upstream).count(), 0);
}

#[test]
fn set_removed_counts_applied_to_local_col_with_checksums() {
    // End-to-end: build a tab with a real checksum collection, apply counts,
    // and verify the entry reflects the expected removed_count.
    let checksums = [md5(0xaa), md5(0xbb), md5(0xcc)];
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![local_col_with_checksums("Songs - 55555", &checksums)]);

    let mut counts = HashMap::new();
    counts.insert(55555u32, 2usize); // 2 of the 3 local checksums are absent upstream
    tab.set_removed_counts(&counts);

    let entry = tab
        .selection
        .local_collections
        .iter()
        .find(|e| e.collection_id == Some(55555))
        .expect("entry must exist");
    assert_eq!(entry.removed_count, 2);
}
