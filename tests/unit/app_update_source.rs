use crate::app::{
    runtime::{FetchCompareSettings, collection_ids_for_scan, should_hide_failed_beatmapset},
    update_source::{MissingBeatmapset, MissingStatus, UpdateSource},
};
use crate::osu_db::{LocalBeatmap, LocalBeatmapset, LocalCollection, Md5};
use std::collections::{HashMap, HashSet};

fn test_md5(seed: u8) -> Md5 {
    let mut out = [0u8; 16];
    out[0] = seed;
    out
}

/// Mirrors what `fetch_missing_beatmapsets` writes: a previously-deleted set is
/// constructed already held back. A fixture with `included: true` on a deleted
/// set is a state the scan cannot produce.
fn missing(id: u32, collection_id: u32, previously_deleted: bool) -> MissingBeatmapset {
    MissingBeatmapset {
        id,
        status: MissingStatus::NotInstalled,
        collection_id,
        collection_name: format!("col {collection_id}"),
        included: !previously_deleted,
        previously_deleted,
        checksums: Box::new([]),
        enrich_diff_id: None,
    }
}

#[test]
fn set_collections_hides_entries_without_ids() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        LocalCollection {
            name: "My Collection - 123".to_string(),
            beatmap_checksums: [test_md5(1)].into(),
        },
        LocalCollection {
            name: "Missing Id".to_string(),
            beatmap_checksums: [test_md5(2)].into(),
        },
    ]);

    assert_eq!(tab.selection.local_collections.len(), 1);
    assert_eq!(tab.selection.local_collections[0].collection_id, Some(123));
    assert!(
        tab.selection.local_collections[0].selected,
        "a fresh scan selects every collection by default"
    );
}

#[test]
fn extract_id_formats() {
    let cases = [
        ("Cool Maps - 9001", Some(9001u64)),
        ("Cool Maps – 9001", Some(9001)),
        ("Cool Maps — 9001", Some(9001)),
        ("#9001 - Cool Maps", Some(9001)),
        ("Cool Maps (9001)", Some(9001)),
        ("Cool Maps [9001]", Some(9001)),
        ("No id here", None),
        ("Short - 1", None),
    ];

    let mut tab = UpdateSource::new();
    for (name, expected_id) in &cases {
        tab.set_collections(vec![LocalCollection {
            name: name.to_string(),
            beatmap_checksums: Box::new([]),
        }]);
        let got = tab
            .selection
            .local_collections
            .first()
            .and_then(|e| e.collection_id);
        assert_eq!(got, *expected_id, "name: {name}");
    }
}

#[test]
fn collection_ids_for_scan_uses_selected_ids_only() {
    assert_eq!(collection_ids_for_scan(vec![1, 3]), vec![1, 3]);
}

#[test]
fn collection_ids_for_scan_skips_ids_outside_u32() {
    assert_eq!(
        collection_ids_for_scan(vec![42, u64::from(u32::MAX) + 1]),
        vec![42]
    );
}

#[test]
fn set_local_beatmapsets_stores_sets() {
    let mut tab = UpdateSource::new();
    tab.set_local_beatmapsets(vec![
        LocalBeatmapset {
            id: 10,
            beatmaps: [LocalBeatmap {
                checksum: test_md5(0xaa),
            }]
            .into(),
        },
        LocalBeatmapset {
            id: 20,
            beatmaps: [LocalBeatmap {
                checksum: test_md5(0xbb),
            }]
            .into(),
        },
    ]);

    let ids: Vec<u32> = tab.scan.local_beatmapsets.iter().map(|bs| bs.id).collect();
    assert!(ids.contains(&10));
    assert!(ids.contains(&20));
    assert!(!ids.contains(&99));
}

#[test]
fn set_all_checksums_builds_hashset() {
    let abc = test_md5(0xab);
    let def = test_md5(0xde);
    let mut tab = UpdateSource::new();
    tab.set_all_checksums(vec![abc, def]);

    assert!(tab.scan.all_local_checksums.contains(&abc));
    assert!(tab.scan.all_local_checksums.contains(&def));
    assert!(!tab.scan.all_local_checksums.contains(&test_md5(0xff)));
}

#[test]
fn installed_beatmapset_not_in_missing() {
    let cksum = test_md5(0xd0);
    let mut tab = UpdateSource::new();
    tab.set_local_beatmapsets(vec![LocalBeatmapset {
        id: 42,
        beatmaps: [LocalBeatmap { checksum: cksum }].into(),
    }]);
    tab.set_all_checksums(vec![cksum]);
    tab.set_missing_beatmaps(vec![], &HashMap::new());

    assert_eq!(tab.total_new_count(), 0);
}

#[test]
fn selected_beatmapset_ids_returns_only_selected_collections() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        LocalCollection {
            name: "Alpha - 100".to_string(),
            beatmap_checksums: Box::new([]),
        },
        LocalCollection {
            name: "Beta - 200".to_string(),
            beatmap_checksums: Box::new([]),
        },
    ]);
    tab.set_missing_beatmaps(
        vec![
            missing(1, 100, false),
            missing(2, 100, false),
            missing(3, 200, false),
        ],
        &HashMap::new(),
    );

    let mut all = tab.selected_beatmapset_ids();
    all.sort_unstable();
    assert_eq!(all, vec![1, 2, 3], "every collection selected by default");

    // Deselect the first collection (100): its two maps drop out.
    tab.selection.local_collections[0].selected = false;
    assert_eq!(tab.selected_beatmapset_ids(), vec![3]);
    assert_eq!(tab.selected_collection_ids(), vec![200]);
}

/// A beatmapset missing from two checked collections is one `MissingBeatmapset`
/// row per collection it's missing from — the run dedupes that down to one id
/// before it ever reaches precheck (`DownloadSession::prepare`,
/// `src/download/session.rs`). The button label and the queued-toast count must
/// agree with that same deduped total, not the raw per-collection row count.
#[test]
fn selected_new_count_dedupes_a_set_missing_from_two_collections() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        LocalCollection {
            name: "Alpha - 100".to_string(),
            beatmap_checksums: Box::new([]),
        },
        LocalCollection {
            name: "Beta - 200".to_string(),
            beatmap_checksums: Box::new([]),
        },
    ]);
    tab.set_missing_beatmaps(
        vec![missing(1, 100, false), missing(1, 200, false)],
        &HashMap::new(),
    );

    assert_eq!(
        tab.selected_beatmapset_ids(),
        vec![1],
        "one beatmapset, shared by two checked collections, counted once"
    );
    assert_eq!(
        tab.selected_new_count(),
        1,
        "download button label matches the deduped run total"
    );
    // Right after a scan every with-updates collection is selected by default,
    // so the scan-toast headline (`total_new_count`) and the button under it
    // (`selected_new_count`) describe the same collections and must agree — a
    // toast saying "2 missing mapsets" over a button saying "download 1 new"
    // is the same drift as defect A, one layer up.
    assert_eq!(
        tab.total_new_count(),
        tab.selected_new_count(),
        "post-scan toast headline must not drift from the download button"
    );
}

/// Beatmapset 5 previously deleted from both collections 100 and 200 (both
/// still upstream, both checked): one row per collection, same shape as the
/// double-count above but for `held_back_count`, which renders in the same
/// toast/form block as `total_new_count`.
#[test]
fn held_back_count_dedupes_a_set_held_back_in_two_collections() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![
        LocalCollection {
            name: "Alpha - 100".to_string(),
            beatmap_checksums: Box::new([]),
        },
        LocalCollection {
            name: "Beta - 200".to_string(),
            beatmap_checksums: Box::new([]),
        },
    ]);
    tab.set_missing_beatmaps(
        vec![missing(5, 100, true), missing(5, 200, true)],
        &HashMap::new(),
    );

    assert_eq!(
        tab.held_back_count(),
        1,
        "one beatmapset, held back in two checked collections, counted once"
    );

    // Re-include both rows (the toggle's own mechanics are covered elsewhere;
    // this pins the counting derivation, not how a row gets re-included) and
    // check the headline lands on the same deduped total.
    for set in &mut tab.selection.cached_missing_sets {
        set.included = true;
    }
    assert_eq!(tab.held_back_count(), 0);
    assert_eq!(
        tab.total_new_count(),
        1,
        "the same beatmapset re-included from both collections is still one set"
    );
}

/// One collection holding a previously-deleted set (10) and two normal ones
/// (20, 30), with the collection checked. The shared fixture for the hold-back
/// tests. The split is deliberately uneven — a 1-and-1 fixture lets a count that
/// inverted its predicate still land on the expected number.
fn tab_with_one_held_back() -> UpdateSource {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![LocalCollection {
        name: "coll - 100".to_string(),
        beatmap_checksums: Box::new([]),
    }]);
    tab.set_missing_beatmaps(
        vec![
            missing(10, 100, true),
            missing(20, 100, false),
            missing(30, 100, false),
        ],
        &HashMap::new(),
    );
    assert!(
        tab.selection.local_collections[0].selected,
        "fixture precondition: the collection is checked, so only the per-set \
         hold-back can drop set 10"
    );
    tab
}

#[test]
fn previously_deleted_set_is_held_back_from_the_run_and_every_count() {
    let tab = tab_with_one_held_back();

    let del = tab
        .selection
        .cached_missing_sets
        .iter()
        .find(|b| b.id == 10)
        .unwrap();
    assert!(del.previously_deleted);
    assert!(
        !del.included,
        "the scan holds a previously-deleted set back"
    );

    assert_eq!(
        tab.selected_beatmapset_ids(),
        vec![20, 30],
        "the run enqueues only the sets that were not previously deleted"
    );
    // Every figure the user reads before pressing download must agree with that
    // id list; the browse's own row count must not, or the set is unreachable.
    assert_eq!(tab.selected_new_count(), 2, "download button label");
    assert_eq!(tab.new_count_for(100), 2, "per-collection `N new` badge");
    assert_eq!(tab.total_new_count(), 2, "form + scan-toast headline");
    assert_eq!(tab.held_back_count(), 1, "form `held back` metric");
    assert_eq!(
        tab.total_missing_count(),
        3,
        "browse rows, so all stay open"
    );
}

#[test]
fn re_including_a_held_back_set_puts_it_back_in_the_run_and_every_count() {
    let mut tab = tab_with_one_held_back();
    tab.descend();
    tab.focus_preview();
    // Park the cursor on set 10. Default preview sort keeps scan order, so the
    // held-back set is row 0 (no marked-installed rows in this fixture).
    tab.selection.preview_cursor = Some(0);
    assert_eq!(
        tab.preview_focused_included(),
        Some(false),
        "row 0 is the held-back set, and the footer advertises re-include there"
    );

    assert_eq!(tab.toggle_preview_included(), Some(true));

    let mut ids = tab.selected_beatmapset_ids();
    ids.sort_unstable();
    assert_eq!(ids, vec![10, 20, 30], "re-included set rejoins the run");
    assert_eq!(tab.selected_new_count(), 3, "download button label");
    assert_eq!(tab.new_count_for(100), 3, "per-collection `N new` badge");
    assert_eq!(tab.total_new_count(), 3, "form + scan-toast headline");
    assert_eq!(tab.held_back_count(), 0, "form `held back` metric");
    assert_eq!(tab.total_missing_count(), 3, "row count never moved");

    // The toggle is symmetric — a second press holds it back again.
    assert_eq!(tab.toggle_preview_included(), Some(false));
    assert_eq!(tab.selected_beatmapset_ids(), vec![20, 30]);
    assert_eq!(tab.selected_new_count(), 2);
    assert_eq!(tab.held_back_count(), 1);
}

#[test]
fn toggle_is_inert_on_a_set_that_was_never_deleted() {
    let mut tab = tab_with_one_held_back();
    tab.descend();
    tab.focus_preview();
    // Row 1 is set 20 — not previously deleted, so its membership is its
    // collection's checkbox and the per-set toggle must not open a second
    // selection model over it.
    tab.selection.preview_cursor = Some(1);
    assert_eq!(tab.preview_focused_included(), None, "no hint on this row");
    assert_eq!(tab.toggle_preview_included(), None);
    assert_eq!(
        tab.selected_beatmapset_ids(),
        vec![20, 30],
        "the inert press changed nothing"
    );
    assert_eq!(tab.held_back_count(), 1, "and held nothing else back");
}

#[test]
fn fetch_compare_settings_identifies_hidden_failed_ids() {
    let settings = FetchCompareSettings {
        hidden_failed_beatmapset_ids: HashSet::from([1234]),
        ..Default::default()
    };
    assert!(should_hide_failed_beatmapset(&settings, 1234));
    assert!(!should_hide_failed_beatmapset(&settings, 5678));
}

#[test]
fn fetch_compare_settings_hides_manually_ignored_ids() {
    let settings = FetchCompareSettings {
        ignored_beatmapset_ids: HashSet::from([42]),
        ..Default::default()
    };
    assert!(should_hide_failed_beatmapset(&settings, 42));
    assert!(!should_hide_failed_beatmapset(&settings, 99));
}
