//! The nzbasic details walk as the enrichment seed source: each landed
//! `beatmapDetails` page pairs its diffs with their sets, so it derives ONE
//! representative diff per set for the osu-batch pager (pruned against the
//! session cache and sets already seeded this run). A failed page falls back
//! to seeding its raw diff ids unpaired — titles never depend on the details
//! endpoint.

use super::{HomeDetailsEvent, handle_home_details_event};
use crate::app::collection_state::STATE_ENV_PATH;
use crate::app::{App, AppCommand, BrowseRow, EnrichSink, EnrichTarget};
use crate::auth::AUTH_ENV_PATH;
use crate::config::Config;
use crate::test_env::TempEnvVar;
use osu_downloader::filter::BeatmapDetails;
use osu_downloader::search::BeatmapSetMeta;
use std::collections::HashMap;

/// An isolated `App` — `App::new` reads `STATE_ENV_PATH` + `AUTH_ENV_PATH` at
/// construction, so both are pointed at nonexistent paths first.
fn isolated_app() -> (TempEnvVar, App) {
    let env = TempEnvVar::set_all([
        (AUTH_ENV_PATH, "/dev/null/home-details-test-auth"),
        (STATE_ENV_PATH, "/dev/null/home-details-test-state"),
    ]);
    (env, App::new(Config::default()))
}

/// One details row: diff `id` under `set_id` at `stars`. Only the seeding- and
/// fold-relevant fields vary; the rest are representative constants.
fn detail(id: u32, set_id: u32, stars: f64) -> BeatmapDetails {
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

fn id_only_rows(ids: &[u32]) -> Vec<BrowseRow> {
    ids.iter().map(|&id| BrowseRow { id, meta: None }).collect()
}

/// Seed a find browse the way a nzbasic results landing leaves it: id-only
/// rows and a details walk over the raw diff ids, first page dispatched.
fn seeded_browse(app: &mut App, set_ids: &[u32], diff_ids: Vec<u32>) -> u64 {
    app.home
        .find
        .browse
        .set_rows(id_only_rows(set_ids), &HashMap::new());
    app.home.find.browse.seed_details_walk(diff_ids);
    let _ = app.home.find.browse.next_details_page();
    app.home.find.browse.mark_details_dispatched();
    app.home.find.browse.details_walk_generation()
}

#[test]
fn loaded_page_derives_one_seed_per_set_and_returns_a_page_command() {
    let (_env, mut app) = isolated_app();
    let generation = seeded_browse(&mut app, &[10, 20], vec![1, 2, 3]);
    assert!(
        app.home.find.browse.is_enriching(),
        "a dispatched details page holds the cue"
    );

    let follow_up = handle_home_details_event(
        HomeDetailsEvent::Loaded {
            generation,
            ids: vec![1, 2, 3],
            rows: vec![detail(1, 10, 5.0), detail(2, 10, 6.2), detail(3, 20, 4.0)],
        },
        &mut app,
    );

    assert!(
        matches!(
            follow_up,
            Some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Find
            })
        ),
        "queued seeds auto-fetch their osu-batch page"
    );
    // One representative diff per set — the first row of each set in page
    // order. Three diff ids in, two out (~3.5x fewer batch ids on real data).
    assert_eq!(
        app.home.find.browse.next_enrich_page(),
        Some(vec![1, 3]),
        "one seed per set, first diff wins"
    );
    // The page's extra columns folded too — the hardest diff of set 10.
    assert_eq!(
        app.home.find.browse.details_for(10).map(|d| d.id),
        Some(2),
        "record_details keeps folding the hardest diff per set"
    );
    assert!(app.home.find.browse.details_for(20).is_some());
    assert!(
        !app.home.find.browse.is_enriching(),
        "the landing settles the walk's cue"
    );
}

#[test]
fn loaded_page_prunes_cached_and_already_seeded_sets() {
    let (_env, mut app) = isolated_app();
    let generation = seeded_browse(&mut app, &[10, 20], vec![1, 3]);
    // Set 10 is already known this session: seeding must prune it.
    let cached = BeatmapSetMeta {
        id: 10,
        ..BeatmapSetMeta::default()
    };
    app.home.meta_cache.insert(10, cached);

    handle_home_details_event(
        HomeDetailsEvent::Loaded {
            generation,
            ids: vec![1, 3],
            rows: vec![detail(1, 10, 5.0), detail(3, 20, 4.0)],
        },
        &mut app,
    );
    assert_eq!(
        app.home.find.browse.next_enrich_page(),
        Some(vec![3]),
        "a cached set is pruned from the derived seeds"
    );
    let pager_generation = app.home.find.browse.enrich_generation();

    // A later page naming set 20 again (a page boundary can split a set)
    // queues nothing: the set is already seeded this run.
    let follow_up = handle_home_details_event(
        HomeDetailsEvent::Loaded {
            generation,
            ids: vec![4],
            rows: vec![detail(4, 20, 4.4)],
        },
        &mut app,
    );
    assert!(
        follow_up.is_none(),
        "no new seeds queued — nothing to auto-fetch"
    );
    assert!(
        app.home.find.browse.next_enrich_page().is_none(),
        "an already-seeded set is never re-queued"
    );
    assert_eq!(
        app.home.find.browse.enrich_generation(),
        pager_generation,
        "queueing extends the pager, it never reseeds"
    );
}

/// A 200-subset response (the server omitted rows — a wrong body shape even
/// returns 200 []) strands the requested ids it didn't return: those holes
/// have no set pairing to derive from, so they fall back to raw seeding.
/// Titles never depend on the details endpoint returning every row.
#[test]
fn loaded_page_seeds_the_requested_ids_a_subset_response_omitted() {
    let (_env, mut app) = isolated_app();
    let generation = seeded_browse(&mut app, &[10, 20], vec![1, 2, 3]);

    let follow_up = handle_home_details_event(
        HomeDetailsEvent::Loaded {
            generation,
            ids: vec![1, 2, 3],
            rows: vec![detail(1, 10, 5.0), detail(3, 20, 4.0)],
        },
        &mut app,
    );

    assert!(
        matches!(
            follow_up,
            Some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Find
            })
        ),
        "the raw holes still need their titles fetched"
    );
    // The returned sets derive their reps first, then the omitted id 2 falls
    // back raw — no id from the requested slice is stranded.
    assert_eq!(
        app.home.find.browse.next_enrich_page(),
        Some(vec![1, 3, 2]),
        "derived reps first, the shortfall raw after"
    );
    assert!(
        !app.home.find.browse.is_enriching(),
        "the landing settles the walk's cue"
    );
}

/// A page whose rows carry ids the requested slice never contained is not
/// diffable — a difference against foreign ids would be meaningless — so the
/// whole requested slice falls back raw, exactly as a failed page would. The
/// fallback is deliberately blunt: id 1 was derived too, so it pages twice
/// (the documented cost of the non-diffable path; the fold fills only rows
/// still missing meta).
#[test]
fn loaded_page_with_foreign_row_ids_falls_back_to_the_whole_slice() {
    let (_env, mut app) = isolated_app();
    let generation = seeded_browse(&mut app, &[10, 20], vec![1, 2, 3]);

    let follow_up = handle_home_details_event(
        HomeDetailsEvent::Loaded {
            generation,
            ids: vec![1, 2, 3],
            rows: vec![detail(99, 999, 4.0), detail(1, 10, 5.0)],
        },
        &mut app,
    );

    assert!(
        matches!(
            follow_up,
            Some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Find
            })
        ),
        "the whole-slice fallback still needs its titles fetched"
    );
    // Whatever rows arrived derive their seeds unconditionally; the foreign
    // response then falls the whole slice back raw.
    assert_eq!(
        app.home.find.browse.next_enrich_page(),
        Some(vec![99, 1, 1, 2, 3]),
        "derived seeds first, then the whole requested slice raw"
    );
}

#[test]
fn failed_page_falls_back_to_raw_ids_and_returns_a_page_command() {
    let (_env, mut app) = isolated_app();
    let generation = seeded_browse(&mut app, &[10], vec![7, 8]);

    let follow_up = handle_home_details_event(
        HomeDetailsEvent::Failed {
            generation,
            ids: vec![7, 8],
        },
        &mut app,
    );

    assert!(
        matches!(
            follow_up,
            Some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Find
            })
        ),
        "the fallback must actively fetch titles, not wait for `m`"
    );
    // Today's behavior, arrived at differently: every raw diff id pages.
    assert_eq!(app.home.find.browse.next_enrich_page(), Some(vec![7, 8]));
    assert!(
        !app.home.find.browse.is_enriching(),
        "the failure settles the walk's cue"
    );
}

#[test]
fn stale_generation_pages_drop_before_folding_or_seeding() {
    let (_env, mut app) = isolated_app();
    let stale = seeded_browse(&mut app, &[10, 20], vec![1, 2]);
    // A newer run replaces the rows — every walk reseeds, bumping generations.
    app.home
        .find
        .browse
        .set_rows(id_only_rows(&[30]), &HashMap::new());
    assert_ne!(app.home.find.browse.details_walk_generation(), stale);

    let follow_up = handle_home_details_event(
        HomeDetailsEvent::Loaded {
            generation: stale,
            ids: vec![1],
            rows: vec![detail(1, 10, 5.0)],
        },
        &mut app,
    );
    assert!(follow_up.is_none());
    assert!(
        app.home.find.browse.details_for(10).is_none(),
        "a superseded page must not fold its columns"
    );
    assert!(
        app.home.find.browse.next_enrich_page().is_none(),
        "a superseded page must not queue seeds into the new run's pager"
    );

    let follow_up = handle_home_details_event(
        HomeDetailsEvent::Failed {
            generation: stale,
            ids: vec![1],
        },
        &mut app,
    );
    assert!(
        follow_up.is_none(),
        "a superseded failure must not seed raw ids either"
    );
    assert!(app.home.find.browse.next_enrich_page().is_none());
}
