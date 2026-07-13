use super::{EnrichEvent, handle_enrich_event};
use crate::app::{App, BrowseRow, EnrichSink, EnrichTarget};
use crate::config::Config;
use osu_downloader::search::{BeatmapRow, BeatmapSetMeta};

fn app() -> App {
    App::new(Config::default())
}

/// A batch row for diff `id` under set `set_id`, with `nsfw`/`video` set so a
/// test can assert they survive the fold (the old details path hardcoded false).
fn beatmap_row(id: u32, set_id: u32, title: &str, nsfw: bool, video: bool) -> BeatmapRow {
    BeatmapRow {
        id,
        beatmapset_id: set_id,
        mode_int: Some(0),
        beatmapset: BeatmapSetMeta {
            id: set_id,
            title: title.to_string(),
            artist: "artist".to_string(),
            creator: "mapper".to_string(),
            status: "ranked".to_string(),
            favourite_count: 10,
            play_count: 1000,
            nsfw,
            video,
        },
    }
}

fn id_only_rows(ids: &[u32]) -> Vec<BrowseRow> {
    ids.iter().map(|&id| BrowseRow { id, meta: None }).collect()
}

/// Seed the find browse with id-only rows + an enrichment pager and pull the
/// first page (as the runtime dispatch would), returning the live generation.
fn seed_find(app: &mut App, set_ids: &[u32], diff_ids: Vec<u32>) -> u64 {
    app.home.find.browse.set_rows(id_only_rows(set_ids));
    app.home.find.browse.seed_enrichment(diff_ids);
    let _ = app.home.find.browse.next_enrich_page();
    app.home.find.browse.enrich_generation()
}

#[test]
fn enriched_folds_set_level_meta_first_diff_wins() {
    let mut app = app();
    let generation = seed_find(&mut app, &[10, 20], vec![1, 2, 3]);
    handle_enrich_event(
        EnrichEvent::Enriched {
            target: EnrichTarget::Find,
            generation,
            rows: vec![
                beatmap_row(1, 10, "first", false, false),
                beatmap_row(2, 10, "second diff, same set", false, false),
                beatmap_row(3, 20, "other set", false, false),
            ],
        },
        &mut app,
    );
    let rows = &app.home.find.browse.rows;
    assert_eq!(
        rows[0].meta.as_ref().map(|m| m.title.as_str()),
        Some("first"),
        "first diff of a set wins"
    );
    assert_eq!(
        rows[1].meta.as_ref().map(|m| m.title.as_str()),
        Some("other set")
    );
}

#[test]
fn enriched_lands_real_nsfw_and_video() {
    let mut app = app();
    let generation = seed_find(&mut app, &[10], vec![1]);
    handle_enrich_event(
        EnrichEvent::Enriched {
            target: EnrichTarget::Find,
            generation,
            rows: vec![beatmap_row(1, 10, "flagged", true, true)],
        },
        &mut app,
    );
    let meta = app.home.find.browse.rows[0]
        .meta
        .as_ref()
        .expect("row enriched");
    // The old beatmapDetails path hardcoded both to false; the batch nests them.
    assert!(meta.nsfw, "nsfw comes real from the nested beatmapset");
    assert!(meta.video, "video comes real from the nested beatmapset");
}

#[test]
fn stale_generation_page_is_dropped() {
    let mut app = app();
    let stale = seed_find(&mut app, &[10, 20], vec![1, 2]);
    // A newer run reseeds the pager, bumping the generation.
    app.home.find.browse.set_rows(id_only_rows(&[10, 20]));
    app.home.find.browse.seed_enrichment(vec![1, 2]);
    assert_ne!(app.home.find.browse.enrich_generation(), stale);

    handle_enrich_event(
        EnrichEvent::Enriched {
            target: EnrichTarget::Find,
            generation: stale,
            rows: vec![beatmap_row(1, 10, "late", false, false)],
        },
        &mut app,
    );
    assert!(
        app.home.find.browse.rows.iter().all(|r| r.meta.is_none()),
        "a page from the superseded generation must not fold"
    );
}

#[test]
fn hole_tolerance_folds_present_and_leaves_missing_id_only() {
    let mut app = app();
    // Three sets requested; the server omits set 20's diff (a hole).
    let generation = seed_find(&mut app, &[10, 20, 30], vec![1, 2, 3]);
    handle_enrich_event(
        EnrichEvent::Enriched {
            target: EnrichTarget::Find,
            generation,
            rows: vec![
                beatmap_row(1, 10, "present", false, false),
                beatmap_row(3, 30, "present too", false, false),
            ],
        },
        &mut app,
    );
    let rows = &app.home.find.browse.rows;
    assert!(rows[0].meta.is_some(), "set 10 enriched");
    assert!(rows[1].meta.is_none(), "set 20 (hole) stays id-only");
    assert!(rows[2].meta.is_some(), "set 30 enriched");
    // The pager already advanced past the hole at dispatch, so it never stalls.
    assert!(!app.home.find.browse.has_more_enrichment());
}

#[test]
fn failed_page_rewinds_the_pager_for_retry() {
    let mut app = app();
    app.home.find.browse.set_rows(id_only_rows(&[10]));
    app.home.find.browse.seed_enrichment((0..300).collect());
    let before = app.home.find.browse.enrich_cursor();
    let _ = app.home.find.browse.next_enrich_page();
    let generation = app.home.find.browse.enrich_generation();

    handle_enrich_event(
        EnrichEvent::Failed {
            target: EnrichTarget::Find,
            generation,
            rewind_to: before,
            reason: "HTTP 500".to_string(),
        },
        &mut app,
    );
    assert_eq!(app.home.find.browse.enrich_cursor(), before);
    assert!(app.home.find.browse.has_more_enrichment());
}

#[test]
fn stale_failed_page_drops_silently() {
    let mut app = app();
    let stale = seed_find(&mut app, &[10], vec![1, 2]);
    // A newer run reseeds the pager, bumping the generation.
    app.home.find.browse.set_rows(id_only_rows(&[10]));
    app.home.find.browse.seed_enrichment(vec![1, 2]);
    let cursor = app.home.find.browse.enrich_cursor();

    handle_enrich_event(
        EnrichEvent::Failed {
            target: EnrichTarget::Find,
            generation: stale,
            rewind_to: 999,
            reason: "HTTP 500".to_string(),
        },
        &mut app,
    );
    assert_eq!(
        app.home.find.browse.enrich_cursor(),
        cursor,
        "a superseded page's failure must not rewind the new pager"
    );
    assert!(
        app.toasts.is_empty(),
        "a superseded page's failure must not toast"
    );
}

#[test]
fn collection_target_folds_into_collection_browse_only() {
    let mut app = app();
    // Seed the find browse too, to prove the collection page doesn't touch it.
    let _ = seed_find(&mut app, &[10], vec![1]);
    app.home.collection_browse.set_rows(id_only_rows(&[10, 20]));
    app.home.collection_browse.seed_enrichment(vec![1, 2]);
    let _ = app.home.collection_browse.next_enrich_page();
    let generation = app.home.collection_browse.enrich_generation();

    handle_enrich_event(
        EnrichEvent::Enriched {
            target: EnrichTarget::Collection,
            generation,
            rows: vec![
                beatmap_row(1, 10, "col set", false, false),
                beatmap_row(2, 20, "col set two", false, false),
            ],
        },
        &mut app,
    );
    assert_eq!(
        app.home.collection_browse.rows[0]
            .meta
            .as_ref()
            .map(|m| m.title.as_str()),
        Some("col set")
    );
    assert!(
        app.home.find.browse.rows.iter().all(|r| r.meta.is_none()),
        "a collection-targeted page must not fold into the find browse"
    );
}

#[test]
fn update_target_folds_into_missing_set_cache_and_clears_loading() {
    use crate::app::update_source::{MissingBeatmapset, MissingStatus};
    let mut app = app();
    app.home
        .update
        .set_missing_beatmaps(vec![MissingBeatmapset {
            id: 10,
            status: MissingStatus::NotInstalled,
            collection_id: 100,
            collection_name: "col".to_string(),
            selected: true,
            previously_deleted: false,
            enrich_diff_id: Some(1000),
        }]);
    app.home.update.descend(); // seeds the pager off the missing sets
    let _ = app.home.update.next_enrich_page();
    app.home.update.mark_enrichment_dispatched();
    let generation = app.home.update.enrich_generation();

    handle_enrich_event(
        EnrichEvent::Enriched {
            target: EnrichTarget::Update,
            generation,
            rows: vec![beatmap_row(1000, 10, "missing song", false, false)],
        },
        &mut app,
    );

    assert_eq!(
        app.home.update.set_meta(10).map(|m| m.title.as_str()),
        Some("missing song"),
        "the batch page backfills the missing set's title"
    );
    assert!(
        !app.home.update.is_enriching(),
        "a landed page clears the loading cue"
    );
}

#[test]
fn late_prior_page_event_does_not_clear_a_newer_fetchs_cue() {
    // Reproduces the race the counter fixes: page N's task finished but its
    // Enriched event is still queued when the user presses `m`, scheduling page
    // N+1. Draining page N's event must NOT clear the cue while N+1 is pending.
    let mut app = app();
    app.home.find.browse.set_rows(id_only_rows(&[10, 20, 30]));
    app.home.find.browse.seed_enrichment((0..300).collect());

    // Page N requested + dispatched (in-flight = 1).
    let _ = app.home.find.browse.next_enrich_page();
    app.home.find.browse.mark_enrichment_dispatched();
    let gen_n = app.home.find.browse.enrich_generation();

    // Before N's event drains, the user presses `m`: page N+1 requested +
    // dispatched (in-flight = 2), same generation (no reseed).
    let _ = app.home.find.browse.next_enrich_page();
    app.home.find.browse.mark_enrichment_dispatched();

    // Now N's (older) Enriched event drains first. It must fold its rows but
    // leave the cue ON — page N+1 is still outstanding.
    handle_enrich_event(
        EnrichEvent::Enriched {
            target: EnrichTarget::Find,
            generation: gen_n,
            rows: vec![beatmap_row(0, 10, "page N", false, false)],
        },
        &mut app,
    );
    assert_eq!(
        app.home.find.browse.rows[0]
            .meta
            .as_ref()
            .map(|m| m.title.as_str()),
        Some("page N"),
        "the older page still folds"
    );
    assert!(
        app.home.find.browse.is_enriching(),
        "the cue stays on while page N+1 is in flight (no flicker)"
    );

    // N+1 lands → in-flight back to 0 → cue clears.
    handle_enrich_event(
        EnrichEvent::Enriched {
            target: EnrichTarget::Find,
            generation: gen_n,
            rows: vec![beatmap_row(1, 20, "page N+1", false, false)],
        },
        &mut app,
    );
    assert!(
        !app.home.find.browse.is_enriching(),
        "the cue clears once dry"
    );
}

/// Live parity check for the osu-batch endpoint. Guest `client_credentials` come
/// from runtime env so the test needs no build-injected creds. Run explicitly:
/// `OSU_CLIENT_ID=… OSU_CLIENT_SECRET=… cargo test --  --ignored live_batch`.
#[tokio::test]
#[ignore = "network: hits osu.ppy.sh"]
async fn live_batch_omits_holes_and_carries_meta() {
    let client_id = std::env::var("OSU_CLIENT_ID").expect("set OSU_CLIENT_ID for the live test");
    let client_secret =
        std::env::var("OSU_CLIENT_SECRET").expect("set OSU_CLIENT_SECRET for the live test");
    let http = reqwest::Client::new();
    let grant = crate::auth::client_credentials(&http, &client_id, &client_secret)
        .await
        .expect("guest token");

    let client = crate::core::search::SearchClient::new();
    // 75 + 129891 are real diff ids; 999999999 is a hole the server omits.
    let rows = client
        .beatmaps(&grant.access_token, &[75, 129_891, 999_999_999])
        .await
        .expect("batch call");

    let got: std::collections::HashSet<u32> = rows.iter().map(|r| r.id).collect();
    assert!(
        got.contains(&75) && got.contains(&129_891),
        "known ids present: {got:?}"
    );
    assert!(
        !got.contains(&999_999_999),
        "bogus id omitted (hole tolerance): {got:?}"
    );
    let meta = &rows
        .iter()
        .find(|r| r.id == 75)
        .expect("id 75 present")
        .beatmapset;
    assert!(
        !meta.title.is_empty() && !meta.creator.is_empty(),
        "set metadata rides nested in the row: {meta:?}"
    );
}
