#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{
    CollectionBeatmapset, build_scan_summary, missing_from_candidate,
    retain_held_back_in_snapshots, snapshots,
};
use crate::app::snapshots::{CollectionSnapshot, SnapshotDiff};
use crate::app::update_source::{MissingBeatmapset, MissingStatus};
use crate::osu_db::{Md5, OsuClient, checksum};
use std::collections::HashMap;

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
