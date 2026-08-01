use super::{
    DownloadSession, OutputPreparation, PrepareParams, PrepareTarget, SessionTarget,
    ids_folder_name, partition_pending, resolve_selective_with,
};
use crate::core::collection::{Beatmap, Beatmapset, Collection, CollectionService, Uploader};
use crate::download::IdsRunSource;
use crate::download::{
    ActiveDownloadRegistry, ArchiveValidation, DownloadConfig, DownloadEvent,
    SelectiveDownloadCollection,
};
use crate::utils;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

struct MockService {
    responses: Vec<(u32, Result<Collection, &'static str>)>,
}

impl CollectionService for MockService {
    async fn fetch_collection(&self, id: u32) -> utils::Result<Collection> {
        let response = self
            .responses
            .iter()
            .find(|(cid, _)| *cid == id)
            .map(|(_, r)| r.clone())
            .unwrap_or(Err("missing"));
        response.map_err(utils::AppError::other)
    }
}

fn beatmapset(id: u32) -> Beatmapset {
    Beatmapset {
        id,
        beatmaps: vec![Beatmap {
            id,
            checksum: "abc".into(),
        }],
    }
}

fn collection(id: u32, name: &str, ids: &[u32]) -> Collection {
    Collection {
        id,
        name: name.to_string(),
        description: None,
        uploader: Uploader {
            id: 0,
            username: "u".to_string(),
        },
        beatmapsets: ids.iter().copied().map(beatmapset).collect(),
        favourites: 0,
    }
}

#[tokio::test]
async fn resolve_selective_dedupes_overlapping_beatmapsets() {
    let service = MockService {
        responses: vec![
            (1, Ok(collection(1, "alpha", &[10, 11]))),
            (2, Ok(collection(2, "beta", &[10, 12]))),
        ],
    };
    let requested = vec![
        SelectiveDownloadCollection {
            id: 1,
            name: String::new(),
            beatmapset_ids: vec![10, 11],
        },
        SelectiveDownloadCollection {
            id: 2,
            name: String::new(),
            beatmapset_ids: vec![10, 12],
        },
    ];
    let emit = |_event| {};
    let (selected, resolved, names) = resolve_selective_with(
        &service,
        &[1, 2],
        requested,
        &[10, 11, 12],
        &HashMap::new(),
        7,
        &emit,
    )
    .await
    .expect("resolve must succeed");

    let mut bs_ids: Vec<u32> = selected.beatmapsets.iter().map(|b| b.id).collect();
    bs_ids.sort_unstable();
    assert_eq!(bs_ids, vec![10, 11, 12]);
    assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    assert_eq!(resolved.len(), 2);
}

#[tokio::test]
async fn resolve_selective_progress_is_monotonic() {
    use std::time::Duration;
    use tokio::time::sleep;

    struct DelayedService {
        responses: Vec<(u32, Collection, Duration)>,
    }
    impl CollectionService for DelayedService {
        async fn fetch_collection(&self, id: u32) -> utils::Result<Collection> {
            let (_, ref c, delay) = *self
                .responses
                .iter()
                .find(|(cid, _, _)| *cid == id)
                .unwrap();
            sleep(delay).await;
            Ok(c.clone())
        }
    }

    let service = DelayedService {
        responses: vec![
            (1, collection(1, "alpha", &[10]), Duration::from_millis(60)),
            (2, collection(2, "beta", &[11]), Duration::from_millis(10)),
            (3, collection(3, "gamma", &[12]), Duration::from_millis(30)),
        ],
    };
    let requested = vec![
        SelectiveDownloadCollection {
            id: 1,
            name: String::new(),
            beatmapset_ids: vec![10],
        },
        SelectiveDownloadCollection {
            id: 2,
            name: String::new(),
            beatmapset_ids: vec![11],
        },
        SelectiveDownloadCollection {
            id: 3,
            name: String::new(),
            beatmapset_ids: vec![12],
        },
    ];
    let events = Arc::new(Mutex::new(Vec::<u32>::new()));
    let events_inner = Arc::clone(&events);
    let emit = move |event: DownloadEvent| {
        if let DownloadEvent::ResolveProgress { current, .. } = event {
            events_inner.lock().unwrap().push(current);
        }
    };

    resolve_selective_with(
        &service,
        &[1, 2, 3],
        requested,
        &[10, 11, 12],
        &HashMap::new(),
        7,
        &emit,
    )
    .await
    .expect("resolve must succeed");

    let observed = events.lock().unwrap().clone();
    assert_eq!(observed, vec![0, 1, 2, 3]);
}

/// A prefetched collection (the scan already fetched it) skips the refetch, while
/// the progress ticks still advance 0..N so the UI gauge is unaffected.
#[tokio::test]
async fn resolve_selective_skips_the_fetch_for_a_prefetched_collection() {
    struct RecordingService {
        responses: Vec<(u32, Collection)>,
        fetched: Arc<Mutex<Vec<u32>>>,
    }
    impl CollectionService for RecordingService {
        async fn fetch_collection(&self, id: u32) -> utils::Result<Collection> {
            self.fetched.lock().unwrap().push(id);
            self.responses
                .iter()
                .find(|(cid, _)| *cid == id)
                .map(|(_, c)| c.clone())
                .ok_or_else(|| utils::AppError::other("missing"))
        }
    }

    let fetched = Arc::new(Mutex::new(Vec::new()));
    let service = RecordingService {
        responses: vec![(2, collection(2, "beta", &[11]))],
        fetched: Arc::clone(&fetched),
    };
    let requested = vec![
        SelectiveDownloadCollection {
            id: 1,
            name: String::new(),
            beatmapset_ids: vec![10],
        },
        SelectiveDownloadCollection {
            id: 2,
            name: String::new(),
            beatmapset_ids: vec![11],
        },
    ];
    let prefetched = HashMap::from([(1, collection(1, "alpha", &[10]))]);
    let progress = Arc::new(Mutex::new(Vec::<u32>::new()));
    let progress_inner = Arc::clone(&progress);
    let emit = move |event: DownloadEvent| {
        if let DownloadEvent::ResolveProgress { current, .. } = event {
            progress_inner.lock().unwrap().push(current);
        }
    };

    let (selected, resolved, names) = resolve_selective_with(
        &service,
        &[1, 2],
        requested,
        &[10, 11],
        &prefetched,
        7,
        &emit,
    )
    .await
    .expect("resolve must succeed");

    // Collection 1 came from the cache; only 2 hit the network.
    assert_eq!(*fetched.lock().unwrap(), vec![2]);
    // The cached payload is used in full: its name and its beatmapsets.
    assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    let mut bs_ids: Vec<u32> = selected.beatmapsets.iter().map(|b| b.id).collect();
    bs_ids.sort_unstable();
    assert_eq!(bs_ids, vec![10, 11]);
    assert_eq!(resolved.len(), 2);
    // The gauge still ticks once per collection, cached or not.
    let mut ticks = progress.lock().unwrap().clone();
    ticks.sort_unstable();
    assert_eq!(ticks, vec![0, 1, 2]);
}

// ── declined-retry exclusion: what the run actually enqueues ─────────────────

/// Run the real `prepare` over a prefetched collection into a fresh output dir
/// (nothing on disk, so precheck satisfies nothing and `pending_ids` is exactly
/// the run's target list). Returns the session plus every event it announced.
async fn prepare_collection_run(
    collection: Collection,
    skip_previously_failed: &HashSet<u32>,
    base_dir: &Path,
) -> (DownloadSession, Vec<DownloadEvent>) {
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
    let events = Mutex::new(Vec::new());
    let emit = |event: DownloadEvent| events.lock().unwrap().push(event);

    let session = DownloadSession::prepare(PrepareParams {
        id: 7,
        cancel_rx,
        config: &config,
        registry: &registry,
        emit: &emit,
        target: PrepareTarget::Collection {
            collection_input: "",
            prefetched: Some(collection),
            skip_previously_failed,
        },
        overwrite: false,
        owned_ids: HashSet::new(),
    })
    .await
    .expect("prepare must succeed")
    .expect("prepare must not abort");

    (session, events.into_inner().unwrap())
}

/// The map count the run page shows (and divides its gauge by).
fn announced_total(events: &[DownloadEvent]) -> usize {
    events
        .iter()
        .find_map(|event| match event {
            DownloadEvent::CollectionReady { total_maps, .. } => Some(*total_maps),
            _ => None,
        })
        .expect("prepare must announce CollectionReady")
}

/// The `N queued` figure on the run page's tally line.
fn announced_queued(events: &[DownloadEvent]) -> usize {
    events
        .iter()
        .find_map(|event| match event {
            DownloadEvent::DownloadTarget { remaining, .. } => Some(*remaining),
            _ => None,
        })
        .expect("prepare must announce DownloadTarget")
}

/// `skip` on the pre-download retry prompt: the declined sets leave the run
/// entirely — not enqueued, not in the announced total, not in the queued count.
/// The collection payload keeps them, so `collection.db` still records all four.
#[tokio::test]
async fn declined_retry_drops_previously_failed_from_the_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let skipped = HashSet::from([10, 20]);
    let (session, events) = prepare_collection_run(
        collection(1, "alpha", &[10, 20, 30, 40]),
        &skipped,
        dir.path(),
    )
    .await;

    assert_eq!(
        session.pending_ids,
        vec![30, 40],
        "a declined-retry set must never be enqueued"
    );
    assert_eq!(session.beatmapset_ids, vec![30, 40]);
    // Every count the page shows describes only what the run enqueues.
    assert_eq!(announced_total(&events), 2);
    assert_eq!(announced_queued(&events), 2);
    // …while collection.db still gets the whole collection.
    let recorded: Vec<u32> = session
        .target
        .collection()
        .expect("a collection run carries its collection")
        .beatmapsets
        .iter()
        .map(|set| set.id)
        .collect();
    assert_eq!(recorded, vec![10, 20, 30, 40]);
}

/// `retry` on the same prompt: nothing is excluded, so the run enqueues the
/// whole collection and its counts match. Same fixture, only the exclusion set
/// differs — so a green here with a red above isolates the exclusion itself.
#[tokio::test]
async fn accepted_retry_enqueues_every_previously_failed_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (session, events) = prepare_collection_run(
        collection(1, "alpha", &[10, 20, 30, 40]),
        &HashSet::new(),
        dir.path(),
    )
    .await;

    assert_eq!(
        session.pending_ids,
        vec![10, 20, 30, 40],
        "an accepted retry must enqueue the previously failed sets too"
    );
    assert_eq!(announced_total(&events), 4);
    assert_eq!(announced_queued(&events), 4);
}

/// The run's output dir already holds an archive for a DECLINED id. Precheck
/// must not see it: counting it would make a map the run never targeted show up
/// as skipped against a total that excludes it, and would carry it into
/// `initial_satisfied` → `reconcile_failed_maps`, clearing it out of
/// `failed-beatmapsets.json` without ever downloading it.
///
/// `30.osz` is the positive control — a targeted id's archive in the very same
/// directory, so an unseen `10` means the expectation filter dropped it rather
/// than the fixture being inert.
#[tokio::test]
async fn declined_retry_is_not_counted_by_precheck() {
    let dir = tempfile::tempdir().expect("tempdir");
    let payload = collection(1, "alpha", &[10, 20, 30, 40]);
    let output_dir = dir.path().join(payload.folder_name());
    std::fs::create_dir_all(&output_dir).expect("output dir");
    std::fs::write(output_dir.join("10.osz"), b"stub archive bytes").expect("declined archive");
    std::fs::write(output_dir.join("30.osz"), b"stub archive bytes").expect("targeted archive");

    let skipped = HashSet::from([10, 20]);
    let (session, _events) = prepare_collection_run(payload, &skipped, dir.path()).await;

    assert!(
        session.initial_satisfied.contains(&30),
        "control: a targeted id's archive in this dir must be counted, else the \
         fixture proves nothing about 10"
    );
    assert!(
        !session.initial_satisfied.contains(&10),
        "a declined id's archive must not be counted, or reconcile would clear \
         it from the failed-maps file without downloading it"
    );
    assert_eq!(
        session.skipped_existing, 1,
        "only the targeted archive may count toward the run's skipped tally"
    );
    assert_eq!(session.pending_ids, vec![40]);
}

#[test]
fn partition_pending_skips_owned_keeps_satisfied_and_drops_unverified() {
    let beatmapset_ids = vec![1, 2, 3, 4];
    // precheck: 4 satisfied (on disk), 3 on disk but FAILED validation (unverified).
    let mut satisfied = HashSet::from([4]);
    let mut unverified = HashSet::from([3]);
    // owned = {2, 3, 99}; 99 is not part of this collection and must not leak in.
    let owned = HashSet::from([2, 3, 99]);

    let (pending, skipped_owned) =
        partition_pending(&beatmapset_ids, &mut satisfied, &mut unverified, &owned);

    // only 1 still needs downloading; 2 + 3 were pre-skipped as owned.
    assert_eq!(pending, vec![1]);
    assert_eq!(skipped_owned, 2);
    // owned-in-collection ids land in `satisfied` (eligible for collection.db);
    // the already-satisfied 4 is not re-counted; the out-of-collection 99 stays out.
    assert!(satisfied.contains(&2));
    assert!(satisfied.contains(&3));
    assert!(satisfied.contains(&4));
    assert!(!satisfied.contains(&99));
    // 3 was owned + unverified: folding it into `satisfied` drops it from the
    // unverified set so it is not counted as both skipped and unverified.
    assert!(unverified.is_empty());
}

#[test]
fn partition_pending_empty_owned_is_noop() {
    let beatmapset_ids = vec![1, 2, 3];
    let mut satisfied = HashSet::from([2]);
    let mut unverified = HashSet::from([1]);

    let (pending, skipped_owned) = partition_pending(
        &beatmapset_ids,
        &mut satisfied,
        &mut unverified,
        &HashSet::new(),
    );

    assert_eq!(pending, vec![1, 3]);
    assert_eq!(skipped_owned, 0);
    // no owned ids → unverified untouched.
    assert_eq!(unverified, HashSet::from([1]));
}

#[test]
fn ids_folder_name_derives_from_tag() {
    // A plain tag becomes `<prefix>-<tag>`; different tags → different dirs,
    // so two concurrent runs never collide on the per-output-dir lock.
    assert_eq!(ids_folder_name("search", "tekno"), "search-tekno");
    assert_eq!(
        ids_folder_name("search", "blue zenith"),
        "search-blue zenith"
    );
    assert_eq!(ids_folder_name("filter", "a1b2c3d4"), "filter-a1b2c3d4");
}

#[test]
fn ids_folder_name_sanitizes_forbidden_chars() {
    // Path separators / reserved chars can't leak into the folder name.
    assert_eq!(ids_folder_name("search", "a/b:c*?"), "search-a_b_c__");
    assert_eq!(ids_folder_name("search", "../etc"), "search-.._etc");
}

#[test]
fn ids_folder_name_blank_falls_back() {
    // An empty or whitespace-only tag still yields a valid, recognizable dir.
    assert_eq!(ids_folder_name("search", ""), "search");
    assert_eq!(ids_folder_name("filter", "   "), "filter");
}

#[test]
fn session_target_ids_has_no_collection() {
    let target = SessionTarget::Ids {
        label: "tekno".to_string(),
        source: IdsRunSource::Search,
    };
    // A raw-ids run carries no collection metadata (skips `collection.db`).
    assert!(target.collection().is_none());
    assert!(target.selective_collections().is_none());
    // What precheck may count is pinned against a real output dir by
    // `declined_retry_is_not_counted_by_precheck`, not from a target kind.
}

#[test]
fn session_target_ids_announces_label_and_count() {
    let target = SessionTarget::Ids {
        label: "blue zenith".to_string(),
        source: IdsRunSource::Search,
    };
    let output = OutputPreparation {
        output_dir: PathBuf::from("/tmp/search-blue zenith"),
        display: "/tmp/search-blue zenith".to_string(),
    };
    let events = Mutex::new(Vec::new());
    target.announce_ready(
        &|event| events.lock().unwrap().push(event),
        7,
        &output,
        &[1, 2, 3],
    );
    let events = events.into_inner().unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        DownloadEvent::CollectionReady {
            collection_name,
            total_maps,
            ..
        } => {
            assert_eq!(collection_name, "blue zenith");
            // Count comes from the id list (no collection to read a length off).
            assert_eq!(*total_maps, 3);
        }
        other => panic!("expected CollectionReady, got {other:?}"),
    }
}
