#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{
    BeatmapsetVerdict, CollectionBeatmapset, FetchCompareSettings, build_scan_summary,
    classify_beatmapset, exclude_reincluded_sets, missing_from_candidate, persist_scan_baselines,
    retain_held_back_in_snapshots, scan_collection_candidates, scan_fetched_collections, snapshots,
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
    // `exclude_reincluded_sets` reads these to strip a re-included set's hashes
    // from the stable diff. Nothing else does.
    assert_eq!(
        deleted.checksums.as_ref(),
        candidate(7).checksums.as_slice(),
        "a held-back set carries the checksums the re-include exclusion needs"
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
        "a non-deleted set pays no allocation for a field only the exclusion reads"
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

/// A diff whose `manually_deleted` carries the given stable hashes for one
/// collection. The fold sources from here, not from the scan's missing list.
fn stable_deleted(collection_id: u32, hashes: &[Md5]) -> (u32, SnapshotDiff) {
    (
        collection_id,
        SnapshotDiff {
            manually_deleted: CollectionSnapshot {
                stable_hashes: hashes.iter().map(|&h| checksum::to_hex(h)).collect(),
                lazer_ids: Vec::new(),
            },
            manually_added: CollectionSnapshot::default(),
        },
    )
}

/// A diff whose `manually_deleted` carries the given lazer ids for one
/// collection.
fn lazer_deleted(collection_id: u32, ids: &[u64]) -> (u32, SnapshotDiff) {
    (
        collection_id,
        SnapshotDiff {
            manually_deleted: CollectionSnapshot {
                stable_hashes: Vec::new(),
                lazer_ids: ids.to_vec(),
            },
            manually_added: CollectionSnapshot::default(),
        },
    )
}

/// Stable snapshots are keyed by beatmap hash. Only what the old baseline
/// recorded as deleted rejoins the new one.
#[test]
fn rebuild_folds_only_manually_deleted_stable_hashes() {
    let (a, m) = (md5(0xa1), md5(0xcc));
    let mut files = HashMap::from([(100, snapshot_file(100, &[a], &[]))]);
    let diffs = HashMap::from([stable_deleted(100, &[m])]);

    retain_held_back_in_snapshots(&mut files, OsuClient::Stable, &diffs);

    assert_eq!(
        files[&100].snapshot.stable_hashes,
        vec![checksum::to_hex(a), checksum::to_hex(m)],
        "the deleted set rejoins the baseline"
    );
}

/// Lazer snapshots are keyed by set id. Both arms exist, so both are enumerated.
#[test]
fn rebuild_folds_only_manually_deleted_lazer_ids() {
    let mut files = HashMap::from([(100, snapshot_file(100, &[], &[1]))]);
    let diffs = HashMap::from([lazer_deleted(100, &[3])]);

    retain_held_back_in_snapshots(&mut files, OsuClient::Lazer, &diffs);

    assert_eq!(files[&100].snapshot.lazer_ids, vec![1, 3]);
    assert!(
        files[&100].snapshot.stable_hashes.is_empty(),
        "the lazer arm must not write into the stable field"
    );
}

/// A deleted entry only rejoins ITS OWN collection's baseline.
#[test]
fn rebuild_is_scoped_to_the_diffs_collection() {
    let mut files = HashMap::from([
        (100, snapshot_file(100, &[], &[1])),
        (200, snapshot_file(200, &[], &[9])),
    ]);
    let diffs = HashMap::from([lazer_deleted(100, &[3])]);

    retain_held_back_in_snapshots(&mut files, OsuClient::Lazer, &diffs);

    assert_eq!(files[&100].snapshot.lazer_ids, vec![1, 3]);
    assert_eq!(
        files[&200].snapshot.lazer_ids,
        vec![9],
        "collection 200 has no diff, so its baseline is untouched"
    );
}

/// The fold sources from the baseline diff, not the scan's missing list. A set
/// the user marked installed is absent from the missing list (and the next
/// scan's ignored-maps gate hides it from the candidate list entirely), but it
/// stays in `manually_deleted` for as long as it is absent from the local
/// library. The fold must preserve it.
#[test]
fn rebuild_preserves_a_deleted_set_absent_from_the_missing_list() {
    let mut files = HashMap::from([(100, snapshot_file(100, &[], &[1]))]);
    let diffs = HashMap::from([lazer_deleted(100, &[5])]);

    retain_held_back_in_snapshots(&mut files, OsuClient::Lazer, &diffs);

    assert_eq!(
        files[&100].snapshot.lazer_ids,
        vec![1, 5],
        "a marked-installed set's deletion record survives the fold"
    );
}

// ── re-include exclusion (strips re-included sets before the fold) ───────────

fn missing_entry(
    id: u32,
    collection_id: u32,
    included: bool,
    previously_deleted: bool,
) -> MissingBeatmapset {
    MissingBeatmapset {
        id,
        status: MissingStatus::NotInstalled,
        collection_id,
        collection_name: format!("col {collection_id}"),
        included,
        previously_deleted,
        checksums: Box::new([]),
        enrich_diff_id: None,
    }
}

/// A re-included set must be stripped from the diff's `manually_deleted` before
/// the fold, or the run writes it back as deleted and the next scan undoes the
/// re-include. A held-back set (not re-included) stays.
#[test]
fn exclude_reincluded_strips_reincluded_keeps_held_back_lazer() {
    let diffs = lazer_deleted(100, &[3, 5]);
    let missing = vec![
        missing_entry(3, 100, true, true),  // re-included
        missing_entry(5, 100, false, true), // still held back
    ];

    let result = exclude_reincluded_sets(HashMap::from([diffs]), OsuClient::Lazer, &missing);

    assert_eq!(result[&100].manually_deleted.lazer_ids, vec![5]);
}

/// Same exclusion on the stable arm: the re-included set's hashes are removed.
#[test]
fn exclude_reincluded_strips_reincluded_stable() {
    let (m, r) = (md5(0xcc), md5(0xdd));
    let diffs = stable_deleted(100, &[m, r]);
    let missing = vec![
        MissingBeatmapset {
            id: 3,
            status: MissingStatus::NotInstalled,
            collection_id: 100,
            collection_name: "col 100".to_string(),
            included: true,
            previously_deleted: true,
            checksums: Box::new([m]),
            enrich_diff_id: None,
        },
        missing_entry(5, 100, false, true),
    ];

    let result = exclude_reincluded_sets(HashMap::from([diffs]), OsuClient::Stable, &missing);

    assert_eq!(
        result[&100].manually_deleted.stable_hashes,
        vec![checksum::to_hex(r)],
        "only the re-included set's hash is removed; the held-back set stays"
    );
}

/// No re-includes → the diff is returned unchanged.
#[test]
fn exclude_reincluded_noop_without_reincludes() {
    let diffs = lazer_deleted(100, &[3, 5]);
    let missing = vec![missing_entry(3, 100, false, true)];

    let result = exclude_reincluded_sets(HashMap::from([diffs]), OsuClient::Lazer, &missing);

    assert_eq!(result[&100].manually_deleted.lazer_ids, vec![3, 5]);
}

/// Re-including a set in one collection must not strip it from another
/// collection's diff. A set can be held-back in multiple collections; the
/// re-include is per-collection.
#[test]
fn exclude_reincluded_is_scoped_to_the_sets_own_collection() {
    let diffs = HashMap::from([lazer_deleted(100, &[5]), lazer_deleted(200, &[5])]);
    let missing = vec![missing_entry(5, 100, true, true)]; // re-included in 100 only

    let result = exclude_reincluded_sets(diffs, OsuClient::Lazer, &missing);

    assert!(
        result[&100].manually_deleted.lazer_ids.is_empty(),
        "collection 100's hold-back for set 5 is stripped (re-included)"
    );
    assert_eq!(
        result[&200].manually_deleted.lazer_ids,
        vec![5],
        "collection 200's hold-back for set 5 survives"
    );
}

// ── scan-time baseline writer (a deletion observed by a plain scan must be
// detectable as previously deleted, not plain missing) ──────────────────────

/// Build an `App` whose update-source scan state holds one local collection
/// ("col - 100", containing beatmapset 1) and a snapshot diff recording
/// beatmapset 7 as manually deleted (absent from the local library). The env
/// guards must already be in place — `App::new` reads `STATE_ENV_PATH` and
/// `AUTH_ENV_PATH` at construction (`docs/architecture.md` § On-disk stores).
fn app_with_local_collection_and_a_deleted_set() -> crate::app::App {
    use crate::config::Config;
    use crate::osu_db::{LocalBeatmap, LocalBeatmapset, LocalCollection};

    let cksum = md5(0xa1);
    let mut app = crate::app::App::new(Config::default());
    app.library.client_type = OsuClient::Lazer;
    app.home.update.set_collections(vec![LocalCollection {
        name: "col - 100".to_string(),
        beatmap_checksums: Box::new([cksum]),
    }]);
    app.home.update.set_local_beatmapsets(vec![LocalBeatmapset {
        id: 1,
        beatmaps: Box::new([LocalBeatmap { checksum: cksum }]),
    }]);
    app.home.update.scan.snapshot_diffs = HashMap::from([lazer_deleted(100, &[7])]);
    app
}

/// A scan with no download in flight persists a baseline built from the local
/// library, folded through the shared held-back logic so the deletion record
/// (set 7, absent locally) survives. The next scan sees set 7 as previously
/// deleted rather than plain missing.
#[tokio::test(flavor = "multi_thread")]
async fn scan_baselines_persist_when_no_download_is_active() {
    use crate::app::collection_state::STATE_ENV_PATH;
    use crate::auth::AUTH_ENV_PATH;
    use crate::test_env::TempEnvVar;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let _env = TempEnvVar::set_all([
        (snapshots::SNAPSHOT_ENV_DIR, dir.path().to_str().unwrap()),
        (
            STATE_ENV_PATH,
            dir.path().join("state.toml").to_str().unwrap(),
        ),
        (AUTH_ENV_PATH, "/dev/null/no-such-auth"),
    ]);

    let app = app_with_local_collection_and_a_deleted_set();
    persist_scan_baselines(&app);

    // The write is fire-and-forget spawn_blocking; poll for the file.
    let path = snapshots::snapshot_path(dir.path(), 100);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if path.exists() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("snapshot file was not written within 3s");

    let written = snapshots::load(&path).expect("snapshot file loads");
    assert!(
        written.snapshot.lazer_ids.contains(&1),
        "the locally-present set must be in the baseline"
    );
    assert!(
        written.snapshot.lazer_ids.contains(&7),
        "the held-back deletion record must survive the write"
    );
}

/// A scan while a download is active writes nothing. The run's completion
/// write builds from request-time data and would overwrite this fresher
/// baseline, so the scan-time write is suppressed entirely.
#[tokio::test(flavor = "multi_thread")]
async fn scan_baselines_skip_when_a_download_is_active() {
    use crate::app::CollectionPage;
    use crate::app::collection_state::STATE_ENV_PATH;
    use crate::auth::AUTH_ENV_PATH;
    use crate::test_env::TempEnvVar;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let _env = TempEnvVar::set_all([
        (snapshots::SNAPSHOT_ENV_DIR, dir.path().to_str().unwrap()),
        (
            STATE_ENV_PATH,
            dir.path().join("state.toml").to_str().unwrap(),
        ),
        (AUTH_ENV_PATH, "/dev/null/no-such-auth"),
    ]);

    let mut app = app_with_local_collection_and_a_deleted_set();
    // `CollectionPage::new` defaults to `DownloadStage::Pending`, which
    // `is_settled()` is false — so `is_downloading()` returns true.
    app.downloads
        .push(CollectionPage::new(1, "active run".to_string(), 1));

    persist_scan_baselines(&app);

    // The guard returns before spawn_blocking, so nothing is in flight. A brief
    // sleep catches the mutation case (guard removed → write lands).
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let path = snapshots::snapshot_path(dir.path(), 100);
    assert!(
        !path.exists(),
        "no snapshot should be written while a download is active"
    );
}
