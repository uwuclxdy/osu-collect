use super::*;
use crate::app::{App, SearchStatusMsg};
use crate::config::Config;
use crate::core::search::BeatmapSetMeta;

fn app() -> App {
    App::new(Config::default())
}

fn meta(id: u32) -> BeatmapSetMeta {
    BeatmapSetMeta {
        id,
        title: format!("title {id}"),
        artist: "artist".to_string(),
        creator: "mapper".to_string(),
        status: "ranked".to_string(),
        favourite_count: 0,
        play_count: 0,
        nsfw: false,
        video: false,
    }
}

#[test]
fn loading_sets_status() {
    let mut app = app();
    handle_home_search_event(HomeSearchEvent::Loading, &mut app);
    assert_eq!(app.home.search.status_msg, SearchStatusMsg::Loading);
}

#[test]
fn fresh_results_populate_and_descend() {
    let mut app = app();
    handle_home_search_event(
        HomeSearchEvent::Results {
            entries: vec![meta(1), meta(2)],
            total: 42,
            cursor: Some("NEXT".to_string()),
            append: false,
        },
        &mut app,
    );
    assert_eq!(
        app.home.search.status_msg,
        SearchStatusMsg::Ready { total: 42 }
    );
    assert_eq!(app.home.search.next_cursor.as_deref(), Some("NEXT"));
    // A fresh search opens the results browse.
    assert!(app.home.search.browse.is_browsing());
    assert_eq!(app.home.search.browse.rows.len(), 2);
}

#[test]
fn append_page_dedups_and_keeps_browse_open() {
    let mut app = app();
    handle_home_search_event(
        HomeSearchEvent::Results {
            entries: vec![meta(1), meta(2)],
            total: 42,
            cursor: Some("P2".to_string()),
            append: false,
        },
        &mut app,
    );
    // Page overlap: 2 repeats, only 3 is new.
    handle_home_search_event(
        HomeSearchEvent::Results {
            entries: vec![meta(2), meta(3)],
            total: 42,
            cursor: None,
            append: true,
        },
        &mut app,
    );
    let ids: Vec<u32> = app.home.search.browse.rows.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
    // Last page reached: no more paging.
    assert!(app.home.search.next_cursor.is_none());
    assert!(app.home.search.browse.is_browsing());
}

#[test]
fn empty_result_stays_on_form() {
    let mut app = app();
    handle_home_search_event(HomeSearchEvent::Empty, &mut app);
    assert_eq!(app.home.search.status_msg, SearchStatusMsg::Empty);
    // No results → the form stays put, nothing to browse.
    assert!(!app.home.search.browse.is_browsing());
    assert!(app.home.search.browse.rows.is_empty());
}

#[test]
fn failed_search_surfaces_reason() {
    let mut app = app();
    handle_home_search_event(
        HomeSearchEvent::Failed {
            reason: "search requires login (401)".to_string(),
        },
        &mut app,
    );
    assert_eq!(
        app.home.search.status_msg,
        SearchStatusMsg::Error("search requires login (401)".to_string())
    );
}
