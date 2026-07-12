use crate::{
    app::runtime::{HomeResolveEvent, handle_home_resolve_event},
    app::{
        App, AppCommand,
        home::{HomeField, ResolveState},
    },
    config::Config,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn char_key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

/// Typing into the collection field emits ResolveCollectionUrl with the new value.
#[test]
fn typing_collection_url_emits_resolve_command() {
    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Collection;
    app.editing = true; // text inputs require edit mode before typing

    let cmd = app.handle_key(char_key('1'));

    assert!(
        matches!(cmd, Some(AppCommand::ResolveCollectionUrl { ref value }) if value == "1"),
        "expected ResolveCollectionUrl, got {cmd:?}"
    );
}

/// Backspace on the collection field also emits ResolveCollectionUrl.
#[test]
fn backspace_collection_field_emits_resolve_command() {
    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Collection;
    app.editing = true; // backspace edits only in edit mode
    // set_value parks the caret at the end so backspace deletes the last char.
    app.home.collection.set_value("12345");

    let cmd = app.handle_key(key(KeyCode::Backspace));

    assert!(
        matches!(cmd, Some(AppCommand::ResolveCollectionUrl { ref value }) if value == "1234"),
        "expected ResolveCollectionUrl after backspace, got {cmd:?}"
    );
}

/// Typing into a non-collection field does NOT emit ResolveCollectionUrl.
#[test]
fn typing_non_collection_field_does_not_emit_resolve() {
    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Directory;

    let cmd = app.handle_key(char_key('x'));

    assert!(
        !matches!(cmd, Some(AppCommand::ResolveCollectionUrl { .. })),
        "should not emit ResolveCollectionUrl for directory field"
    );
}

/// Backspace on an empty collection field must NOT emit — the value did not change.
#[test]
fn backspace_empty_collection_does_not_emit_resolve() {
    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Collection;
    assert!(app.home.collection.value.is_empty());

    let cmd = app.handle_key(key(KeyCode::Backspace));

    assert!(
        !matches!(cmd, Some(AppCommand::ResolveCollectionUrl { .. })),
        "no-op backspace must not spawn a resolve, got {cmd:?}"
    );
}

/// handle_home_resolve_event with Loading sets Loading state.
#[test]
fn resolve_loading_event_sets_loading_state() {
    let config = Config::default();
    let mut home = crate::app::HomeTab::new(&config);

    handle_home_resolve_event(HomeResolveEvent::Loading, &mut home);

    assert!(matches!(
        home.collection_resolve,
        Some((ResolveState::Loading, _))
    ));
}

/// handle_home_resolve_event with Resolved sets Success state and formats message.
#[test]
fn resolve_success_event_sets_success_state() {
    let config = Config::default();
    let mut home = crate::app::HomeTab::new(&config);

    handle_home_resolve_event(
        HomeResolveEvent::Resolved {
            name: "Top 100 of 2024".to_string(),
            map_count: 100,
            collection_id: 1,
            beatmapset_ids: Vec::new(),
            enrich_ids: vec![101, 202],
            folder_name: "top 100 of 2024-1".to_string(),
        },
        &mut home,
    );

    let Some((state, ref text)) = home.collection_resolve else {
        panic!("collection_resolve should be set");
    };
    assert_eq!(state, ResolveState::Success);
    assert!(text.contains("Top 100 of 2024"), "text = {text}");
    assert!(text.contains("100"), "text = {text}");
    assert!(text.contains("maps"), "text = {text}");
    // The per-collection folder is stored for the directory tooltip.
    assert_eq!(
        home.resolved_folder_name.as_deref(),
        Some("top 100 of 2024-1")
    );
    // The diff ids for browse&pick enrichment are stored alongside.
    assert_eq!(home.resolved_enrich_ids, vec![101, 202]);
}

/// handle_home_resolve_event with Failed sets Error state.
#[test]
fn resolve_failed_event_sets_error_state() {
    let config = Config::default();
    let mut home = crate::app::HomeTab::new(&config);

    handle_home_resolve_event(
        HomeResolveEvent::Failed {
            reason: "collection not found".to_string(),
        },
        &mut home,
    );

    let Some((state, ref text)) = home.collection_resolve else {
        panic!("collection_resolve should be set");
    };
    assert_eq!(state, ResolveState::Error);
    assert!(text.contains("not found"), "text = {text}");
}

/// handle_home_resolve_event with Cleared clears the resolve display.
#[test]
fn resolve_cleared_event_clears_state() {
    let config = Config::default();
    let mut home = crate::app::HomeTab::new(&config);
    home.set_collection_resolve(ResolveState::Success, "something");

    handle_home_resolve_event(HomeResolveEvent::Cleared, &mut home);

    assert!(home.collection_resolve.is_none());
}

/// Singular map count uses "map" not "maps".
#[test]
fn resolve_single_map_uses_singular() {
    let config = Config::default();
    let mut home = crate::app::HomeTab::new(&config);

    handle_home_resolve_event(
        HomeResolveEvent::Resolved {
            name: "Solo".to_string(),
            map_count: 1,
            collection_id: 2,
            beatmapset_ids: Vec::new(),
            enrich_ids: Vec::new(),
            folder_name: "solo-2".to_string(),
        },
        &mut home,
    );

    let Some((_, ref text)) = home.collection_resolve else {
        panic!("collection_resolve should be set");
    };
    assert!(
        text.contains("1 mapset"),
        "expected '1 mapset', got: {text}"
    );
    assert!(
        !text.contains("1 mapsets"),
        "should not contain '1 mapsets': {text}"
    );
}
