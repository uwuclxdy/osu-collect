use crate::app::{
    runtime::{FetchCompareSettings, collection_ids_for_scan, should_hide_failed_beatmapset},
    update_source::{MissingBeatmapset, MissingStatus, UpdateSource},
};
use crate::osu_db::{LocalBeatmap, LocalBeatmapset, LocalCollection, Md5};
use std::collections::HashSet;

fn test_md5(seed: u8) -> Md5 {
    let mut out = [0u8; 16];
    out[0] = seed;
    out
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
    tab.set_missing_beatmaps(vec![]);

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
    tab.set_missing_beatmaps(vec![
        missing(1, 100, false),
        missing(2, 100, false),
        missing(3, 200, false),
    ]);

    let mut all = tab.selected_beatmapset_ids();
    all.sort_unstable();
    assert_eq!(all, vec![1, 2, 3], "every collection selected by default");

    // Deselect the first collection (100): its two maps drop out.
    tab.selection.local_collections[0].selected = false;
    assert_eq!(tab.selected_beatmapset_ids(), vec![3]);
    assert_eq!(tab.selected_collection_ids(), vec![200]);
}

#[test]
fn previously_deleted_flag_is_stored_on_missing_sets() {
    let mut tab = UpdateSource::new();
    tab.set_collections(vec![LocalCollection {
        name: "coll - 100".to_string(),
        beatmap_checksums: Box::new([]),
    }]);
    tab.set_missing_beatmaps(vec![missing(10, 100, true), missing(20, 100, false)]);

    let del = tab
        .selection
        .cached_missing_sets
        .iter()
        .find(|b| b.id == 10)
        .unwrap();
    assert!(del.previously_deleted);
    // Whole-collection selection: the set is still downloadable when its
    // collection is selected, regardless of the previously-deleted marker.
    assert!(tab.selected_beatmapset_ids().contains(&10));
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
