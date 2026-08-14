//! Tests for every `dispatch_command` arm: the side effect each `AppCommand`
//! produces when dispatched, not just which command a key emits.
//!
//! ~40 key tests pin `app.handle_key -> AppCommand`; these pin
//! `dispatch_command(AppCommand, ..) -> side effect` for all 25 `Some` arms plus
//! the `None` fallthrough. Every arm that stores a `JoinHandle` asserts the slot
//! flipped to `Some`; every fire-and-forget arm asserts an event landed on its
//! channel; the download arms assert the `downloads` map grew (or shrank).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::app::EnrichSink;
use crate::app::collection::FailureReason;
use crate::app::collection_state::STATE_ENV_PATH;
use crate::app::failed_maps::{FAILED_MAPS_ENV_PATH, FailedMapsFile};
use crate::app::find_source::BrowseRow;
use crate::app::home::HomeField;
use crate::auth::AUTH_ENV_PATH;
use crate::config::Config;
use crate::download::{
    DownloadConfig, DownloadRequest, FailedMap, IdsDownloadRequest, IdsRunSource,
    SelectiveDownloadCollection, SelectiveDownloadRequest,
};
use crate::osu_db::OsuClient;
use crate::test_env::TempEnvVar;
use osu_downloader::ArchiveValidation;
use osu_downloader::filter::BeatmapDetails;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::sync::watch;

/// A minimal config with a temp output dir. Every download-spawn arm gets one
/// of these so the spawned task has *something* to work with before failing.
fn test_config() -> DownloadConfig {
    DownloadConfig {
        directory: std::env::temp_dir().to_string_lossy().into_owned(),
        mirrors: Vec::new(),
        concurrent: 1,
        archive_validation: ArchiveValidation::Off,
        auto_skip_rate_limited: false,
        rate_limit_skip_secs: 60,
    }
}

/// Build a `DownloadHandle` from fresh watch channels + a no-op task, so the
/// cancel/defer/skip receivers can be inspected after dispatch.
struct ProbeHandle {
    handle: DownloadHandle,
    cancel_rx: watch::Receiver<bool>,
    defer_rx: watch::Receiver<u64>,
    skip_rx: watch::Receiver<u64>,
}

fn probe_handle() -> ProbeHandle {
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (defer_tx, defer_rx) = watch::channel(0u64);
    let (skip_tx, skip_rx) = watch::channel(0u64);
    let join = tokio::spawn(async {});
    ProbeHandle {
        handle: DownloadHandle::new(cancel_tx, defer_tx, skip_tx, join),
        cancel_rx,
        defer_rx,
        skip_rx,
    }
}

/// The full runtime dispatch context: an isolated `App`, the `BackgroundTasks`
/// with all senders wired, and the receivers for assertions.
#[allow(dead_code)]
struct Fixture {
    _env: TempEnvVar,
    app: App,
    tasks: BackgroundTasks,
    downloads: HashMap<DownloadId, DownloadHandle>,
    download_tx: mpsc::UnboundedSender<DownloadEvent>,
    updates_tx: mpsc::UnboundedSender<UpdatesEvent>,
    auth_tx: mpsc::UnboundedSender<AuthEvent>,
    update_tx: mpsc::UnboundedSender<UpdateEvent>,
    download_rx: mpsc::UnboundedReceiver<DownloadEvent>,
    updates_rx: mpsc::UnboundedReceiver<UpdatesEvent>,
    auth_rx: mpsc::UnboundedReceiver<AuthEvent>,
    update_rx: mpsc::UnboundedReceiver<UpdateEvent>,
    home_resolve_rx: mpsc::UnboundedReceiver<HomeResolveEvent>,
    home_search_rx: mpsc::UnboundedReceiver<HomeSearchEvent>,
    home_filter_rx: mpsc::UnboundedReceiver<HomeFilterEvent>,
    home_enrich_rx: mpsc::UnboundedReceiver<EnrichEvent>,
    home_details_rx: mpsc::UnboundedReceiver<HomeDetailsEvent>,
    home_size_rx: mpsc::UnboundedReceiver<HomeSizeEvent>,
    home_cover_rx: mpsc::UnboundedReceiver<HomeCoverEvent>,
    mirror_probe_rx: mpsc::UnboundedReceiver<MirrorProbeEvent>,
}

impl Fixture {
    /// Isolate `AUTH_ENV_PATH` + `STATE_ENV_PATH` to nonexistent paths so
    /// `App::new` inherits none of the developer's real state, then build the
    /// full dispatch context with all channels wired.
    fn new() -> Self {
        Self::with_extra(&[])
    }

    /// Same, with extra `(env_key, value)` overrides for tests that need a
    /// store the constructor doesn't touch (e.g. `FAILED_MAPS_ENV_PATH`).
    fn with_extra(extra: &[(&'static str, String)]) -> Self {
        let mut vars: Vec<(&'static str, &str)> = vec![
            (AUTH_ENV_PATH, "/dev/null/dispatch-test-auth"),
            (STATE_ENV_PATH, "/dev/null/dispatch-test-state"),
        ];
        // Bind the extra values to a holding vec so their `&str` outlives `set_all`.
        let extra_refs: Vec<(&'static str, &str)> =
            extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
        vars.extend(extra_refs.iter().copied());
        let env = TempEnvVar::set_all(vars);

        let app = App::new(Config::default());

        let (download_tx, download_rx) = mpsc::unbounded_channel::<DownloadEvent>();
        let (updates_tx, updates_rx) = mpsc::unbounded_channel::<UpdatesEvent>();
        let (auth_tx, auth_rx) = mpsc::unbounded_channel::<AuthEvent>();
        let (update_tx, update_rx) = mpsc::unbounded_channel::<UpdateEvent>();
        let (home_resolve_tx, home_resolve_rx) = mpsc::unbounded_channel::<HomeResolveEvent>();
        let (home_search_tx, home_search_rx) = mpsc::unbounded_channel::<HomeSearchEvent>();
        let (home_filter_tx, home_filter_rx) = mpsc::unbounded_channel::<HomeFilterEvent>();
        let (home_enrich_tx, home_enrich_rx) = mpsc::unbounded_channel::<EnrichEvent>();
        let (home_details_tx, home_details_rx) = mpsc::unbounded_channel::<HomeDetailsEvent>();
        let (home_size_tx, home_size_rx) = mpsc::unbounded_channel::<HomeSizeEvent>();
        let (home_cover_tx, home_cover_rx) = mpsc::unbounded_channel::<HomeCoverEvent>();
        let (mirror_probe_tx, mirror_probe_rx) = mpsc::unbounded_channel::<MirrorProbeEvent>();

        let tasks = BackgroundTasks {
            login: None,
            resolve: None,
            resolve_cancel: None,
            home_resolve_tx,
            search: None,
            search_cancel: None,
            home_search_tx,
            filter: None,
            filter_cancel: None,
            home_filter_tx,
            enrich_find: None,
            enrich_collection: None,
            enrich_update: None,
            home_enrich_tx,
            home_details_tx,
            home_size_tx,
            home_cover_tx,
            mirror_probe: None,
            mirror_probe_cancel: None,
            mirror_probe_tx,
            update_apply: None,
        };

        Self {
            _env: env,
            app,
            tasks,
            downloads: HashMap::new(),
            download_tx,
            updates_tx,
            auth_tx,
            update_tx,
            download_rx,
            updates_rx,
            auth_rx,
            update_rx,
            home_resolve_rx,
            home_search_rx,
            home_filter_rx,
            home_enrich_rx,
            home_details_rx,
            home_size_rx,
            home_cover_rx,
            mirror_probe_rx,
        }
    }

    /// Dispatch a command through the real `dispatch_command`, returning the
    /// `should_quit` bool. Split-borrows `self` so callers don't need to.
    fn dispatch(&mut self, cmd: Option<AppCommand>) -> bool {
        dispatch_command(
            cmd,
            &mut self.app,
            &self.download_tx,
            &self.updates_tx,
            &self.auth_tx,
            &self.update_tx,
            &mut self.downloads,
            &mut self.tasks,
        )
    }
}

// ── download spawn ─────────────────────────────────────────────────────────

#[tokio::test]
async fn start_download_inserts_handle_into_downloads_map() {
    let mut fx = Fixture::new();
    let id: DownloadId = 7;
    let request = DownloadRequest {
        collection_input: "0".to_string(),
        config: test_config(),
        auto_overwrite: false,
        previously_failed_skipped: HashSet::new(),
        skip_already_imported: false,
        osu_client: OsuClient::Stable,
        osu_path: String::new(),
        prefetched: None,
    };
    assert!(!fx.dispatch(Some(AppCommand::StartDownload { id, request })));
    assert_eq!(fx.downloads.len(), 1, "handle inserted");
    assert!(fx.downloads.contains_key(&id), "keyed by the request id");
}

#[tokio::test]
async fn start_selective_download_inserts_handle_into_downloads_map() {
    let mut fx = Fixture::new();
    let id: DownloadId = 9;
    let request = SelectiveDownloadRequest {
        collection_ids: vec![1],
        beatmapset_ids: vec![10],
        collections: vec![SelectiveDownloadCollection {
            id: 1,
            name: "c".to_string(),
            beatmapset_ids: vec![10],
        }],
        config: test_config(),
        snapshot_dir: None,
        snapshots: vec![],
        skip_already_imported: false,
        osu_client: OsuClient::Stable,
        osu_path: String::new(),
        prefetched: HashMap::new(),
    };
    assert!(!fx.dispatch(Some(AppCommand::StartSelectiveDownload { id, request })));
    assert_eq!(fx.downloads.len(), 1, "selective handle inserted");
    assert!(fx.downloads.contains_key(&id));
}

#[tokio::test]
async fn start_ids_download_inserts_handle_into_downloads_map() {
    let mut fx = Fixture::new();
    let id: DownloadId = 11;
    let request = IdsDownloadRequest {
        beatmapset_ids: vec![10, 20],
        label: "ids-test".to_string(),
        folder_tag: "tag".to_string(),
        source: IdsRunSource::Search,
        config: test_config(),
        auto_overwrite: false,
        skip_already_imported: false,
        osu_client: OsuClient::Stable,
        osu_path: String::new(),
        known_sizes: HashMap::new(),
    };
    assert!(!fx.dispatch(Some(AppCommand::StartIdsDownload { id, request })));
    assert_eq!(fx.downloads.len(), 1, "ids handle inserted");
    assert!(fx.downloads.contains_key(&id));
}

// ── download control ───────────────────────────────────────────────────────

#[tokio::test]
async fn cancel_download_removes_handle_and_signals_shutdown() {
    let mut fx = Fixture::new();
    let id: DownloadId = 3;
    let probe = probe_handle();
    let cancel_rx = probe.cancel_rx;
    fx.downloads.insert(id, probe.handle);

    assert!(!fx.dispatch(Some(AppCommand::CancelDownload { id })));
    assert!(
        !fx.downloads.contains_key(&id),
        "handle removed from the map"
    );
    assert!(
        cancel_rx.has_changed().unwrap_or(false),
        "request_shutdown fired on the cancel channel"
    );
}

#[tokio::test]
async fn defer_rate_limited_bumps_the_defer_generation() {
    let mut fx = Fixture::new();
    let id: DownloadId = 5;
    let probe = probe_handle();
    let defer_rx = probe.defer_rx;
    fx.downloads.insert(id, probe.handle);

    assert!(!fx.dispatch(Some(AppCommand::DeferRateLimited { id })));
    assert!(
        fx.downloads.contains_key(&id),
        "defer does not remove the handle"
    );
    assert!(
        defer_rx.has_changed().unwrap_or(false),
        "defer_rate_limited bumped the defer generation"
    );
}

#[tokio::test]
async fn skip_rate_limited_bumps_the_skip_generation() {
    let mut fx = Fixture::new();
    let id: DownloadId = 6;
    let probe = probe_handle();
    let skip_rx = probe.skip_rx;
    fx.downloads.insert(id, probe.handle);

    assert!(!fx.dispatch(Some(AppCommand::SkipRateLimited { id })));
    assert!(
        fx.downloads.contains_key(&id),
        "skip does not remove the handle"
    );
    assert!(
        skip_rx.has_changed().unwrap_or(false),
        "skip_rate_limited bumped the skip generation"
    );
}

// ── auth ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn lazer_login_stores_a_task_handle() {
    let mut fx = Fixture::new();
    assert!(fx.tasks.login.is_none());
    assert!(!fx.dispatch(Some(AppCommand::LazerLogin {
        username: "user".to_string(),
        password: "pass".to_string(),
    })));
    assert!(
        fx.tasks.login.is_some(),
        "login task handle stored for cancellation"
    );
}

#[tokio::test]
async fn submit_verification_stores_a_task_handle() {
    let mut fx = Fixture::new();
    assert!(fx.tasks.login.is_none());
    assert!(!fx.dispatch(Some(AppCommand::SubmitVerification {
        code: "123456".to_string(),
    })));
    assert!(
        fx.tasks.login.is_some(),
        "verification task stored in the login slot"
    );
}

#[tokio::test]
async fn reissue_verification_fires_forget_event_on_auth_channel() {
    let mut fx = Fixture::new();
    assert!(fx.tasks.login.is_none(), "reissue is fire-and-forget");
    assert!(!fx.dispatch(Some(AppCommand::ReissueVerification)));
    let event = tokio::time::timeout(Duration::from_secs(10), fx.auth_rx.recv())
        .await
        .expect("reissue sent an AuthEvent before the timeout");
    assert!(
        event.is_some(),
        "ReissueComplete arrived (fails fast: no stored auth to load)"
    );
    assert!(
        fx.tasks.login.is_none(),
        "reissue did not occupy the login slot"
    );
}

#[tokio::test]
async fn cancel_login_aborts_the_stored_handle() {
    let mut fx = Fixture::new();
    // Seed the login slot the way LazerLogin would.
    fx.dispatch(Some(AppCommand::LazerLogin {
        username: "u".to_string(),
        password: "p".to_string(),
    }));
    assert!(fx.tasks.login.is_some());

    assert!(!fx.dispatch(Some(AppCommand::CancelLogin)));
    assert!(
        fx.tasks.login.is_none(),
        "cancel_login aborted + cleared the handle"
    );
}

#[tokio::test]
async fn logout_fires_event_on_auth_channel() {
    let mut fx = Fixture::new();
    assert!(!fx.dispatch(Some(AppCommand::Logout)));
    let event = tokio::time::timeout(Duration::from_secs(10), fx.auth_rx.recv())
        .await
        .expect("logout sent an AuthEvent before the timeout");
    assert!(
        event.is_some(),
        "LogoutComplete arrived (delete() no-ops on a nonexistent file)"
    );
}

// ── resolve ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cancel_resolve_clears_both_resolve_slots() {
    let mut fx = Fixture::new();
    // Seed a resolve the way ResolveCollectionUrl would (valid collection id).
    schedule_resolve(
        "123456",
        0,
        &mut fx.tasks.resolve,
        &mut fx.tasks.resolve_cancel,
        &fx.tasks.home_resolve_tx,
    );
    assert!(fx.tasks.resolve.is_some());

    assert!(!fx.dispatch(Some(AppCommand::CancelResolve)));
    assert!(
        fx.tasks.resolve.is_none(),
        "resolve handle cleared by cancel_resolve"
    );
    assert!(
        fx.tasks.resolve_cancel.is_none(),
        "resolve cancel sender cleared too"
    );
}

#[tokio::test]
async fn resolve_collection_url_schedules_a_resolve_task() {
    let mut fx = Fixture::new();
    assert!(fx.tasks.resolve.is_none());

    assert!(!fx.dispatch(Some(AppCommand::ResolveCollectionUrl {
        generation: 1,
        value: "123456".to_string(),
    })));
    assert!(
        fx.tasks.resolve.is_some(),
        "resolve task spawned for a parseable collection id"
    );
    assert!(
        fx.tasks.resolve_cancel.is_some(),
        "cancel sender installed alongside"
    );
}

// ── search / filter ────────────────────────────────────────────────────────

#[tokio::test]
async fn run_search_stores_task_and_cancel_handle() {
    let mut fx = Fixture::new();
    assert!(fx.tasks.search.is_none());
    assert!(fx.tasks.search_cancel.is_none());

    assert!(!fx.dispatch(Some(AppCommand::RunSearch {
        query: Default::default(),
        append: false,
    })));
    assert!(fx.tasks.search.is_some(), "search task stored");
    assert!(
        fx.tasks.search_cancel.is_some(),
        "cancel sender installed for the new search"
    );
    // The spawned task sends Loading immediately.
    let event = tokio::time::timeout(Duration::from_secs(5), fx.home_search_rx.recv())
        .await
        .expect("search sent Loading");
    assert!(
        matches!(event, Some(HomeSearchEvent::Loading)),
        "Loading is the first event a search run emits"
    );
}

#[tokio::test]
async fn run_filter_stores_task_and_cancel_handle() {
    let mut fx = Fixture::new();
    assert!(fx.tasks.filter.is_none());
    assert!(fx.tasks.filter_cancel.is_none());

    assert!(!fx.dispatch(Some(AppCommand::RunFilter {
        query: Default::default(),
    })));
    assert!(fx.tasks.filter.is_some(), "filter task stored");
    assert!(
        fx.tasks.filter_cancel.is_some(),
        "cancel sender installed for the new filter"
    );
    let event = tokio::time::timeout(Duration::from_secs(5), fx.home_filter_rx.recv())
        .await
        .expect("filter sent Loading");
    assert!(
        matches!(event, Some(HomeFilterEvent::Loading)),
        "Loading is the first event a filter run emits"
    );
}

// ── enrichment ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn load_enrichment_dispatches_a_find_enrichment_page() {
    let mut fx = Fixture::new();
    // Seed id-only find rows + the enrichment pager so next_enrich_page yields a page.
    let cache = HashMap::new();
    fx.app
        .home
        .find
        .browse
        .set_rows(vec![BrowseRow { id: 42, meta: None }], &cache);
    fx.app
        .home
        .find
        .browse
        .seed_enrichment(vec![(99, None)], &cache);

    assert!(fx.tasks.enrich_find.is_none());
    assert!(!fx.dispatch(Some(AppCommand::LoadEnrichment {
        target: EnrichTarget::Find,
    })));
    assert!(
        fx.tasks.enrich_find.is_some(),
        "enrichment page dispatched into the find slot"
    );
}

/// The nzbasic find target walks `beatmapDetails` first — the details response
/// is the only place nzbasic pairs a diff with its set, so the osu-batch pager
/// holds only seeds derived from landed pages. A dispatch with a dry pager
/// advances the walk instead of paging osu-batch directly.
#[tokio::test]
async fn load_enrichment_find_nzbasic_advances_the_details_walk_first() {
    let mut fx = Fixture::new();
    fx.app.home.find.note_results_backend(FindBackend::Nzbasic);
    let cache = HashMap::new();
    fx.app
        .home
        .find
        .browse
        .set_rows(vec![BrowseRow { id: 42, meta: None }], &cache);
    fx.app.home.find.browse.seed_details_walk(vec![999_999_999]);

    assert!(fx.tasks.enrich_find.is_none());
    assert!(!fx.dispatch(Some(AppCommand::LoadEnrichment {
        target: EnrichTarget::Find,
    })));
    assert!(
        fx.tasks.enrich_find.is_none(),
        "no osu-batch page while the walk still feeds the pager"
    );
    assert!(
        !fx.app.home.find.browse.has_more_enrichment(),
        "the walk advanced its only page (the pager is dry, so `m`'s view \
         reads the walk alone)"
    );
    assert!(
        fx.app.home.find.browse.is_enriching(),
        "the dispatched details page holds the cue"
    );

    // A bogus id 500s the whole details batch (or the network fails): either
    // way the fallback event lands carrying the slice's ids.
    let event = tokio::time::timeout(Duration::from_secs(20), fx.home_details_rx.recv())
        .await
        .expect("details event lands");
    assert!(
        matches!(event, Some(HomeDetailsEvent::Failed { ref ids, .. }) if ids == &vec![999_999_999]),
        "the failed slice reports its raw ids: {event:?}"
    );
}

/// Once a landed details page has queued derived seeds, a dispatch serves the
/// osu-batch pager (titles ready now) and leaves the walk for the next `m`.
#[tokio::test]
async fn load_enrichment_find_nzbasic_pages_the_pager_once_seeds_are_queued() {
    let mut fx = Fixture::new();
    fx.app.home.find.note_results_backend(FindBackend::Nzbasic);
    let cache = HashMap::new();
    fx.app
        .home
        .find
        .browse
        .set_rows(vec![BrowseRow { id: 42, meta: None }], &cache);
    fx.app.home.find.browse.seed_details_walk(vec![1, 2, 3]);
    // A landed details page's derivation, applied directly.
    let queued = fx
        .app
        .home
        .find
        .browse
        .queue_details_seeds(&[details_row(1, 42, 5.0), details_row(2, 42, 6.0)], &cache);
    assert_eq!(queued, 1, "two diffs of one set queue one seed");

    assert!(!fx.dispatch(Some(AppCommand::LoadEnrichment {
        target: EnrichTarget::Find,
    })));
    assert!(
        fx.tasks.enrich_find.is_some(),
        "the queued seeds page the osu-batch endpoint"
    );
    assert!(
        fx.app.home.find.browse.has_more_enrichment(),
        "the walk itself was not advanced by this dispatch — its only page \
         still stands, and the dispatched pager seed has paged out"
    );
}

/// Collection (and update) never seed a details walk: their seeds arrive
/// pre-paired and page osu-batch directly, exactly as before the rework.
#[tokio::test]
async fn load_enrichment_collection_target_keeps_paging_the_pager_directly() {
    let mut fx = Fixture::new();
    let cache = HashMap::new();
    fx.app
        .home
        .collection_browse
        .set_rows(vec![BrowseRow { id: 42, meta: None }], &cache);
    fx.app
        .home
        .collection_browse
        .seed_enrichment(vec![(99, Some(42))], &cache);

    assert!(!fx.dispatch(Some(AppCommand::LoadEnrichment {
        target: EnrichTarget::Collection,
    })));
    assert!(
        fx.tasks.enrich_collection.is_some(),
        "the collection pager dispatched into its own slot"
    );
    assert!(
        !fx.app.home.find.browse.has_more_enrichment(),
        "a collection dispatch never touches the find browse's walks"
    );
}

/// One details row for dispatch-test seeding: diff `id` under `set_id`.
fn details_row(id: u32, set_id: u32, stars: f64) -> BeatmapDetails {
    BeatmapDetails {
        id,
        set_id,
        title: format!("title {set_id}"),
        artist: "artist".to_string(),
        creator: "mapper".to_string(),
        version: "Insane".to_string(),
        stars,
        bpm: 180.0,
        ar: 9.0,
        cs: 4.0,
        od: 8.0,
        hp: 6.0,
        status: None,
        mode: None,
        total_length: 210,
        favourite_count: 100,
        play_count: 1000,
        size: 0,
        hash: String::new(),
        tags: String::new(),
        source: String::new(),
        genre: String::new(),
        language: String::new(),
        max_combo: 1000,
        hit_length: 118,
        pass_count: 500,
        approved_date: 0,
        last_update: 0,
    }
}

// ── probes ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn probe_mirrors_stores_probe_handle_and_cancel() {
    let mut fx = Fixture::new();
    assert!(fx.tasks.mirror_probe.is_none());

    assert!(!fx.dispatch(Some(AppCommand::ProbeMirrors)));
    assert!(fx.tasks.mirror_probe.is_some(), "mirror probe task stored");
    assert!(
        fx.tasks.mirror_probe_cancel.is_some(),
        "cancel sender installed for the probe"
    );
}

#[tokio::test]
async fn probe_find_sizes_claims_and_probes_checked_ids() {
    let mut fx = Fixture::new();
    // Seed a checked id-only find row so claim_size_probes has something to claim.
    let cache = HashMap::new();
    fx.app
        .home
        .find
        .browse
        .set_rows(vec![BrowseRow { id: 77, meta: None }], &cache);
    fx.app.home.find.browse.set_all_selected(true);
    fx.app.home.find.note_results_backend(FindBackend::Osu);

    assert!(
        !fx.app.home.find.claim_size_probes().is_empty(),
        "before dispatch the checked id is un-probed, so claim returns it"
    );
    // Restore the un-claimed state the arm expects to see: undo our probe claim
    // so the arm's own claim is what runs. The id is now Pending from our claim;
    // release it so the arm can re-claim.
    fx.app.home.find.release_size_probe(77);
    assert!(!fx.dispatch(Some(AppCommand::ProbeFindSizes)));
    assert!(
        fx.app.home.find.claim_size_probes().is_empty(),
        "after dispatch the arm already claimed the checked id (marked it \
         Pending), so a fresh claim returns nothing"
    );
}

#[tokio::test]
async fn fetch_cover_fires_event_on_cover_channel() {
    let mut fx = Fixture::new();
    assert!(!fx.dispatch(Some(AppCommand::FetchCover {
        set_id: 999_999_999
    })));
    // The CDN resolves a bogus set id to 404 on both variants -> Missing. If the
    // network is unreachable the reqwest calls fail -> also Missing. Either way
    // an event lands.
    let event = tokio::time::timeout(Duration::from_secs(20), fx.home_cover_rx.recv())
        .await
        .expect("cover fetch sent an event before the timeout");
    assert!(
        event.is_some(),
        "a cover fetch always resolves to Missing or Loaded"
    );
}

// ── scan ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn scan_local_database_sets_scan_handle() {
    let mut fx = Fixture::new();
    assert!(fx.app.scan_handle.is_none());
    assert!(!fx.dispatch(Some(AppCommand::ScanLocalDatabase)));
    assert!(
        fx.app.scan_handle.is_some(),
        "scan task stored in app.scan_handle"
    );
}

#[tokio::test]
async fn recheck_failed_maps_sets_scan_handle_when_failed_file_has_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("failed.json");
    let file = FailedMapsFile {
        schema_version: 1,
        beatmapset_ids: vec![100, 200],
    };
    std::fs::write(&path, serde_json::to_string(&file).unwrap()).unwrap();

    let path_str = path.to_str().unwrap().to_string();
    let mut fx = Fixture::with_extra(&[(FAILED_MAPS_ENV_PATH, path_str)]);

    assert!(fx.app.scan_handle.is_none());
    assert!(!fx.dispatch(Some(AppCommand::RecheckFailedMaps)));
    assert!(
        fx.app.scan_handle.is_some(),
        "recheck task stored in scan_handle when the failed-maps file has ids"
    );
}

// ── retry ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn retry_all_failed_spawns_ids_download_from_failed_page() {
    let mut fx = Fixture::new();
    // Pre-populate app.downloads with a page that has a retryable failed map
    // (non-NotFound reason) and a download_config snapshot.
    let mut page = crate::app::collection::CollectionPage::new(42, "run".to_string(), 1);
    page.failed_maps.push(FailedMap {
        beatmapset_id: 555,
        title: None,
        reason: FailureReason::NetworkError,
    });
    page.download_config = Some(test_config());
    page.output_dir = Some(std::env::temp_dir().to_string_lossy().into_owned());
    fx.app.downloads.push(page);

    assert!(fx.downloads.is_empty());
    assert!(!fx.dispatch(Some(AppCommand::RetryAllFailed { download_id: 42 })));
    assert_eq!(
        fx.downloads.len(),
        1,
        "retry spawned a new ids download for the retryable failed maps"
    );
}

// ── state ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn focus_output_dir_switches_to_home_and_targets_directory_field() {
    let mut fx = Fixture::new();
    // Pre-set away from the defaults so we confirm the arm overrides, not
    // that the defaults are still standing.
    fx.app.active_tab = Tab::Config;
    fx.app.editing = true;
    assert!(!fx.dispatch(Some(AppCommand::FocusOutputDir)));
    assert_eq!(fx.app.active_tab, Tab::Home, "switched to the home tab");
    assert_eq!(
        fx.app.home.focus,
        HomeField::Directory,
        "focus landed on the directory field"
    );
    assert!(
        !fx.app.editing,
        "selected-not-editing so Enter starts editing"
    );
}

// ── update ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn start_update_returns_false_without_panicking() {
    let mut fx = Fixture::new();
    // The spawned task hits the real GitHub API and may return Ok(None), so
    // update_rx can stay empty legitimately. The stored handle pins the
    // dispatch arm: deleting the spawn call leaves the slot None.
    assert!(
        !fx.dispatch(Some(AppCommand::StartUpdate)),
        "StartUpdate never signals quit"
    );
    assert!(
        fx.tasks.update_apply.is_some(),
        "StartUpdate must spawn the apply-update task"
    );
}

// ── quit ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn quit_signals_abort_on_every_active_download_and_returns_true() {
    let mut fx = Fixture::new();
    let probe = probe_handle();
    let cancel_rx = probe.cancel_rx;
    fx.downloads.insert(22, probe.handle);

    assert!(
        fx.dispatch(Some(AppCommand::Quit)),
        "Quit returns true so the loop breaks"
    );
    assert!(
        cancel_rx.has_changed().unwrap_or(false),
        "signal_abort_downloads fired request_shutdown on the live handle"
    );
}

// ── None ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn none_command_is_a_noop_returning_false() {
    let mut fx = Fixture::new();
    assert!(!fx.dispatch(None), "None must never signal quit");
    assert!(
        fx.downloads.is_empty(),
        "None left the downloads map untouched"
    );
    assert!(
        fx.tasks.login.is_none()
            && fx.tasks.resolve.is_none()
            && fx.tasks.search.is_none()
            && fx.tasks.filter.is_none(),
        "None left every task slot empty"
    );
}
