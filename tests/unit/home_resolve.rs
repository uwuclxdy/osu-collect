use crate::{
    app::runtime::{HomeResolveEvent, HomeResolveKind, handle_home_resolve_event},
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

/// An event stamped with the generation the form is currently accepting — what
/// the newest scheduled request carries.
fn at_live_gen(home: &crate::app::HomeTab, kind: HomeResolveKind) -> HomeResolveEvent {
    HomeResolveEvent {
        generation: home.resolve_generation(),
        kind,
    }
}

/// An event carrying a generation captured earlier, i.e. the one an in-flight
/// request is holding.
///
/// Deliberately NOT `live - 1`: a fabricated stale value is stale whether or not
/// `supersede_resolve` actually advances anything, so a test built on one pins
/// the handler's check while leaving the bump free to be deleted. Callers
/// capture `resolve_generation()` before the superseding action and pass it here,
/// so the two halves are pinned together.
fn at_gen(generation: u64, kind: HomeResolveKind) -> HomeResolveEvent {
    HomeResolveEvent { generation, kind }
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

/// A form in the state a resolve for `id` is dispatched from: the URL field
/// holds that id, which is the only way `schedule_resolve` spawns a task for it.
/// A landing handler now reads the field to decide whether its response is still
/// wanted, so a fixture that skipped this was in a state the app cannot produce.
fn home_awaiting(id: u32) -> crate::app::HomeTab {
    let mut home = crate::app::HomeTab::new(&Config::default());
    home.collection.set_value(id.to_string());
    home
}

/// Typing into the collection field emits ResolveCollectionUrl with the new value.
#[test]
fn typing_collection_url_emits_resolve_command() {
    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Collection;
    app.editing = true; // text inputs require edit mode before typing

    let cmd = app.handle_key(char_key('1'));

    assert!(
        matches!(cmd, Some(AppCommand::ResolveCollectionUrl { ref value, .. }) if value == "1"),
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
        matches!(cmd, Some(AppCommand::ResolveCollectionUrl { ref value, .. }) if value == "1234"),
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

/// `cancel_resolve` both signals a task still inside its debounce and drops the
/// handle it would otherwise be asked to abort twice. Pinned directly because
/// the cancel is now an effect a caller can reach without scheduling, so its
/// contract has to hold on its own rather than as a prefix of `schedule_resolve`.
#[test]
fn cancel_resolve_signals_and_clears_the_in_flight_task() {
    use tokio::sync::watch;

    let (tx, rx) = watch::channel(false);
    let mut handle = None;
    let mut cancel_tx = Some(tx);

    super::cancel_resolve(&mut handle, &mut cancel_tx);

    assert!(
        *rx.borrow(),
        "a task inside its debounce must see the cancel"
    );
    assert!(
        cancel_tx.is_none(),
        "the sender is taken, so a second cancel is a no-op rather than a re-signal"
    );
}

/// handle_home_resolve_event with Loading sets Loading state.
#[test]
fn resolve_loading_event_sets_loading_state() {
    let mut home = home_awaiting(5);

    handle_home_resolve_event(at_live_gen(&home, HomeResolveKind::Loading), &mut home);

    assert!(matches!(
        home.collection_resolve,
        Some((ResolveState::Loading, _))
    ));
}

/// handle_home_resolve_event with Resolved sets Success state and formats message.
#[test]
fn resolve_success_event_sets_success_state() {
    let mut home = home_awaiting(1);

    handle_home_resolve_event(
        at_live_gen(
            &home,
            HomeResolveKind::Resolved {
                collection_id: 1,
                collection: collection(1, "Top 100 of 2024", &[(11, &[101, 102]), (22, &[202])]),
            },
        ),
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
    // Every (set, diff) pair feeds browse&pick enrichment so each set gains a
    // full difficulty spread, not just its first diff.
    assert_eq!(
        home.resolved_enrich_pairs,
        vec![(11, 101), (11, 102), (22, 202)]
    );
}

/// The resolve's payload is parked in the session cache, so the download press
/// reuses it instead of refetching the identical collection.
#[test]
fn resolve_success_event_caches_the_payload() {
    let mut home = home_awaiting(1);

    handle_home_resolve_event(
        at_live_gen(
            &home,
            HomeResolveKind::Resolved {
                collection_id: 1,
                collection: collection(1, "cached", &[(11, &[101])]),
            },
        ),
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
    let mut home = home_awaiting(3);

    handle_home_resolve_event(
        at_live_gen(
            &home,
            HomeResolveKind::Resolved {
                collection_id: 3,
                collection: collection(
                    3,
                    "dupes",
                    &[(11, &[101]), (11, &[999]), (22, &[]), (33, &[303])],
                ),
            },
        ),
        &mut home,
    );

    assert_eq!(home.resolved_enrich_pairs, vec![(11, 101), (33, 303)]);
    // The id list keeps every row (the duplicate included) — only the seeds dedupe.
    assert_eq!(home.resolved_collection, Some((3, vec![11, 11, 22, 33])));
}

/// handle_home_resolve_event with Failed sets Error state.
#[test]
fn resolve_failed_event_sets_error_state() {
    let mut home = home_awaiting(7);

    handle_home_resolve_event(
        at_live_gen(
            &home,
            HomeResolveKind::Failed {
                reason: "collection not found".to_string(),
            },
        ),
        &mut home,
    );

    let Some((state, ref text)) = home.collection_resolve else {
        panic!("collection_resolve should be set");
    };
    assert_eq!(state, ResolveState::Error);
    assert!(text.contains("not found"), "text = {text}");
}

/// The keystroke that returns to an already-resolved id schedules NO fetch: the
/// settle re-armed the snapshot from the session cache, so there is nothing left
/// to ask for. Driven through the key handler because the command is what
/// spawns the request — asserting the snapshot alone would leave the round trip
/// this is meant to avoid unobserved.
///
/// A cache hit deliberately does not fire a confirming request either. The TTL
/// inside `get_fresh` is the freshness contract the download path already reads
/// this store under; revalidating on a hit would be a second, stricter notion of
/// fresh living only on this path, and its failure would land an error line over
/// a snapshot that is present and correct.
#[test]
fn returning_to_a_cached_id_schedules_no_fetch() {
    let mut app = App::new(Config::default());
    app.editing = true;
    app.home.focus = HomeField::Collection;

    // Collection 42 resolves the way the runtime resolves it, which is also what
    // fills the cache.
    app.home.collection.set_value("42");
    handle_home_resolve_event(
        at_live_gen(
            &app.home,
            HomeResolveKind::Resolved {
                collection_id: 42,
                collection: collection(42, "Farm", &[(10, &[101]), (20, &[202])]),
            },
        ),
        &mut app.home,
    );
    assert_eq!(app.home.resolved_collection, Some((42, vec![10, 20])));

    // A stray digit: 421 was never fetched, so this one does schedule.
    let cmd = app.handle_key(char_key('1'));
    assert!(
        matches!(cmd, Some(AppCommand::ResolveCollectionUrl { ref value, .. }) if value == "421"),
        "an uncached id must still go to the network, got {cmd:?}"
    );
    assert_eq!(app.home.resolved_collection, None, "421 is not resolved");

    // Backspace back onto 42: served from the cache, so the command cancels
    // rather than schedules. NOT `None` — cancelling and scheduling are separate
    // effects, and returning nothing here would skip the cancel along with the
    // fetch, leaving 421's task alive to strand a `resolving…` cue over the
    // re-armed form.
    let cmd = app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.home.collection.value, "42");
    assert!(
        matches!(cmd, Some(AppCommand::CancelResolve)),
        "a cached id must kill the in-flight fetch without starting one, got {cmd:?}"
    );
    assert_eq!(
        app.home.resolved_collection,
        Some((42, vec![10, 20])),
        "the snapshot is back on the keystroke"
    );
    assert_eq!(app.home.resolved_folder_name.as_deref(), Some("Farm-42"));
}

/// A superseded request cannot touch the form, whatever it had to say.
///
/// Enumerated from `HomeResolveKind`'s own definition, not from memory: every
/// variant it declares appears below, each stamped with a retired generation and
/// each asserted inert against a form holding a full resolved snapshot. Before
/// the envelope this was three separate id-keyed guards added over three rounds,
/// and each round's fix left the next variant unguarded — `Cleared` last of all,
/// which has no id to key on at all.
///
/// A FIFTH variant is covered without touching this test, and that is the point:
/// `handle_home_resolve_event` compares the generation before it looks at `kind`,
/// so a new variant is guarded by construction rather than by its author
/// remembering to opt in. That property is structural and this test cannot
/// observe it — what this test can do is fail if the entry check is moved back
/// into the arms.
#[test]
fn a_superseded_request_cannot_touch_the_form_whatever_it_says() {
    let before = |home: &crate::app::HomeTab| {
        (
            home.resolved_collection.clone(),
            home.resolved_folder_name.clone(),
            home.resolved_enrich_pairs.clone(),
            home.collection_resolve.clone(),
        )
    };

    for kind in [
        HomeResolveKind::Loading,
        HomeResolveKind::Resolved {
            collection_id: 999,
            collection: collection(999, "Other", &[(70, &[707])]),
        },
        HomeResolveKind::Failed {
            reason: "collection not found".to_string(),
        },
        HomeResolveKind::Cleared,
    ] {
        let label = format!("{kind:?}");
        let mut home = home_awaiting(42);
        handle_home_resolve_event(
            at_live_gen(
                &home,
                HomeResolveKind::Resolved {
                    collection_id: 42,
                    collection: collection(42, "Farm", &[(10, &[101]), (20, &[202])]),
                },
            ),
            &mut home,
        );
        let expected = before(&home);
        assert!(
            expected.0.is_some(),
            "{label}: fixture must hold a snapshot"
        );

        // The generation an in-flight request is carrying, captured from the
        // form before a later request retires it — so this reds if the bump ever
        // stops happening, not only if the check does.
        let in_flight = home.resolve_generation();
        home.supersede_resolve();
        handle_home_resolve_event(at_gen(in_flight, kind), &mut home);

        assert_eq!(
            before(&home),
            expected,
            "{label}: a retired request wrote to the form"
        );
    }
}

/// A superseded response is dropped from the FORM but its payload is kept: the
/// fetch already happened, and the bytes are a fact about a collection id rather
/// than about the request that lost the race.
///
/// The banked id (999) is one the live path never handles in this test — the
/// form only ever resolves 42 — so a cache hit on 999 can have come from nowhere
/// but the superseded path. A fixture that banked the same id the live path
/// resolves would pass with the insert deleted and prove nothing.
///
/// Asserted through to the payoff rather than stopping at the cache: pointing the
/// field at 999 re-arms it with no request, which is the round-3 behaviour this
/// insert exists to feed. Losing it cost a full debounce plus refetch in exactly
/// the window the re-arm was built for.
#[test]
fn a_superseded_landing_banks_its_payload_without_touching_the_form() {
    let mut home = home_awaiting(42);
    handle_home_resolve_event(
        at_live_gen(
            &home,
            HomeResolveKind::Resolved {
                collection_id: 42,
                collection: collection(42, "Farm", &[(10, &[101]), (20, &[202])]),
            },
        ),
        &mut home,
    );
    let form_before = (
        home.resolved_collection.clone(),
        home.collection_resolve.clone(),
    );

    // 999's fetch landed a moment after the field moved off it.
    let in_flight = home.resolve_generation();
    home.supersede_resolve();
    handle_home_resolve_event(
        at_gen(
            in_flight,
            HomeResolveKind::Resolved {
                collection_id: 999,
                collection: collection(999, "Other", &[(70, &[707]), (80, &[808])]),
            },
        ),
        &mut home,
    );

    assert_eq!(
        (
            home.resolved_collection.clone(),
            home.collection_resolve.clone()
        ),
        form_before,
        "a superseded landing must not reach the form"
    );
    assert!(
        home.collection_cache.get_fresh(999).is_some(),
        "the payload it fetched is still a fact about 999"
    );

    // The payoff: 999 now re-arms on the keystroke instead of refetching.
    home.collection.set_value("999");
    assert!(
        home.settle_collection_resolve(),
        "a banked payload owes no request"
    );
    assert_eq!(home.resolved_collection, Some((999, vec![70, 80])));
    assert_eq!(home.resolved_folder_name.as_deref(), Some("Other-999"));
}

/// The scenario the guard exists for, driven through the keys: the cache re-arm
/// installs a snapshot synchronously, so a superseded request's cue now lands
/// AFTER the install rather than before it, and nothing later overwrites it —
/// the form would read `resolving…` over a resolved, armed collection until the
/// next keystroke. The keystroke that re-arms is also the one that retires the
/// request, which is what makes the cue inert.
#[test]
fn a_superseded_cue_cannot_strand_the_rearmed_line() {
    let mut app = App::new(Config::default());
    app.editing = true;
    app.home.focus = HomeField::Collection;
    app.home.collection.set_value("42");
    handle_home_resolve_event(
        at_live_gen(
            &app.home,
            HomeResolveKind::Resolved {
                collection_id: 42,
                collection: collection(42, "Farm", &[(10, &[101]), (20, &[202])]),
            },
        ),
        &mut app.home,
    );

    // Mistype, then backspace inside the debounce: the second keystroke re-arms
    // from the cache and cancels rather than schedules.
    app.handle_key(char_key('1'));
    // What 421's task is carrying, read off the form rather than computed — the
    // backspace below is what has to retire it.
    let in_flight = app.home.resolve_generation();
    let cmd = app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.home.collection.value, "42");
    assert!(
        matches!(cmd, Some(AppCommand::CancelResolve)),
        "a cache hit must still kill the fetch it is skipping, got {cmd:?}"
    );

    // 421's task got past its debounce before the abort reached it.
    handle_home_resolve_event(at_gen(in_flight, HomeResolveKind::Loading), &mut app.home);

    let Some((state, ref text)) = app.home.collection_resolve else {
        panic!("the re-armed collection's own line must still be there");
    };
    assert_eq!(
        state,
        ResolveState::Success,
        "a busy cue for 421 must not sit over a resolved 42: {text}"
    );
    assert!(text.contains("Farm"), "text = {text}");
    assert_eq!(app.home.resolved_collection, Some((42, vec![10, 20])));
}

/// handle_home_resolve_event with Cleared clears the resolve display.
#[test]
fn resolve_cleared_event_clears_state() {
    let config = Config::default();
    let mut home = crate::app::HomeTab::new(&config);
    home.set_collection_resolve(ResolveState::Success, "something");

    handle_home_resolve_event(at_live_gen(&home, HomeResolveKind::Cleared), &mut home);

    assert!(home.collection_resolve.is_none());
}

/// Singular map count uses "map" not "maps".
#[test]
fn resolve_single_map_uses_singular() {
    let mut home = home_awaiting(2);

    handle_home_resolve_event(
        at_live_gen(
            &home,
            HomeResolveKind::Resolved {
                collection_id: 2,
                collection: collection(2, "Solo", &[(11, &[101])]),
            },
        ),
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
