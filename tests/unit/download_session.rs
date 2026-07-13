use super::{
    OutputPreparation, SessionTarget, ids_folder_name, partition_pending, resolve_selective_with,
};
use crate::core::collection::{Beatmap, Beatmapset, Collection, CollectionService, Uploader};
use crate::download::IdsRunSource;
use crate::download::{DownloadEvent, SelectiveDownloadCollection};
use crate::utils;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
    // The expectation index is the requested id set, not a collection's contents.
    let ids = [10u32, 20, 30];
    let index = target.expectation_index(&ids);
    assert_eq!(*index, HashSet::from([10, 20, 30]));
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
