use crate::{
    app::runtime::{HomeResolveEvent, handle_home_resolve_event},
    app::{
        App, AppCommand,
        home::{HomeField, ResolveState},
    },
    config::Config,
    core::collection::{Beatmap, Beatmapset, Collection, Uploader},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn char_key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

/// `(set_id, diff_ids)` pairs → a collection payload the resolve task would send.
fn collection(id: u32, name: &str, sets: &[(u32, &[u32])]) -> Collection {
    Collection {
        id,
        name: name.to_string(),
        description: None,
        uploader: Uploader {
            id: 0,
            username: "u".to_string(),
        },
        beatmapsets: sets
            .iter()
            .map(|&(set_id, diffs)| Beatmapset {
                id: set_id,
                beatmaps: diffs
                    .iter()
                    .map(|&diff_id| Beatmap {
                        id: diff_id,
                        checksum: "abc".to_string(),
                    })
                    .collect(),
            })
            .collect(),
        favourites: 0,
    }
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
            collection_id: 1,
            collection: collection(1, "Top 100 of 2024", &[(11, &[101, 102]), (22, &[202])]),
        },
        &mut home,
    );

    let Some((state, ref text)) = home.collection_resolve else {
        panic!("collection_resolve should be set");
    };
    assert_eq!(state, ResolveState::Success);
    assert!(text.contains("Top 100 of 2024"), "text = {text}");
    assert!(text.contains("2 mapsets"), "text = {text}");
    // The per-collection folder is derived for the directory tooltip.
    assert_eq!(
        home.resolved_folder_name.as_deref(),
        Some("Top 100 of 2024-1")
    );
    assert_eq!(home.resolved_collection, Some((1, vec![11, 22])));
    // One (set, diff) pair per set for browse&pick enrichment — the first diff.
    assert_eq!(home.resolved_enrich_pairs, vec![(11, 101), (22, 202)]);
}

/// The resolve's payload is parked in the session cache, so the download press
/// reuses it instead of refetching the identical collection.
#[test]
fn resolve_success_event_caches_the_payload() {
    let config = Config::default();
    let mut home = crate::app::HomeTab::new(&config);

    handle_home_resolve_event(
        HomeResolveEvent::Resolved {
            collection_id: 1,
            collection: collection(1, "cached", &[(11, &[101])]),
        },
        &mut home,
    );

    let cached = home
        .collection_cache
        .get_fresh(1)
        .expect("a fresh resolve is cached for the download");
    assert_eq!(cached.name, "cached");
    assert_eq!(cached.beatmapsets.len(), 1);
    // A different collection was never resolved, so it stays a miss.
    assert!(home.collection_cache.get_fresh(2).is_none());
}

/// Enrichment seeds carry one diff per UNIQUE set; a set with no diffs has no
/// pair to seed and is left id-only.
#[test]
fn resolve_enrich_pairs_dedupe_sets_and_skip_diffless() {
    let config = Config::default();
    let mut home = crate::app::HomeTab::new(&config);

    handle_home_resolve_event(
        HomeResolveEvent::Resolved {
            collection_id: 3,
            collection: collection(
                3,
                "dupes",
                &[(11, &[101]), (11, &[999]), (22, &[]), (33, &[303])],
            ),
        },
        &mut home,
    );

    assert_eq!(home.resolved_enrich_pairs, vec![(11, 101), (33, 303)]);
    // The id list keeps every row (the duplicate included) — only the seeds dedupe.
    assert_eq!(home.resolved_collection, Some((3, vec![11, 11, 22, 33])));
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
            collection_id: 2,
            collection: collection(2, "Solo", &[(11, &[101])]),
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

/// A resolve outcome invalidates a deferred collection-browse open (its rows may
/// have changed), so any pending wait is dropped.
#[test]
fn resolve_event_clears_pending_collection_browse() {
    let config = Config::default();
    let mut home = crate::app::HomeTab::new(&config);
    home.collection_browse_pending = Some(7);

    handle_home_resolve_event(HomeResolveEvent::Cleared, &mut home);

    assert!(
        home.collection_browse_pending.is_none(),
        "a resolve/clear drops a deferred open"
    );
}
