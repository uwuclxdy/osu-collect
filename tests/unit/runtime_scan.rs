#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{
    BeatmapsetVerdict, CollectionBeatmapset, FetchCompareSettings, build_scan_summary,
    classify_beatmapset, missing_from_candidate, retain_held_back_in_snapshots,
    scan_collection_candidates, scan_fetched_collections, snapshots,
};
use crate::app::snapshots::{CollectionSnapshot, SnapshotDiff};
use crate::app::update_source::{MissingBeatmapset, MissingStatus};
use crate::core::collection::{Beatmap, Beatmapset, Collection, Uploader};
use crate::osu_db::{Md5, OsuClient, checksum};
use std::collections::{HashMap, HashSet};

fn candidate(id: u32) -> CollectionBeatmapset {
    CollectionBeatmapset {
        id,
        // Non-empty so the conditional carry is observable: an empty upstream set
        // makes both arms of `checksums` look identical.
        checksums: vec![md5(id as u8)],
        enrich_diff_id: Some(id * 10),
    }
}

/// A lazer diff naming `deleted_id` as manually deleted from collection 100.
fn diffs(deleted_id: u32) -> HashMap<u32, SnapshotDiff> {
    HashMap::from([(
        100,
        SnapshotDiff {
            manually_deleted: CollectionSnapshot {
                stable_hashes: Vec::new(),
                lazer_ids: vec![u64::from(deleted_id)],
            },
            manually_added: CollectionSnapshot::default(),
        },
    )])
}

/// The sole writer of the hold-back flag. Both legs run through it so the two
/// arms of `included: !previously_deleted` are each covered — a scan that stopped
/// holding deleted sets back, or started holding everything back, reds here.
#[test]
fn a_previously_deleted_candidate_is_built_held_back() {
    let diffs = diffs(7);

    let deleted = missing_from_candidate(
        &candidate(7),
        100,
        "col".to_string(),
        OsuClient::Lazer,
        &diffs,
    );
    assert!(deleted.previously_deleted);
    assert!(!deleted.included, "the run must not enqueue it");
    // Carried ONLY here: the snapshot rebuild re-expresses this set from these
    // hashes, and nothing else ever reads the field.
    assert_eq!(
        deleted.checksums.as_ref(),
        candidate(7).checksums.as_slice(),
        "a held-back set carries the checksums the stable rebuild needs"
    );

    let kept = missing_from_candidate(
        &candidate(8),
        100,
        "col".to_string(),
        OsuClient::Lazer,
        &diffs,
    );
    assert!(!kept.previously_deleted, "8 was never deleted");
    assert!(kept.included, "so the run enqueues it");
    assert!(
        kept.checksums.is_empty(),
        "and it pays no allocation for a field only a held-back set reads"
    );
}

/// A collection with no snapshot diff at all (first scan) holds nothing back.
#[test]
fn a_candidate_without_a_snapshot_diff_is_included() {
    let built = missing_from_candidate(
        &candidate(7),
        999,
        "col".to_string(),
        OsuClient::Lazer,
        &diffs(7),
    );
    assert!(!built.previously_deleted, "999 has no diff of its own");
    assert!(built.included);
}

// ── hidden-failed suppression count (defect: reported "known bad" figure must
// count what THIS scan hid, not the size of the failed-maps store) ───────────

fn upstream_beatmapset(id: u32) -> Beatmapset {
    Beatmapset {
        id,
        // Empty checksum: `api_checksums` filters it out, so these fixtures
        // never trip the "all checksums exist locally" skip and the suppression
        // reason under test stays isolated.
        beatmaps: vec![Beatmap {
            id: id * 10,
            checksum: String::new(),
        }],
    }
}

fn upstream_collection(id: u32, sets: Vec<Beatmapset>) -> Collection {
    Collection {
        id,
        name: format!("col {id}"),
        description: None,
        uploader: Uploader {
            id: 0,
            username: "uploader".to_string(),
        },
        beatmapsets: sets,
        favourites: 0,
    }
}

#[test]
fn classify_beatmapset_hidden_for_failed_or_ignored() {
    let settings = FetchCompareSettings {
        hidden_failed_beatmapset_ids: HashSet::from([1]),
        ignored_beatmapset_ids: HashSet::from([2]),
    };
    assert_eq!(
        classify_beatmapset(1, &[], &HashSet::new(), &settings),
        BeatmapsetVerdict::Hidden
    );
    assert_eq!(
        classify_beatmapset(2, &[], &HashSet::new(), &settings),
        BeatmapsetVerdict::Hidden
    );
    assert_eq!(
        classify_beatmapset(3, &[], &HashSet::new(), &settings),
        BeatmapsetVerdict::Candidate
    );
}

#[test]
fn classify_beatmapset_prefers_locally_present_over_hidden() {
    // A set whose checksums fully match the local library is resolved by that
    // match before the hidden/ignored check ever runs, so it must not count as
    // a suppression even though its id also sits in the failed-maps store.
    let cksum = md5(0xaa);
    let settings = FetchCompareSettings {
        hidden_failed_beatmapset_ids: HashSet::from([7]),
        ignored_beatmapset_ids: HashSet::new(),
    };
    assert_eq!(
        classify_beatmapset(7, &[cksum], &HashSet::from([cksum]), &settings),
        BeatmapsetVerdict::LocallyPresent
    );
}

/// The exact scenario the bug report describes: the failed-maps store holds ids
/// that never appear in the scanned collection at all. Only an id genuinely
/// walked during this scan can be counted as hidden by it.
#[test]
fn scan_collection_candidates_ignores_failed_ids_outside_the_collection() {
    let settings = FetchCompareSettings {
        hidden_failed_beatmapset_ids: HashSet::from([2, 999, 1000]),
        ignored_beatmapset_ids: HashSet::new(),
    };
    let collection = upstream_collection(100, vec![upstream_beatmapset(1), upstream_beatmapset(2)]);

    let (candidates, hidden) = scan_collection_candidates(
        &collection,
        100,
        &HashSet::new(),
        &HashSet::new(),
        &settings,
    );

    assert_eq!(
        hidden,
        HashSet::from([2]),
        "999 and 1000 never appear in this collection; only 2 was actually hidden"
    );
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].0.id, 1);
}

/// A beatmapset skipped because it's already in the local library must not be
/// counted as hidden either — that's a different suppression reason from
/// "known bad".
///
/// Separately, a MANUALLY-IGNORED id (4) is suppressed from candidates the
/// same way a known-bad one is, but must NOT join the hidden set: the "known
/// bad" figure and the recheck flow only ever read the failed-maps store, so
/// folding the ignore store in reports ids the `r` recheck can't act on and
/// wrongly gates the recheck key open on a scan with no known-bad maps at all.
#[test]
fn scan_collection_candidates_hidden_set_excludes_ignored_and_local_install_skips() {
    let settings = FetchCompareSettings {
        hidden_failed_beatmapset_ids: HashSet::from([3]),
        ignored_beatmapset_ids: HashSet::from([4]),
    };
    let local_set_ids = HashSet::from([1]);
    let collection = upstream_collection(
        100,
        vec![
            upstream_beatmapset(1),
            upstream_beatmapset(3),
            upstream_beatmapset(4),
            upstream_beatmapset(5),
        ],
    );

    let (candidates, hidden) =
        scan_collection_candidates(&collection, 100, &local_set_ids, &HashSet::new(), &settings);

    assert_eq!(
        hidden,
        HashSet::from([3]),
        "only 3 (failed-maps store) is known-bad; 1's local-install skip and \
         4's manually-ignored skip must not join the hidden set"
    );
    assert_eq!(
        candidates.iter().map(|c| c.0.id).collect::<Vec<_>>(),
        vec![5],
        "both 3 (known bad) and 4 (ignored) are still excluded from candidates"
    );
}

/// The same beatmapset id twice in one collection's upstream listing must
/// still produce a single candidate — `seen_in_collection` is what enforces
/// that, and no other fixture here has an intra-collection duplicate.
#[test]
fn scan_collection_candidates_dedupes_a_repeated_id_within_one_collection() {
    let settings = FetchCompareSettings::default();
    let collection = upstream_collection(100, vec![upstream_beatmapset(1), upstream_beatmapset(1)]);

    let (candidates, hidden) = scan_collection_candidates(
        &collection,
        100,
        &HashSet::new(),
        &HashSet::new(),
        &settings,
    );

    assert_eq!(
        candidates.len(),
        1,
        "the repeated id must not become two candidates"
    );
    assert!(hidden.is_empty());
}

/// `scan_collection_candidates` takes the REQUESTED collection id separately
/// from the payload's own `.id` because in production they can differ; the
/// requested id is what lands in `MissingBeatmapset.collection_id` and is
/// matched against ids extracted from local collection names. A fixture where
/// the payload disagrees with the request catches an accidental swap that a
/// same-id fixture cannot.
#[test]
fn scan_collection_candidates_tags_the_requested_id_not_the_payload_id() {
    let settings = FetchCompareSettings::default();
    // Payload claims id 999; the request was for collection 100.
    let collection = upstream_collection(999, vec![upstream_beatmapset(1)]);

    let (candidates, _hidden) = scan_collection_candidates(
        &collection,
        100,
        &HashSet::new(),
        &HashSet::new(),
        &settings,
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].1, 100,
        "the candidate must carry the requested id, not the payload's own id"
    );
}

/// `hidden_failed_count`'s fix (defect B) must not re-introduce defect A's
/// double-count shape: a beatmapset hidden in two scanned collections is one
/// id, not two. The accumulation across collections is a `HashSet` union
/// (`scan_fetched_collections`), not a running sum of per-collection counts —
/// this fixture also catches an accumulator that assigns the last collection's
/// set instead of extending: collection 200 (processed last) carries only a
/// SUBSET of collection 100's hidden ids, so an assignment would silently
/// drop id 7.
#[test]
fn scan_fetched_collections_unions_hidden_ids_across_collections() {
    let settings = FetchCompareSettings {
        hidden_failed_beatmapset_ids: HashSet::from([7, 42]),
        ignored_beatmapset_ids: HashSet::new(),
    };
    let fetched = vec![
        (
            100,
            upstream_collection(100, vec![upstream_beatmapset(7), upstream_beatmapset(42)]),
        ),
        (200, upstream_collection(200, vec![upstream_beatmapset(42)])),
    ];

    let (candidates, hidden) =
        scan_fetched_collections(&fetched, &HashSet::new(), &HashSet::new(), &settings);

    assert_eq!(
        hidden,
        HashSet::from([7, 42]),
        "42 is hidden in both collections and counts once; 7 must survive \
         collection 200 being processed after it"
    );
    assert!(candidates.is_empty());
}

/// The headline counts what a run would fetch, so the two zero cases mean
/// different things and must not share a title: nothing missing at all, versus
/// everything missing being held back.
#[test]
fn scan_summary_titles_separate_the_two_zero_cases() {
    assert_eq!(build_scan_summary(0, 0, 0, 0).0, "no missing mapsets");
    assert_eq!(build_scan_summary(0, 3, 0, 0).0, "nothing to fetch");
}

#[test]
fn scan_summary_headline_counts_only_fetchable_sets() {
    let (title, detail) = build_scan_summary(7, 3, 0, 0);
    assert_eq!(title, "7 missing mapsets");
    assert_eq!(
        detail.as_deref(),
        Some("3 previously deleted, held back"),
        "the held-back sets are named in the detail, not folded into the headline"
    );
}

#[test]
fn scan_summary_singular_at_one() {
    assert_eq!(build_scan_summary(1, 0, 0, 0).0, "1 missing mapset");
}

#[test]
fn scan_summary_detail_joins_every_caveat_in_order() {
    let (_, detail) = build_scan_summary(4, 1, 2, 5);
    assert_eq!(
        detail.as_deref(),
        Some("1 previously deleted, held back · 2 added since last scan · 5 known bad")
    );
}

#[test]
fn scan_summary_has_no_detail_without_caveats() {
    assert_eq!(build_scan_summary(4, 0, 0, 0).1, None);
}

// ── snapshot rebuild (shared by the TUI run write and the CLI report write) ───

fn snapshot_file(id: u32, stable: &[Md5], lazer: &[u64]) -> snapshots::CollectionSnapshotFile {
    snapshots::CollectionSnapshotFile::new(
        id,
        format!("col - {id}"),
        CollectionSnapshot {
            stable_hashes: stable.iter().map(|&h| checksum::to_hex(h)).collect(),
            lazer_ids: lazer.to_vec(),
        },
    )
}

fn md5(seed: u8) -> Md5 {
    let mut out = [0u8; 16];
    out[0] = seed;
    out
}

fn held_back(id: u32, collection_id: u32, checksums: &[Md5]) -> MissingBeatmapset {
    MissingBeatmapset {
        id,
        status: MissingStatus::NotInstalled,
        collection_id,
        collection_name: format!("col - {collection_id}"),
        included: false,
        previously_deleted: true,
        checksums: checksums.to_vec().into_boxed_slice(),
        enrich_diff_id: None,
    }
}

/// Stable snapshots are keyed by beatmap hash, so a held-back set is re-expressed
/// from the checksums the scan captured. An INCLUDED set must not be folded in —
/// it is genuinely absent from the library and the next scan should keep finding
/// it missing.
#[test]
fn rebuild_readds_only_held_back_sets_stable() {
    let (a, m, b) = (md5(0xa1), md5(0xcc), md5(0xb2));
    let mut files = HashMap::from([(100, snapshot_file(100, &[a], &[]))]);
    let mut fetchable = held_back(2, 100, &[b]);
    fetchable.included = true;
    fetchable.previously_deleted = false;

    retain_held_back_in_snapshots(
        &mut files,
        OsuClient::Stable,
        &[held_back(3, 100, &[m]), fetchable],
    );

    assert_eq!(
        files[&100].snapshot.stable_hashes,
        vec![checksum::to_hex(a), checksum::to_hex(m)],
        "the held-back set rejoins the baseline; the still-missing one does not"
    );
}

/// Lazer snapshots are keyed by set id, so the same decision reads a different
/// field. Both arms exist, so both are enumerated.
#[test]
fn rebuild_readds_only_held_back_sets_lazer() {
    let mut files = HashMap::from([(100, snapshot_file(100, &[], &[1]))]);
    retain_held_back_in_snapshots(&mut files, OsuClient::Lazer, &[held_back(3, 100, &[])]);
    assert_eq!(files[&100].snapshot.lazer_ids, vec![1, 3]);
    assert!(
        files[&100].snapshot.stable_hashes.is_empty(),
        "the lazer arm must not write into the stable field"
    );
}

/// A held-back set only rejoins ITS OWN collection's baseline.
#[test]
fn rebuild_is_scoped_to_the_held_back_sets_collection() {
    let mut files = HashMap::from([
        (100, snapshot_file(100, &[], &[1])),
        (200, snapshot_file(200, &[], &[9])),
    ]);
    retain_held_back_in_snapshots(&mut files, OsuClient::Lazer, &[held_back(3, 100, &[])]);
    assert_eq!(files[&100].snapshot.lazer_ids, vec![1, 3]);
    assert_eq!(
        files[&200].snapshot.lazer_ids,
        vec![9],
        "collection 200 holds nothing back, so its baseline is untouched"
    );
}
