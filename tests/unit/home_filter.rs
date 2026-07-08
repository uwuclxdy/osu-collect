use super::*;
use crate::app::{App, AppCommand, FilterStatusMsg};
use crate::config::Config;
use osu_downloader::filter::{BeatmapDetails, FilterResults};
use std::collections::HashMap;

fn app() -> App {
    App::new(Config::default())
}

fn results(set_ids: Vec<u32>, diff_ids: Vec<u32>) -> FilterResults {
    let size_map: HashMap<u32, u64> = set_ids.iter().map(|&id| (id, 1_000_000)).collect();
    FilterResults {
        ids: diff_ids,
        set_ids,
        size_map,
        hashes: Vec::new(),
    }
}

fn detail(id: u32, set_id: u32, title: &str) -> BeatmapDetails {
    BeatmapDetails {
        id,
        set_id,
        title: title.to_string(),
        artist: "artist".to_string(),
        creator: "mapper".to_string(),
        version: "hard".to_string(),
        stars: 5.0,
        bpm: 180.0,
        ar: 9.0,
        cs: 4.0,
        od: 8.0,
        hp: 5.0,
        approved: "ranked".to_string(),
        mode: "osu".to_string(),
        total_length: 120,
        favourite_count: 10,
        play_count: 1000,
        size: 5_000_000,
    }
}

#[test]
fn loading_sets_status() {
    let mut app = app();
    let follow_up = handle_home_filter_event(HomeFilterEvent::Loading, &mut app);
    assert!(follow_up.is_none());
    assert_eq!(app.home.filter.status_msg, FilterStatusMsg::Loading);
}

#[test]
fn results_populate_descend_and_request_first_details_page() {
    let mut app = app();
    let follow_up = handle_home_filter_event(
        HomeFilterEvent::Results {
            results: results(vec![10, 20], vec![1, 2, 3]),
        },
        &mut app,
    );
    // The auto-fetch of the first details page rides back as a command.
    assert!(matches!(follow_up, Some(AppCommand::LoadFilterDetails)));
    assert_eq!(
        app.home.filter.status_msg,
        FilterStatusMsg::Ready {
            sets: 2,
            total_bytes: 2_000_000
        }
    );
    assert!(app.home.filter.browse.is_browsing());
    assert_eq!(app.home.filter.browse.rows.len(), 2);
    assert!(app.home.filter.browse.rows.iter().all(|r| r.meta.is_none()));
    assert!(app.home.filter.results_current());
    assert!(app.home.filter.has_more_details());
}

#[test]
fn details_fold_set_level_meta_first_diff_wins() {
    let mut app = app();
    handle_home_filter_event(
        HomeFilterEvent::Results {
            results: results(vec![10, 20], vec![1, 2, 3]),
        },
        &mut app,
    );
    let follow_up = handle_home_filter_event(
        HomeFilterEvent::Details {
            rows: vec![
                detail(1, 10, "first"),
                detail(2, 10, "second diff, same set"),
                detail(3, 20, "other set"),
            ],
        },
        &mut app,
    );
    assert!(follow_up.is_none());
    let rows = &app.home.filter.browse.rows;
    assert_eq!(
        rows[0].meta.as_ref().map(|m| m.title.as_str()),
        Some("first")
    );
    assert_eq!(
        rows[1].meta.as_ref().map(|m| m.title.as_str()),
        Some("other set")
    );
}

#[test]
fn details_failure_rewinds_the_pager_for_retry() {
    let mut app = app();
    handle_home_filter_event(
        HomeFilterEvent::Results {
            results: results(vec![10], (0..300).collect()),
        },
        &mut app,
    );
    // Simulate the first page having been pulled (as LoadFilterDetails would).
    let rewind_to = app.home.filter.details_cursor();
    let _ = app.home.filter.next_details_page().expect("page 1");
    handle_home_filter_event(
        HomeFilterEvent::DetailsFailed {
            reason: "HTTP 500".to_string(),
            rewind_to,
        },
        &mut app,
    );
    assert_eq!(app.home.filter.details_cursor(), rewind_to);
    assert!(app.home.filter.has_more_details());
}

#[test]
fn empty_clears_rows_and_snapshot() {
    let mut app = app();
    handle_home_filter_event(
        HomeFilterEvent::Results {
            results: results(vec![10], vec![1]),
        },
        &mut app,
    );
    handle_home_filter_event(HomeFilterEvent::Empty, &mut app);
    assert_eq!(app.home.filter.status_msg, FilterStatusMsg::Empty);
    assert!(app.home.filter.browse.rows.is_empty());
    assert!(!app.home.filter.results_current());
    assert!(!app.home.filter.has_more_details());
}

#[test]
fn failure_reports_the_reason_and_stales_results() {
    let mut app = app();
    handle_home_filter_event(
        HomeFilterEvent::Failed {
            reason: "nzbasic unreachable".to_string(),
        },
        &mut app,
    );
    assert_eq!(
        app.home.filter.status_msg,
        FilterStatusMsg::Error("nzbasic unreachable".to_string())
    );
    assert!(!app.home.filter.results_current());
}
