#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{report_counts, row_tag};
use crate::app::update_source::{MissingBeatmapset, MissingStatus, UpdateSource};
use crate::osu_db::LocalCollection;
use std::collections::HashMap;

/// Mirrors what `fetch_missing_beatmapsets` writes: a previously-deleted set is
/// constructed already held back.
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

fn local_col(name: &str) -> LocalCollection {
    LocalCollection {
        name: name.to_string(),
        beatmap_checksums: Box::new([]),
    }
}

/// The CLI's report and the TUI's run read one hold-back predicate. Each side is
/// driven through its own entry point — `report_counts` and
/// `UpdateSource::selected_beatmapset_ids` — rather than recomputing the filter
/// in this body, so the test reds if either surface drifts from the other.
#[test]
fn cli_report_counts_match_what_an_update_run_enqueues() {
    let missing_sets = vec![
        missing(1, 100, false),
        missing(2, 100, true),
        missing(3, 200, true),
        missing(4, 200, false),
    ];

    let (fetchable, held_back) = report_counts(&missing_sets);

    let mut tab = UpdateSource::new();
    tab.set_collections(vec![local_col("Alpha - 100"), local_col("Beta - 200")]);
    tab.set_missing_beatmaps(missing_sets.clone(), &HashMap::new());
    assert!(
        tab.selection.local_collections.iter().all(|c| c.selected),
        "fixture precondition: both collections are checked, so the run's set is \
         narrowed only by the hold-back"
    );

    let mut enqueued = tab.selected_beatmapset_ids();
    enqueued.sort_unstable();
    assert_eq!(enqueued, vec![1, 4], "the run drops both deleted sets");
    assert_eq!(
        fetchable,
        enqueued.len(),
        "the report's headline counts what a run would enqueue"
    );
    assert_eq!(held_back, 2, "the summary's held-back figure");
    assert_eq!(
        fetchable + held_back,
        missing_sets.len(),
        "every scanned row lands in exactly one of the two figures"
    );

    // The listing's markers are driven per row through their own function, so a
    // marker that stopped tracking the count reds here rather than shipping a
    // listing that contradicts its own summary.
    let tagged: Vec<u32> = missing_sets
        .iter()
        .filter(|m| !row_tag(m).is_empty())
        .map(|m| m.id)
        .collect();
    assert_eq!(
        tagged,
        vec![2, 3],
        "exactly the deleted sets carry a marker"
    );
    assert_eq!(tagged.len(), held_back, "marker count == summary count");
    assert_eq!(row_tag(&missing_sets[0]), "", "a fetchable row is untagged");
    assert_eq!(row_tag(&missing_sets[1]), " [held back]");
}

/// Nothing deleted → nothing held back, and the headline is the whole scan.
#[test]
fn cli_report_counts_hold_nothing_back_without_a_deletion() {
    let missing_sets = vec![missing(1, 100, false), missing(2, 100, false)];
    assert_eq!(report_counts(&missing_sets), (2, 0));
}

/// The report command's own snapshot write. It never downloads, so writing the
/// scan's baselines verbatim would erase the deletions the report is about to
/// call held back — the summary would contradict the file it just wrote.
///
/// Asserts the BYTES ON DISK, not a returned map: dropping the rebuild from
/// `persist_baselines` then changes what lands, where a return-value assertion
/// would have left the caller free to skip it with the suite still green.
#[test]
fn cli_persisted_baselines_keep_held_back_sets_recorded_as_deleted() {
    use super::persist_baselines;
    use crate::app::snapshots::{self, CollectionSnapshot, CollectionSnapshotFile};
    use crate::osu_db::{Md5, OsuClient, checksum};

    fn md5(seed: u8) -> Md5 {
        let mut out = [0u8; 16];
        out[0] = seed;
        out
    }
    let (a, m) = (md5(0xa1), md5(0xcc));
    let dir = tempfile::tempdir().expect("temp dir");

    let mut deleted = missing(3, 100, true);
    deleted.checksums = Box::new([m]);
    // The scan's baseline reads the LOCAL library, where M is already gone.
    let current = HashMap::from([(
        100,
        CollectionSnapshotFile::new(
            100,
            "col - 100".to_string(),
            CollectionSnapshot {
                stable_hashes: vec![checksum::to_hex(a)],
                lazer_ids: Vec::new(),
            },
        ),
    )]);

    persist_baselines(
        dir.path(),
        current,
        OsuClient::Stable,
        &[missing(2, 100, false), deleted],
    );

    let written =
        snapshots::load(&snapshots::snapshot_path(dir.path(), 100)).expect("baseline written");
    assert_eq!(
        written.snapshot.stable_hashes,
        vec![checksum::to_hex(a), checksum::to_hex(m)],
        "M stays in the baseline on disk, so the next scan still reports it deleted"
    );
}
