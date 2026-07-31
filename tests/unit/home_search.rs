use super::*;
use crate::app::{App, AppCommand, FindStatusMsg};
use crate::config::Config;
use crate::core::search::BeatmapSetMeta;

fn app() -> App {
    App::new(Config::default())
}

fn meta(id: u32) -> BeatmapSetMeta {
    BeatmapSetMeta {
        id,
        title: format!("title {id}"),
        title_unicode: String::new(),
        artist: "artist".to_string(),
        artist_unicode: String::new(),
        creator: "mapper".to_string(),
        status: "ranked".to_string(),
        favourite_count: 0,
        play_count: 0,
        nsfw: false,
        video: false,
        beatmaps: Vec::new(),
    }
}

#[test]
fn loading_sets_status() {
    let mut app = app();
    handle_home_search_event(HomeSearchEvent::Loading, &mut app);
    assert_eq!(app.home.find.status_msg, FindStatusMsg::Loading);
}

#[test]
fn fresh_results_populate_and_descend() {
    let mut app = app();
    let follow_up = handle_home_search_event(
        HomeSearchEvent::Results {
            entries: vec![meta(1), meta(2)],
            total: 42,
            cursor: Some("NEXT".to_string()),
            append: false,
        },
        &mut app,
    );
    // Landed osu results hand back a size probe for whatever is checked.
    assert!(matches!(follow_up, Some(AppCommand::ProbeFindSizes)));
    assert_eq!(
        app.home.find.status_msg,
        FindStatusMsg::ReadySearch { total: 42 }
    );
    assert_eq!(app.home.find.next_cursor.as_deref(), Some("NEXT"));
    // A fresh search opens the results browse and records the osu backend.
    assert!(app.home.find.browse.is_browsing());
    assert_eq!(app.home.find.browse.rows.len(), 2);
    assert_eq!(
        app.home.find.results_backend(),
        Some(crate::app::FindBackend::Osu)
    );
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
    let ids: Vec<u32> = app.home.find.browse.rows.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
    // Last page reached: no more paging.
    assert!(app.home.find.next_cursor.is_none());
    assert!(app.home.find.browse.is_browsing());
}

#[test]
fn empty_result_stays_on_form() {
    let mut app = app();
    // Nothing landed → no size probe follow-up.
    let follow_up = handle_home_search_event(HomeSearchEvent::Empty, &mut app);
    assert!(follow_up.is_none());
    assert_eq!(app.home.find.status_msg, FindStatusMsg::Empty);
    // No results → the form stays put, nothing to browse.
    assert!(!app.home.find.browse.is_browsing());
    assert!(app.home.find.browse.rows.is_empty());
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
        app.home.find.status_msg,
        FindStatusMsg::Error("search requires login (401)".to_string())
    );
}

/// The painted hint bar for `app`: the last row of a rendered frame, at a width
/// wide enough that nothing trims.
fn hint_bar(app: &App) -> String {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).expect("test backend");
    terminal
        .draw(|frame| crate::tui::draw(frame, app))
        .expect("frame renders");
    let buf = terminal.backend().buffer();
    let y = buf.area.height - 1;
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

/// A fresh search lands with no keypress behind it, so it can open the browse
/// over a form row that is still descended into its own edit mode. The browse
/// owns input from that frame on, and a stale edit flag steals two things from
/// it: its whole hint bar (which collapses to the text-field exit affordance)
/// and its first `esc` (the edit-mode arm sits ahead of the browse ascend).
#[test]
fn results_landing_over_a_descended_chip_row_hand_input_to_the_browse() {
    use crate::app::{GetMapsSource, HomeField};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = app();
    app.home.source = GetMapsSource::Find;
    app.config.set_login_complete(true);
    app.home.find.toggle_advanced_filters();
    app.home.focus = HomeField::FindRank;
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.find_chip_editing(), "the row must start out descended");

    handle_home_search_event(
        HomeSearchEvent::Results {
            entries: vec![meta(1), meta(2)],
            total: 2,
            cursor: None,
            append: false,
        },
        &mut app,
    );
    assert!(app.home.find.browse.is_browsing());

    // The browse's own keys are what the painted bar advertises.
    let hints = hint_bar(&app);
    for key in ["↑↓ scroll", "↵ toggle", "a all / A none", "→ preview"] {
        assert!(hints.contains(key), "browse hint {key:?} missing: {hints}");
    }
    assert!(
        !hints.contains("esc done"),
        "the bar collapsed to the edit affordance: {hints}"
    );

    // And one press ascends, rather than being eaten by the stale edit mode.
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        !app.home.find.browse.is_browsing(),
        "the browse needed a second esc to ascend"
    );
}
