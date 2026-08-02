use super::{try_remove_empty_output_dir, unsatisfied_targets, verified_targets};
use crate::app::collection::FailureReason;
use crate::core::collection::{test_beatmapset, test_collection};
use crate::download::collection_db::create_selective_collection_db;
use crate::download::events::{Tally, translate_event};
use crate::download::session::{DownloadSession, PrepareParams, PrepareTarget};
use crate::download::{
    ActiveDownloadRegistry, ArchiveValidation, BeatmapStage, DownloadConfig, DownloadEvent,
    SelectiveDownloadCollection, selective_folder_name,
};
use crate::osu_db::OsuClient;
use osu_downloader::{Event as LibEvent, MirrorKind, MirrorRef, Skip, Status};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tempfile::tempdir;
use tokio::sync::watch;

fn make_selective(id: u32, name: &str, beatmapset_ids: Vec<u32>) -> SelectiveDownloadCollection {
    SelectiveDownloadCollection {
        id,
        name: name.to_string(),
        beatmapset_ids,
    }
}

/// A built-in mirror's [`MirrorRef`] (kind + static host) for event fixtures.
fn mirror_ref_of(kind: MirrorKind) -> MirrorRef {
    MirrorRef {
        kind,
        host: kind.host().into(),
    }
}

fn drive_status(status: Status) -> DownloadEvent {
    let captured: std::sync::Arc<std::sync::Mutex<Option<DownloadEvent>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_clone = std::sync::Arc::clone(&captured);
    let emit = move |event: DownloadEvent| {
        *captured_clone.lock().unwrap() = Some(event);
    };
    let mut tally = Tally::default();
    translate_event(
        0,
        LibEvent::BeatmapsetStatus {
            beatmapset_id: 0,
            status,
        },
        &mut tally,
        &emit,
    );
    captured.lock().unwrap().take().unwrap()
}

fn drive_translate(events: Vec<LibEvent>) -> (Tally, Vec<DownloadEvent>) {
    let captured: Arc<Mutex<Vec<DownloadEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = Arc::clone(&captured);
    let emit = move |event: DownloadEvent| captured_clone.lock().unwrap().push(event);
    let mut tally = Tally::default();
    for event in events {
        translate_event(42, event, &mut tally, &emit);
    }
    let collected = std::mem::take(&mut *captured.lock().unwrap());
    (tally, collected)
}

fn last_overall_progress(events: &[DownloadEvent]) -> &DownloadEvent {
    events
        .iter()
        .rev()
        .find(|event| matches!(event, DownloadEvent::OverallProgress { .. }))
        .expect("at least one OverallProgress emission")
}

fn completed(beatmapset_id: u32) -> LibEvent {
    LibEvent::BeatmapsetCompleted {
        beatmapset_id,
        filename: format!("{beatmapset_id}.osz"),
        size_bytes: 0,
        md5_hash: Some("md5".into()),
        mirror_used: mirror_ref_of(MirrorKind::Nerinyan),
        verify_duration_us: 0,
    }
}

#[test]
fn completed_events_populate_tally_successful() {
    let (tally, _events) = drive_translate(vec![completed(10), completed(20)]);
    assert_eq!(tally.downloaded, 2);
    assert!(tally.successful.contains(&10) && tally.successful.contains(&20));
    assert_eq!(tally.to_summary().downloaded, 2);
}

#[test]
fn missing_progress_total_translates_to_zero_total() {
    let (_tally, events) = drive_translate(vec![LibEvent::Progress {
        beatmapset_id: 42,
        downloaded_bytes: 1_500_000,
        total_bytes: None,
        speed_bps: 0,
    }]);

    assert!(matches!(
        events.as_slice(),
        [DownloadEvent::BeatmapProgress {
            id: 42,
            beatmapset_id: 42,
            downloaded: 1_500_000,
            total: 0,
        }]
    ));
}

#[test]
fn network_error_counts_as_failed() {
    let (tally, events) = drive_translate(vec![LibEvent::BeatmapsetFailed {
        beatmapset_id: 77,
        error: osu_downloader::Error::Network("timeout".into()),
        mirror: None,
    }]);
    assert_eq!(tally.failed, 1);
    assert!(tally.failures.iter().any(|f| f.beatmapset_id == 77));
    assert!(events.iter().any(|event| matches!(
        event,
        DownloadEvent::BeatmapStatus {
            beatmapset_id: 77,
            stage: BeatmapStage::Failed,
            ..
        }
    )));
    let DownloadEvent::OverallProgress { failed, .. } = last_overall_progress(&events) else {
        unreachable!()
    };
    assert_eq!(*failed, 1);
}

#[test]
fn deferred_map_emits_deferred_event_and_touches_no_tally() {
    let (tally, events) = drive_translate(vec![LibEvent::BeatmapsetDeferred {
        beatmapset_id: 42,
        pass: 2,
        retry_in: std::time::Duration::from_secs(30),
    }]);

    // Deferred is a soft requeue: nothing counted, so the map stays "queued".
    assert_eq!(tally.downloaded, 0);
    assert_eq!(tally.skipped, 0);
    assert_eq!(tally.failed, 0);
    assert!(tally.failures.is_empty());

    // Exactly one BeatmapDeferred, carrying the pass + retry_in; no progress emit.
    assert!(matches!(
        events.as_slice(),
        [DownloadEvent::BeatmapDeferred {
            beatmapset_id: 42,
            pass: 2,
            ..
        }]
    ));
}

#[test]
fn already_exists_still_counts_as_skipped() {
    let (tally, _events) = drive_translate(vec![LibEvent::BeatmapsetSkipped {
        beatmapset_id: 5,
        reason: Skip::AlreadyExists,
    }]);
    assert_eq!(tally.skipped, 1);
    assert_eq!(tally.failed, 0);
}

#[tokio::test]
async fn empty_output_dir_is_removed_after_cancel() {
    let root = tempdir().unwrap();
    let empty = root.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let occupied = root.path().join("occupied");
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("123.osz"), b"hi").unwrap();

    try_remove_empty_output_dir(&empty).await;
    assert!(!empty.exists(), "empty output dir must be removed");

    try_remove_empty_output_dir(&occupied).await;
    assert!(occupied.exists(), "non-empty output dir must remain");
}

#[test]
fn finish_emits_summary_and_completed_stage() {
    use crate::download::events::emit_finish;
    use crate::download::{DownloadStage, DownloadSummary};

    let events: Arc<Mutex<Vec<DownloadEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let emit = move |event: DownloadEvent| events_clone.lock().unwrap().push(event);

    emit_finish(
        99,
        &emit,
        DownloadSummary {
            downloaded: 3,
            skipped: 1,
            failed: 0,
            unverified: 0,
        },
    );

    let collected = events.lock().unwrap().clone();
    assert!(matches!(
        collected.as_slice(),
        [
            DownloadEvent::Finished {
                id: 99,
                summary: DownloadSummary { downloaded: 3, .. }
            },
            DownloadEvent::StageChanged {
                id: 99,
                stage: DownloadStage::Completed
            },
        ]
    ));
}

#[test]
fn duplicate_completed_events_dedupe_in_successful_set() {
    let (tally, _events) = drive_translate(vec![completed(10), completed(10)]);
    assert_eq!(tally.downloaded, 2);
    assert_eq!(tally.successful.len(), 1);
}

#[test]
fn unavailable_on_mirrors_is_recorded_as_failure() {
    let (tally, _events) = drive_translate(vec![LibEvent::BeatmapsetSkipped {
        beatmapset_id: 7,
        reason: Skip::UnavailableOnMirrors,
    }]);
    assert_eq!(tally.failed, 1);
    assert_eq!(tally.skipped, 0);
    assert!(
        tally
            .failures
            .iter()
            .any(|f| f.beatmapset_id == 7 && f.reason == FailureReason::NotFound)
    );
}

#[test]
fn completed_event_decrements_unverified_when_present() {
    let mut tally = Tally {
        unverified: 2,
        ..Tally::default()
    };
    let captured: Arc<Mutex<Vec<DownloadEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = Arc::clone(&captured);
    let emit = move |event| captured_clone.lock().unwrap().push(event);
    translate_event(1, completed(123), &mut tally, &emit);
    assert_eq!(tally.unverified, 1);
}

#[tokio::test]
async fn write_selective_collection_db_skips_empty_set() {
    use crate::download::collection_db::write_selective_collection_db;
    use std::collections::HashSet;

    let dir = tempdir().unwrap();
    let collection = test_collection(1, vec![test_beatmapset(10, &["hash"])]);

    write_selective_collection_db(
        collection,
        Vec::new(),
        HashSet::new(),
        dir.path().to_path_buf(),
    )
    .await
    .expect("empty verified set must succeed without writing a db");

    assert!(!dir.path().join("collection.db").exists());
}

#[test]
fn emit_status_messages_match_format_output() {
    let mirrors = [
        MirrorKind::Nerinyan,
        MirrorKind::OsuDirect,
        MirrorKind::Sayobot,
        MirrorKind::Nekoha,
    ];
    for mirror in mirrors {
        let label = mirror.label();

        let DownloadEvent::BeatmapStatus { message, .. } = drive_status(Status::Contacting {
            mirror: mirror_ref_of(mirror),
        }) else {
            panic!("expected BeatmapStatus");
        };
        assert_eq!(message, format!("checking {label}"));

        let DownloadEvent::BeatmapStatus { message, .. } = drive_status(Status::Downloading {
            mirror: mirror_ref_of(mirror),
        }) else {
            panic!("expected BeatmapStatus");
        };
        assert_eq!(message, format!("downloading from {label}"));

        let DownloadEvent::BeatmapStatus { message, .. } = drive_status(Status::Verifying {
            mirror: mirror_ref_of(mirror),
        }) else {
            panic!("expected BeatmapStatus");
        };
        assert_eq!(message, format!("verifying from {label}"));

        let reasons = [
            "connection reset",
            "connection reset by peer (os error 104)",
        ];
        for reason in reasons {
            let DownloadEvent::BeatmapStatus {
                message,
                rate_limited,
                ..
            } = drive_status(Status::RetryingTransient {
                mirror: mirror_ref_of(mirror),
                attempt: 2,
                max_attempts: 3,
                reason: reason.to_string(),
            })
            else {
                panic!("expected BeatmapStatus");
            };
            assert_eq!(
                message,
                format!("retrying {label} after {reason} (attempt 2/3)")
            );
            assert!(!rate_limited);
        }
    }

    // The base message ends at "...waiting" with NO number — the live countdown
    // is appended once at render time from `cooldown_until` (single source), so
    // the seconds shown always update. `cooldown_until` must be set whatever the
    // cooldown duration, including zero.
    let cooldowns = [
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(0),
        std::time::Duration::from_secs(1),
    ];
    for cooldown in cooldowns {
        let DownloadEvent::BeatmapStatus {
            message,
            rate_limited,
            cooldown_until,
            ..
        } = drive_status(Status::RateLimited { cooldown })
        else {
            panic!("expected BeatmapStatus");
        };
        assert_eq!(message, "rate-limited on all mirrors, waiting");
        assert!(rate_limited);
        assert!(
            cooldown_until.is_some(),
            "rate-limited status must carry a cooldown deadline for the live countdown"
        );
    }
}

#[test]
fn only_newly_downloaded_hashes_are_included() {
    let dir = tempdir().unwrap();
    let collection = test_collection(
        1,
        vec![
            test_beatmapset(10, &["hash-a1", "hash-a2"]),
            test_beatmapset(20, &["hash-b1"]),
            test_beatmapset(30, &["hash-c1"]),
        ],
    );
    let selective = vec![make_selective(1, "my collection", vec![10, 20, 30])];
    let newly_downloaded: HashSet<u32> = [10].into_iter().collect();

    create_selective_collection_db(&collection, &selective, &newly_downloaded, dir.path()).unwrap();

    let list =
        osu_db::collection::CollectionList::from_file(dir.path().join("collection.db")).unwrap();
    assert_eq!(list.collections.len(), 1);
    let hashes: Vec<_> = list.collections[0]
        .beatmap_hashes
        .iter()
        .flatten()
        .collect();
    assert_eq!(hashes.len(), 2);
}

#[test]
fn rate_limited_status_forwards_cooldown_until() {
    let cooldown = std::time::Duration::from_secs(30);
    let before = std::time::Instant::now();
    let event = drive_status(Status::RateLimited { cooldown });
    let after = std::time::Instant::now();

    let DownloadEvent::BeatmapStatus {
        rate_limited,
        cooldown_until,
        ..
    } = event
    else {
        panic!("expected BeatmapStatus");
    };
    assert!(rate_limited);
    let deadline = cooldown_until.expect("cooldown_until must be Some for RateLimited");
    // remaining must be ≈ 30s — within a 1s tolerance for test execution time
    let remaining = deadline.saturating_duration_since(before);
    let upper = deadline.saturating_duration_since(after);
    assert!(
        remaining.as_secs() <= 30,
        "cooldown_until must not be more than 30s from now, got {remaining:?}"
    );
    assert!(
        upper.as_secs() >= 29,
        "cooldown_until must be at least 29s from start, got {upper:?}"
    );
}

#[test]
fn non_rate_limited_status_has_no_cooldown_until() {
    use osu_downloader::MirrorKind;
    let event = drive_status(Status::Contacting {
        mirror: mirror_ref_of(MirrorKind::Nerinyan),
    });
    let DownloadEvent::BeatmapStatus { cooldown_until, .. } = event else {
        panic!("expected BeatmapStatus");
    };
    assert!(cooldown_until.is_none());
}

// ── selective runs honor "skip already imported" ─────────────────────────────

const COLLECTION_ID: u32 = 1;
/// The ids the user checked in the browse — a part-picked collection.
const PICKED: [u32; 5] = [10, 20, 30, 40, 50];

/// Drive the real `prepare` over a part-picked collection. `owned` is what the
/// osu! library already holds (what `resolve_owned_ids` returns for a run with
/// the toggle on); every `on_disk` id gets a stub archive in the run's output
/// dir first, so `initial_satisfied` is fed by precheck *and* the owned fold
/// rather than one source that could mask the other going missing.
async fn prepare_selective_run(
    picked: &[u32],
    owned: HashSet<u32>,
    on_disk: &[u32],
    base_dir: &Path,
) -> (DownloadSession, Vec<DownloadEvent>) {
    let output_dir = base_dir.join(selective_folder_name(&[COLLECTION_ID]));
    std::fs::create_dir_all(&output_dir).expect("output dir");
    for id in on_disk {
        std::fs::write(output_dir.join(format!("{id}.osz")), b"stub archive bytes")
            .expect("existing archive");
    }

    let registry = ActiveDownloadRegistry::new();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let config = DownloadConfig {
        directory: base_dir.to_string_lossy().into_owned(),
        mirrors: Vec::new(),
        concurrent: 1,
        archive_validation: ArchiveValidation::Off,
        auto_skip_rate_limited: false,
        rate_limit_skip_secs: 60,
    };
    let payload = test_collection(
        COLLECTION_ID,
        picked
            .iter()
            .map(|&id| test_beatmapset(id, &["hash"]))
            .collect(),
    );
    let events = Mutex::new(Vec::new());
    let emit = |event: DownloadEvent| events.lock().unwrap().push(event);

    let session = DownloadSession::prepare(PrepareParams {
        id: 7,
        cancel_rx,
        config: &config,
        registry: &registry,
        emit: &emit,
        target: PrepareTarget::Selective {
            collection_ids: &[COLLECTION_ID],
            collections: vec![SelectiveDownloadCollection {
                id: COLLECTION_ID,
                name: String::new(),
                beatmapset_ids: picked.to_vec(),
            }],
            beatmapset_ids: picked,
            // Prefetched, so the resolve never reaches osu!collector.
            prefetched: HashMap::from([(COLLECTION_ID, payload)]),
        },
        overwrite: false,
        owned_ids: owned,
    })
    .await
    .expect("prepare must succeed")
    .expect("prepare must not abort");

    (session, events.into_inner().unwrap())
}

/// The `N queued` figure on the run page's tally line.
fn queued(events: &[DownloadEvent]) -> usize {
    events
        .iter()
        .find_map(|event| match event {
            DownloadEvent::DownloadTarget { remaining, .. } => Some(*remaining),
            _ => None,
        })
        .expect("prepare must announce DownloadTarget")
}

/// A part-picked collection half-present in the osu! library pre-skips the owned
/// half, the way the whole-collection and raw-ids arms already do — the toggle
/// no longer changes meaning because one map is unchecked.
#[tokio::test]
async fn selective_run_pre_skips_library_owned_sets() {
    let dir = tempdir().unwrap();
    let (session, events) =
        prepare_selective_run(&PICKED, HashSet::from([20, 40]), &[50], dir.path()).await;

    assert_eq!(session.pending_ids, vec![10, 30]);
    assert_eq!(session.skipped_owned, 2);
    assert_eq!(session.skipped_existing, 1);
    // Owned sets join the on-disk one in `satisfied`: they reach the selective
    // `collection.db` and count toward the snapshot gate without being fetched.
    assert_eq!(session.initial_satisfied, HashSet::from([20, 40, 50]));
    assert_eq!(queued(&events), 2);
}

/// Positive control: the same fixture with the owned set empty — the one
/// dimension that differs — enqueues every picked id not already on disk. A
/// green here against a red above isolates the pre-skip from the fixture.
#[tokio::test]
async fn selective_run_without_owned_sets_enqueues_every_pick() {
    let dir = tempdir().unwrap();
    let (session, events) = prepare_selective_run(&PICKED, HashSet::new(), &[50], dir.path()).await;

    assert_eq!(session.pending_ids, vec![10, 20, 30, 40]);
    assert_eq!(session.skipped_owned, 0);
    assert_eq!(session.skipped_existing, 1);
    assert_eq!(session.initial_satisfied, HashSet::from([50]));
    assert_eq!(queued(&events), 4);
}

// ── size-total denominator excludes owned-only pre-skips ────────────────────

/// An owned-only pre-skip (owned by the library, never verified on disk by
/// precheck) never earns a byte figure: it is never downloaded
/// (`bytes_downloaded`) and precheck never sized it (`verified_bytes`). The
/// size-fetch target list must exclude it, or the run's byte total counts a
/// map its own numerator can never reach and the progress bar stalls short of
/// full.
#[tokio::test]
async fn size_target_ids_excludes_owned_only_pre_skips() {
    let dir = tempdir().unwrap();
    let (session, _events) =
        prepare_selective_run(&PICKED, HashSet::from([20, 40]), &[50], dir.path()).await;

    assert_eq!(session.size_target_ids, vec![10, 30, 50]);
}

/// Positive control isolating the one case that could make the exclusion
/// over-broad: an id that is BOTH owned by the library AND already
/// precheck-verified on disk. It must stay in the size total — precheck
/// already counted its bytes in `verified_bytes`, so dropping it here would
/// swing the defect the other way (the total falling below what the numerator
/// can report).
#[tokio::test]
async fn size_target_ids_keeps_an_owned_id_precheck_already_verified() {
    let dir = tempdir().unwrap();
    let (session, _events) =
        prepare_selective_run(&PICKED, HashSet::from([20, 40]), &[40, 50], dir.path()).await;

    assert_eq!(session.size_target_ids, vec![10, 30, 40, 50]);
}

/// Defect B's only production-visible effect is what `run_pipeline_core`
/// (pipeline.rs:601) hands `fetch_collection_sizes`, not the `DownloadSession`
/// field alone. Drives the real `run_ids` with `known_sizes` covering every id
/// (so `fetch_collection_sizes` takes its network-free `unknown.is_empty()`
/// branch — mod.rs:406-412) and one id owned-only.
///
/// `run_ids` also reconciles the failed-maps store (pipeline.rs:482,
/// `reconcile_failed_maps` → `failed_maps::reconcile`) once it returns, and
/// `initial_satisfied` here is non-empty (the owned fold), so that early return
/// does not apply — the run must never be allowed to touch the real
/// `~/.local/share/osu-collect/failed-beatmapsets.json`, hence the second env
/// override below (matching `selective_run_pre_skips_owned_and_then_persists_its_snapshot`)
/// and the `!failed_maps.exists()` assertion pinning that it never ran.
///
/// The wait is bounded by a short ceiling rather than run to completion: with
/// `pending_ids` non-empty the run goes on to attempt a real connection against
/// a refused mirror and (per `osu-downloader`'s `batch.rs:526-573` retry loop,
/// `NETWORK_RETRY_BACKOFF` = 5s at `config.rs:10`) backs off 5s per attempt for
/// up to 1000 attempts on a `NetworkError`, so it cannot finish inside any
/// test-sized budget. `CollectionSizeResolved` fires from a detached
/// `tokio::spawn` with zero I/O (the known-sizes branch) well before the first
/// connect attempt even resolves, so polling for it and aborting the outer task
/// the moment it arrives keeps the typical run in the low milliseconds; the 3s
/// bound only catches a genuine regression.
#[tokio::test]
async fn size_fetch_call_site_excludes_owned_only_ids() {
    let dir = tempdir().unwrap();
    let (install_dir, cache_path) = seed_owned_library(dir.path(), &[20]);
    let failed_maps = dir.path().join("failed-beatmapsets.json");
    let _env = crate::test_env::TempEnvVar::set_all([
        (
            crate::app::library_cache::LIBRARY_CACHE_ENV_PATH,
            cache_path.to_str().unwrap(),
        ),
        (
            crate::app::failed_maps::FAILED_MAPS_ENV_PATH,
            failed_maps.to_str().unwrap(),
        ),
    ]);

    let config = DownloadConfig {
        directory: dir.path().to_string_lossy().into_owned(),
        mirrors: vec![
            osu_downloader::Mirror::custom("http://127.0.0.1:1/{id}").expect("custom mirror"),
        ],
        concurrent: 1,
        archive_validation: ArchiveValidation::Off,
        auto_skip_rate_limited: false,
        rate_limit_skip_secs: 60,
    };
    let request = crate::download::IdsDownloadRequest {
        beatmapset_ids: vec![10, 20, 30],
        label: "size fetch test".to_string(),
        folder_tag: "size-fetch".to_string(),
        source: crate::download::IdsRunSource::Search,
        config,
        auto_overwrite: false,
        skip_already_imported: true,
        osu_client: OsuClient::Stable,
        osu_path: install_dir.to_string_lossy().into_owned(),
        known_sizes: HashMap::from([(10, 1_000_000), (20, 2_000_000), (30, 3_000_000)]),
    };

    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let (_defer_tx, defer_rx) = watch::channel(0u64);
    let (_skip_tx, skip_rx) = watch::channel(0u64);
    let events: Arc<Mutex<Vec<DownloadEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let emit: Arc<dyn Fn(DownloadEvent) + Send + Sync> =
        Arc::new(move |event| sink.lock().unwrap().push(event));

    let start = std::time::Instant::now();
    let handle = tokio::spawn(super::run_ids(
        7, request, cancel_rx, defer_rx, skip_rx, emit,
    ));
    let poll_events = Arc::clone(&events);
    let total_bytes = tokio::time::timeout(std::time::Duration::from_secs(3), async move {
        loop {
            let found = poll_events
                .lock()
                .unwrap()
                .iter()
                .find_map(|event| match event {
                    DownloadEvent::CollectionSizeResolved { total_bytes, .. } => Some(*total_bytes),
                    _ => None,
                });
            if let Some(total_bytes) = found {
                break total_bytes;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "CollectionSizeResolved never arrived within {:?} — \
             network-touching path unpinnable at HEAD within a bounded timeout",
            start.elapsed()
        )
    });
    // Stop driving the run the moment the size event lands rather than waiting
    // out the timeout: the retry loop it would otherwise be stuck in never lets
    // it reach `reconcile_failed_maps` regardless, but this keeps the typical
    // run in the low milliseconds instead of a flat 3s.
    handle.abort();
    let elapsed = start.elapsed();

    assert_eq!(
        total_bytes, 4_000_000,
        "size total must exclude the owned-only id's known size (2_000_000); got \
         {total_bytes} after {elapsed:?}"
    );
    // The env override above is the only thing standing between this run and
    // the developer's real failed-maps file; pin that it was never reached
    // rather than assume it from the retry-loop timing.
    assert!(
        !failed_maps.exists(),
        "run_ids must not have reached reconcile_failed_maps — it would write \
         the real failed-maps file without the env override in effect"
    );
}

/// `30` is neither owned, on disk, nor downloaded, so the snapshot gate must
/// still name it. Miss this and the run writes a snapshot claiming the
/// collection is in hand, and the next scan stops reporting `30` as missing.
#[tokio::test]
async fn snapshot_gate_reports_the_target_a_run_left_behind() {
    let dir = tempdir().unwrap();
    let (session, _events) =
        prepare_selective_run(&PICKED, HashSet::from([20, 40]), &[50], dir.path()).await;
    // Of the two it enqueued the run landed `10`; `30` did not arrive.
    let (tally, _) = drive_translate(vec![completed(10)]);

    let verified = verified_targets(&session.initial_satisfied, &tally);
    assert_eq!(verified, HashSet::from([10, 20, 40, 50]));
    assert_eq!(
        unsatisfied_targets(&session.beatmapset_ids, &verified),
        vec![30]
    );
}

/// Same fixture and same owned set — only what the run downloaded differs. Every
/// target is now in the user's hands by one of the three routes (library-owned,
/// already on disk, downloaded now), so the gate clears and snapshots persist.
#[tokio::test]
async fn snapshot_gate_clears_once_every_target_is_in_hand() {
    let dir = tempdir().unwrap();
    let (session, _events) =
        prepare_selective_run(&PICKED, HashSet::from([20, 40]), &[50], dir.path()).await;
    let (tally, _) = drive_translate(vec![completed(10), completed(30)]);

    let verified = verified_targets(&session.initial_satisfied, &tally);
    assert_eq!(verified, HashSet::from([10, 20, 30, 40, 50]));
    assert_eq!(
        unsatisfied_targets(&session.beatmapset_ids, &verified),
        Vec::<u32>::new()
    );
}

// ── run_selective end to end: the owned-id join and the snapshot gate ────────

/// Seed the library cache so `resolve_owned_ids` returns `owned` without parsing
/// a real osu! database: `owned_ids_cached_with` serves a hit purely on the db
/// file's path + mtime matching the cached entry, so a stub `osu!.db` plus a
/// matching `library-cache.json` is enough. Returns the install dir to hand the
/// request and the cache path to point `OSU_COLLECT_LIBRARY_CACHE` at.
fn seed_owned_library(base: &Path, owned: &[u32]) -> (PathBuf, PathBuf) {
    let install_dir = base.join("osu-install");
    std::fs::create_dir_all(&install_dir).expect("install dir");
    let db_path = crate::app::library_cache::db_file_path(OsuClient::Stable, &install_dir);
    std::fs::write(&db_path, b"stub osu!.db").expect("stub db");

    let mtime_ns = std::fs::metadata(&db_path)
        .and_then(|meta| meta.modified())
        .expect("stub db mtime")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("mtime after epoch")
        .as_nanos();
    let cache_path = base.join("library-cache.json");
    let cache = crate::app::library_cache::LibraryCacheFile {
        schema_version: 1,
        db_path: db_path.to_string_lossy().into_owned(),
        mtime_ns,
        beatmapset_ids: owned.to_vec(),
    };
    std::fs::write(
        &cache_path,
        serde_json::to_string(&cache).expect("serialize cache"),
    )
    .expect("write cache");

    (install_dir, cache_path)
}

/// A whole `run_selective` over a part-picked collection whose every target is
/// already in the osu! library. Every id owned means `pending_ids` is empty, so
/// `run_pipeline_core` returns before it builds a `Downloader` or spawns the
/// size probe — the run is hermetic, no socket is opened.
///
/// This is the only pin on the `resolve_owned_ids` → `PrepareParams.owned_ids`
/// join. It also covers the snapshot gate's satisfied arm: with nothing left
/// behind, the snapshot file must appear on disk.
#[tokio::test]
async fn selective_run_pre_skips_owned_and_then_persists_its_snapshot() {
    let dir = tempdir().unwrap();
    let (install_dir, cache_path) = seed_owned_library(dir.path(), &[10, 20]);
    let failed_maps = dir.path().join("failed-beatmapsets.json");
    let snapshot_dir = dir.path().join("snapshots");
    std::fs::create_dir_all(&snapshot_dir).expect("snapshot dir");

    // One guard, two keys: the global env lock is not reentrant, so two `set`
    // guards would deadlock rather than nest.
    let _env = crate::test_env::TempEnvVar::set_all([
        (
            crate::app::library_cache::LIBRARY_CACHE_ENV_PATH,
            cache_path.to_str().unwrap(),
        ),
        (
            crate::app::failed_maps::FAILED_MAPS_ENV_PATH,
            failed_maps.to_str().unwrap(),
        ),
    ]);

    let events = run_selective_fixture(&install_dir, dir.path(), Some(snapshot_dir.clone())).await;

    assert_eq!(
        skipped_imported(&events),
        Some(2),
        "both targets are library-owned, so the run must pre-skip both"
    );
    assert!(
        snapshot_dir.join("collection-1.json").exists(),
        "every target is in hand, so the run must persist its snapshot"
    );
}

/// Pull the `SkippedImported` count out of a run's events.
fn skipped_imported(events: &[DownloadEvent]) -> Option<usize> {
    events.iter().find_map(|event| match event {
        DownloadEvent::SkippedImported { count, .. } => Some(*count),
        _ => None,
    })
}

/// Drive the real `run_selective` for collection 1 over picks `[10, 20]`.
/// The mirror template is never contacted (nothing is left to download); it
/// exists only because `run_pipeline_core` rejects an empty mirror list before
/// it reaches the fully-satisfied early return.
async fn run_selective_fixture(
    install_dir: &Path,
    base_dir: &Path,
    snapshot_dir: Option<PathBuf>,
) -> Vec<DownloadEvent> {
    let config = DownloadConfig {
        directory: base_dir.to_string_lossy().into_owned(),
        mirrors: vec![
            osu_downloader::Mirror::custom("http://127.0.0.1:1/{id}").expect("custom mirror"),
        ],
        concurrent: 1,
        archive_validation: ArchiveValidation::Off,
        auto_skip_rate_limited: false,
        rate_limit_skip_secs: 60,
    };
    let request = crate::download::SelectiveDownloadRequest {
        collection_ids: vec![COLLECTION_ID],
        beatmapset_ids: vec![10, 20],
        collections: vec![SelectiveDownloadCollection {
            id: COLLECTION_ID,
            name: String::new(),
            beatmapset_ids: vec![10, 20],
        }],
        config,
        snapshot_dir,
        snapshots: vec![crate::app::snapshots::CollectionSnapshotFile {
            collection_id: COLLECTION_ID.to_string(),
            name: "alpha".to_string(),
            last_run_at: "2026-08-01T00:00:00Z".to_string(),
            snapshot: crate::app::snapshots::CollectionSnapshot::default(),
            version: 1,
        }],
        skip_already_imported: true,
        osu_client: OsuClient::Stable,
        osu_path: install_dir.to_string_lossy().into_owned(),
        prefetched: HashMap::from([(
            COLLECTION_ID,
            test_collection(
                COLLECTION_ID,
                vec![
                    test_beatmapset(10, &["hash"]),
                    test_beatmapset(20, &["hash"]),
                ],
            ),
        )]),
    };

    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let (_defer_tx, defer_rx) = watch::channel(0u64);
    let (_skip_tx, skip_rx) = watch::channel(0u64);
    let events: Arc<Mutex<Vec<DownloadEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let emit: Arc<dyn Fn(DownloadEvent) + Send + Sync> =
        Arc::new(move |event| sink.lock().unwrap().push(event));

    // Bounded on purpose. At HEAD every target is owned, so `pending_ids` is
    // empty and this returns in milliseconds without opening a socket. Break the
    // owned-id join and the same run instead builds a `Downloader` and spawns the
    // nekoha size probe, which retries and stalls for minutes — a hang that reads
    // as an infrastructure problem rather than as this assertion failing. The
    // timeout turns that back into a loud, fast red.
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        super::run_selective(7, request, cancel_rx, defer_rx, skip_rx, emit),
    )
    .await
    .expect("a fully-owned selective run must not reach the network")
    .expect("a fully-owned selective run must succeed");

    events.lock().unwrap().clone()
}

/// The gate's unsatisfied arm, pinned on the file it does or does not write.
///
/// Driven here rather than through `run_selective` because a gate test must
/// make no outbound request: an unsatisfied target is by definition still
/// pending, and a non-empty `pending_ids` spawns `fetch_collection_sizes`
/// detached, which with `known_sizes` empty for a Selective target fires a real
/// nekoha request that outlives the test. Every network-touching test in this
/// repo is `#[ignore]`d, which would leave this arm unpinned in the gate.
#[tokio::test]
async fn snapshot_gate_writes_nothing_when_a_target_is_left_behind() {
    let dir = tempdir().unwrap();
    let files = vec![crate::app::snapshots::CollectionSnapshotFile {
        collection_id: COLLECTION_ID.to_string(),
        name: "alpha".to_string(),
        last_run_at: "2026-08-01T00:00:00Z".to_string(),
        snapshot: crate::app::snapshots::CollectionSnapshot::default(),
        version: 1,
    }];

    // 30 was targeted and never verified — the run fell short.
    super::persist_snapshots_if_complete(
        Some(dir.path().to_path_buf()),
        files,
        &[10, 20, 30],
        &HashSet::from([10, 20]),
    )
    .await
    .expect("the gate itself must not fail");

    assert!(
        !dir.path().join("collection-1.json").exists(),
        "a run that left a target behind must write no snapshot"
    );
}

/// Positive control for the test above: the same call with the same snapshot
/// file, varying only whether the last target is verified. Without this, a
/// broken fixture (unwritable dir, wrong filename) would read as a passing gate.
#[tokio::test]
async fn snapshot_gate_writes_the_file_when_every_target_is_verified() {
    let dir = tempdir().unwrap();
    let files = vec![crate::app::snapshots::CollectionSnapshotFile {
        collection_id: COLLECTION_ID.to_string(),
        name: "alpha".to_string(),
        last_run_at: "2026-08-01T00:00:00Z".to_string(),
        snapshot: crate::app::snapshots::CollectionSnapshot::default(),
        version: 1,
    }];

    super::persist_snapshots_if_complete(
        Some(dir.path().to_path_buf()),
        files,
        &[10, 20, 30],
        &HashSet::from([10, 20, 30]),
    )
    .await
    .expect("the gate itself must not fail");

    assert!(
        dir.path().join("collection-1.json").exists(),
        "every target verified, so the snapshot must be written"
    );
}

fn failure_reason_for(error: osu_downloader::Error) -> FailureReason {
    let (tally, _events) = drive_translate(vec![LibEvent::BeatmapsetFailed {
        beatmapset_id: 99,
        error,
        mirror: None,
    }]);
    tally
        .failures
        .into_iter()
        .find(|f| f.beatmapset_id == 99)
        .expect("failure recorded")
        .reason
}

#[test]
fn not_found_error_maps_to_not_found_reason() {
    assert_eq!(
        failure_reason_for(osu_downloader::Error::NotFound),
        FailureReason::NotFound
    );
}

#[test]
fn rate_limited_error_maps_to_rate_limited_reason() {
    assert_eq!(
        failure_reason_for(osu_downloader::Error::RateLimited { retry_after: None }),
        FailureReason::RateLimited
    );
}

#[test]
fn network_error_maps_to_network_error_reason() {
    assert_eq!(
        failure_reason_for(osu_downloader::Error::Network("connection refused".into())),
        FailureReason::NetworkError
    );
}

#[test]
fn timeout_maps_to_network_error_reason() {
    assert_eq!(
        failure_reason_for(osu_downloader::Error::Timeout),
        FailureReason::NetworkError
    );
}

#[test]
fn validation_error_maps_to_validation_failed_reason() {
    assert_eq!(
        failure_reason_for(osu_downloader::Error::Validation(
            "checksum mismatch".into()
        )),
        FailureReason::ValidationFailed
    );
}

#[test]
fn unavailable_on_mirrors_maps_to_not_found_reason() {
    let (tally, _events) = drive_translate(vec![LibEvent::BeatmapsetSkipped {
        beatmapset_id: 7,
        reason: Skip::UnavailableOnMirrors,
    }]);
    let reason = tally
        .failures
        .into_iter()
        .find(|f| f.beatmapset_id == 7)
        .expect("failure recorded")
        .reason;
    assert_eq!(reason, FailureReason::NotFound);
}

// ── the run-target → snapshot-gate → next-scan seam ───────────────────────────

/// Held-back sets are removed from a run's target list, so the completion gate
/// cannot withhold the snapshot on their behalf: nothing that is not a target can
/// ever be unsatisfied. Without `retain_held_back_in_snapshots` the baseline a
/// finished run writes is the current LOCAL library, which omits the deleted set,
/// and the next scan reads that as "no longer deleted" and re-fetches it — the
/// exact defect the hold-back exists to prevent, one run later.
///
/// Nothing else in the suite joins those two sides: the gate's own tests pass
/// literal id slices, and the scan's tests build diffs by hand. This drives the
/// real chain — `request_selective_download` → `persist_snapshots_if_complete` →
/// the file on disk → `snapshot_diffs_for_scan` → `missing_from_candidate`.
#[tokio::test]
async fn a_completed_run_leaves_a_held_back_set_still_held_back_on_the_next_scan() {
    use crate::app::App;
    use crate::app::runtime::scan::{CollectionBeatmapset, missing_from_candidate};
    use crate::app::runtime::snapshot_diffs_for_scan;
    use crate::app::snapshots::{self, CollectionSnapshot, CollectionSnapshotFile, SnapshotDiff};
    use crate::app::update_source::{MissingBeatmapset, MissingStatus};
    use crate::config::Config;
    use crate::osu_db::{LocalBeatmap, LocalBeatmapset, LocalCollection, checksum};

    fn md5(seed: u8) -> crate::osu_db::Md5 {
        let mut out = [0u8; 16];
        out[0] = seed;
        out
    }

    // Collection 100: A is installed, B is new upstream, M the user deleted.
    let (a, b, m) = (md5(0xa1), md5(0xb2), md5(0xcc));
    let snapshot_dir = tempdir().expect("temp dir");
    let dir = snapshot_dir.path().to_path_buf();

    // The baseline from before the deletion still names M — that is what makes
    // this scan classify M as manually deleted.
    snapshots::save(
        &CollectionSnapshotFile::new(
            100,
            "coll - 100".to_string(),
            CollectionSnapshot {
                stable_hashes: vec![checksum::to_hex(a), checksum::to_hex(m)],
                lazer_ids: Vec::new(),
            },
        ),
        &snapshots::snapshot_path(&dir, 100),
    );

    // `App::new` also reads two other on-disk stores at construction
    // (docs/architecture.md § "On-disk stores"): the stored auth (via
    // `ConfigTab::new` → `auth::load`) and `collection_state.toml` (via
    // `collection_state::state_path`/`load`). Both must be isolated too, or
    // this test inherits the developer's real login state and collection
    // state — the exact class of bug that section's incident describes.
    // The auth fixture is a genuine logged-in supporter (not an absent file),
    // proving the test's outcome does not depend on that content either way.
    let auth_path = dir.join("auth.json");
    std::fs::write(
        &auth_path,
        serde_json::to_string(&crate::auth::StoredAuth {
            client_id: "5".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: String::new(),
            access_token: "token".to_string(),
            refresh_token: None,
            expires_at: u64::MAX,
            scopes: vec!["*".to_string()],
            supporter: Some(true),
        })
        .expect("serialize stub auth"),
    )
    .expect("write stub auth.json");
    let state_path = dir.join("collection_state.toml");
    let _env = crate::test_env::TempEnvVar::set_all([
        (
            snapshots::SNAPSHOT_ENV_DIR,
            dir.to_str().expect("utf-8 temp path"),
        ),
        (crate::auth::AUTH_ENV_PATH, auth_path.to_str().unwrap()),
        (
            crate::app::collection_state::STATE_ENV_PATH,
            state_path.to_str().unwrap(),
        ),
    ]);

    let mut app = App::new(Config::default());
    // The app defaults to lazer, whose snapshot is keyed by set id; this fixture
    // exercises the stable representation, where a set id cannot be re-expressed
    // in the baseline at all and the checksums carried on the missing set are the
    // only way back in.
    app.library.client_type = OsuClient::Stable;
    app.home.directory.value = snapshot_dir
        .path()
        .join("out")
        .to_string_lossy()
        .into_owned();
    // Local library: the collection holds A only — M is gone, B not yet fetched.
    app.home.update.set_collections(vec![LocalCollection {
        name: "coll - 100".to_string(),
        beatmap_checksums: Box::new([a]),
    }]);
    app.home.update.set_local_beatmapsets(vec![LocalBeatmapset {
        id: 1,
        beatmaps: Box::new([LocalBeatmap { checksum: a }]),
    }]);
    app.home.update.set_missing_beatmaps(
        vec![
            MissingBeatmapset {
                id: 2,
                status: MissingStatus::NotInstalled,
                collection_id: 100,
                collection_name: "coll - 100".to_string(),
                included: true,
                previously_deleted: false,
                checksums: Box::new([b]),
                enrich_diff_id: None,
            },
            MissingBeatmapset {
                id: 3,
                status: MissingStatus::NotInstalled,
                collection_id: 100,
                collection_name: "coll - 100".to_string(),
                included: false,
                previously_deleted: true,
                checksums: Box::new([m]),
                enrich_diff_id: None,
            },
        ],
        &HashMap::new(),
    );
    // The scan normally computes the diff (old baseline vs local lib) and stores
    // it for the request path. This test bypasses the scan, so set it by hand:
    // M is in the old baseline, absent locally, so it is manually deleted.
    app.home.update.scan.snapshot_diffs = HashMap::from([(
        100,
        SnapshotDiff {
            manually_deleted: CollectionSnapshot {
                stable_hashes: vec![checksum::to_hex(m)],
                lazer_ids: Vec::new(),
            },
            manually_added: CollectionSnapshot::default(),
        },
    )]);

    let (_, request) = app
        .request_selective_download()
        .expect("a mixed collection with mirrors enabled builds a request");
    assert_eq!(
        request.beatmapset_ids,
        vec![2],
        "precondition: M is NOT a run target, which is why the gate cannot protect it"
    );
    // The per-collection payload is a second id list on the same request. It is
    // neutralised downstream by two intersections today, so only a direct
    // assertion can keep it from drifting back into a superset of the run.
    assert_eq!(
        request.collections[0].beatmapset_ids,
        vec![2],
        "the per-collection payload carries the run's set, not every missing set"
    );

    // B downloads and verifies; M never does, because it was never requested.
    let verified: HashSet<u32> = HashSet::from([2]);
    super::persist_snapshots_if_complete(
        request.snapshot_dir.clone(),
        request.snapshots.clone(),
        &request.beatmapset_ids,
        &verified,
    )
    .await
    .expect("snapshot persist must not fail");

    // Next scan: the collection now holds A and B locally (B was just imported).
    let next_local = vec![LocalCollection {
        name: "coll - 100".to_string(),
        beatmap_checksums: Box::new([a, b]),
    }];
    let next_snapshots =
        snapshots::current_snapshots(OsuClient::Stable, &next_local, [].iter(), |_| Some(100));
    let diffs = snapshot_diffs_for_scan(&dir, &[100], &next_snapshots);

    let rebuilt = missing_from_candidate(
        &CollectionBeatmapset {
            id: 3,
            checksums: vec![m],
            enrich_diff_id: None,
        },
        100,
        "coll - 100".to_string(),
        OsuClient::Stable,
        &diffs,
    );
    assert!(
        rebuilt.previously_deleted,
        "the next scan must still see M as deleted, or the run silently re-fetches it"
    );
    assert!(!rebuilt.included, "so it stays held back");

    // The set that DID download is not a deletion — the rebuild must re-add the
    // held-back set only, never blanket-restore the pre-run baseline.
    let b_rebuilt = missing_from_candidate(
        &CollectionBeatmapset {
            id: 2,
            checksums: vec![b],
            enrich_diff_id: None,
        },
        100,
        "coll - 100".to_string(),
        OsuClient::Stable,
        &diffs,
    );
    assert!(
        !b_rebuilt.previously_deleted,
        "B is present locally, so it is not a deletion"
    );

    // The written baseline is the pre-run local membership plus M — exactly one
    // entry re-added, so the rebuild cannot be passing by restoring everything.
    let written = snapshots::load(&snapshots::snapshot_path(&dir, 100)).expect("snapshot written");
    assert_eq!(
        written.snapshot.stable_hashes,
        vec![checksum::to_hex(a), checksum::to_hex(m)],
        "baseline is the pre-run local membership plus the held-back set, nothing else"
    );
}
