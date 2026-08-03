use crate::{
    app::{App, AppCommand, Tab},
    config::Config,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// The update-source arm carries the "skip already imported" toggle to the run.
/// This is the arm the toggle matters most on — an update scan reports exactly
/// the sets the user does not have installed, so "I already own this" is the
/// common case. Both legs vary only the config toggle, so a call site that
/// hardcodes either position reds one of them.
#[test]
fn selective_download_carries_the_skip_imported_toggle() {
    use crate::app::update_source::{MissingBeatmapset, MissingStatus};
    use crate::osu_db::LocalCollection;
    use std::collections::HashMap;

    fn update_request(skip_already_imported: bool) -> bool {
        let mut app = App::new(Config::default());
        app.config.skip_already_imported = skip_already_imported;
        app.home.directory.value = "/tmp/osu-collect-test".to_string();
        app.home.update.set_collections(vec![LocalCollection {
            name: "alpha - 100".to_string(),
            beatmap_checksums: Box::new([]),
        }]);
        app.home.update.set_missing_beatmaps(
            vec![MissingBeatmapset {
                id: 10,
                status: MissingStatus::NotInstalled,
                collection_id: 100,
                collection_name: "alpha".to_string(),
                included: true,
                previously_deleted: false,
                checksums: Box::new([]),
                enrich_diff_id: None,
            }],
            &HashMap::new(),
        );

        let (_, request) = app
            .request_selective_download()
            .expect("a selected collection with mirrors enabled builds a request");
        request.skip_already_imported
    }

    assert!(update_request(true));
    assert!(!update_request(false));
}

#[test]
fn right_tab_switch_from_home_off_a_non_text_field() {
    use crate::app::HomeField;
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Home;
    // Focus a non-text field so Right switches tabs rather than moving the caret.
    app.home.focus = HomeField::Video;

    app.handle_key(key(KeyCode::Right));

    // Three static tabs: Home → Downloads.
    assert_eq!(app.active_tab, Tab::Downloads);
}

#[test]
fn left_tab_switch_to_home_probes_mirrors() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Downloads;

    let cmd = app.handle_key(key(KeyCode::Left));

    assert_eq!(app.active_tab, Tab::Home);
    assert!(matches!(cmd, Some(AppCommand::ProbeMirrors)));
}

#[test]
fn arrow_stays_on_home_while_browsing_updates() {
    use crate::app::GetMapsSource;
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Home;
    app.home.source = GetMapsSource::Update;
    app.home.update.descend();

    // In browse, → focuses a pane instead of switching tabs.
    let cmd = app.handle_key(key(KeyCode::Right));

    assert_eq!(app.active_tab, Tab::Home);
    assert!(cmd.is_none());
}

#[test]
fn u_opens_update_modal_only_when_available() {
    use crate::app::HomeField;
    use crate::auto_update::AvailableUpdate;
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Home;
    // Non-text field so `u` fires as a hotkey rather than typed input.
    app.home.focus = HomeField::Video;

    assert!(app.handle_key(key(KeyCode::Char('u'))).is_none());
    assert!(app.update_modal.is_none());

    app.set_available_update(AvailableUpdate {
        version: "9.9.9".into(),
        name: "v9.9.9".into(),
        changelog: "- cool stuff".into(),
    });
    assert!(app.handle_key(key(KeyCode::Char('u'))).is_none());
    assert!(app.update_modal.is_some());
}

#[test]
fn update_modal_confirm_returns_start_update_and_clears_banner() {
    use crate::app::HomeField;
    use crate::auto_update::AvailableUpdate;
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Home;
    app.home.focus = HomeField::Video;
    app.set_available_update(AvailableUpdate {
        version: "9.9.9".into(),
        name: "v9.9.9".into(),
        changelog: String::new(),
    });
    app.handle_key(key(KeyCode::Char('u')));
    // default focus = update; Enter confirms.
    let cmd = app.handle_key(key(KeyCode::Enter));
    assert!(matches!(cmd, Some(AppCommand::StartUpdate)));
    assert!(app.update_modal.is_none());
    assert!(app.available_update.is_none());
}

#[test]
fn update_modal_later_closes_and_keeps_availability() {
    use crate::app::HomeField;
    use crate::auto_update::AvailableUpdate;
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Home;
    app.home.focus = HomeField::Video;
    app.set_available_update(AvailableUpdate {
        version: "9.9.9".into(),
        name: "v9.9.9".into(),
        changelog: String::new(),
    });
    app.handle_key(key(KeyCode::Char('u')));
    app.handle_key(key(KeyCode::Left)); // move focus to `later`
    let cmd = app.handle_key(key(KeyCode::Enter));
    assert!(cmd.is_none());
    assert!(app.update_modal.is_none());
    assert!(app.available_update.is_some());
}

// ── find_size_probe_cmd gate (phase 5 nekoha size backfill) ─────────────────
//
// `find_size_probe_cmd` is private; this test module is a descendant of
// `state.rs` (linked via `#[path]`), so private methods are directly callable —
// exercising the gate this way pins the three-way condition itself rather than
// routing it through a specific keypress, so a regression that drops any leg of
// `source == Find && results_backend == Osu` fails here regardless of which key
// site forgot to check it.

#[test]
fn size_probe_cmd_fires_only_for_find_osu() {
    use crate::app::{FindBackend, GetMapsSource};
    let mut app = App::new(Config::default());
    app.home.source = GetMapsSource::Find;
    app.home.find.note_results_backend(FindBackend::Osu);
    assert!(matches!(
        app.find_size_probe_cmd(),
        Some(AppCommand::ProbeFindSizes)
    ));
}

#[test]
fn size_probe_cmd_is_none_for_find_nzbasic() {
    use crate::app::{FindBackend, GetMapsSource};
    let mut app = App::new(Config::default());
    app.home.source = GetMapsSource::Find;
    app.home.find.note_results_backend(FindBackend::Nzbasic);
    assert!(app.find_size_probe_cmd().is_none());
}

#[test]
fn size_probe_cmd_is_none_for_collection_source() {
    use crate::app::GetMapsSource;
    let mut app = App::new(Config::default());
    app.home.source = GetMapsSource::Collection;
    assert!(app.find_size_probe_cmd().is_none());
}

#[test]
fn size_probe_cmd_is_none_for_update_source() {
    use crate::app::GetMapsSource;
    let mut app = App::new(Config::default());
    app.home.source = GetMapsSource::Update;
    assert!(app.find_size_probe_cmd().is_none());
}

#[test]
fn size_probe_cmd_is_none_for_find_with_no_recorded_backend() {
    // A fresh find source with no fetch yet — `results_backend()` is `None`,
    // which must not be mistaken for the osu route.
    use crate::app::GetMapsSource;
    let mut app = App::new(Config::default());
    app.home.source = GetMapsSource::Find;
    assert!(app.find_size_probe_cmd().is_none());
}

#[test]
fn page_down_reaches_a_focused_find_browse_preview() {
    use crate::app::find_source::BrowseRow;
    use crate::app::home::GetMapsSource;
    use std::collections::HashMap;

    let mut app = App::new(Config::default());
    app.active_tab = Tab::Home;
    app.home.source = GetMapsSource::Find;
    app.home
        .find
        .browse
        .set_rows(vec![BrowseRow { id: 1, meta: None }], &HashMap::new());
    app.home.find.browse.descend();
    app.home.find.browse.focus_preview();

    app.handle_key(key(KeyCode::PageDown));

    assert_eq!(
        app.home.find.browse.preview_offset.get(),
        10,
        "the page key has to reach the browse, not stop at a tab-level handler"
    );
}

// ── the supporter gate closing ────────────────────────────────────────────────

/// A supporter mid-edit: `special = farm` (not gated, so it survives) plus a
/// gated `genre`, with focus parked on the genre row. The pair is the exact
/// repro — `farm` forces nzbasic and `genre` forces osu, so a `genre` left
/// standing after the gate closes names a field with no row on screen.
fn supporter_mid_edit() -> App {
    use crate::app::{GetMapsSource, HomeField};
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Home;
    app.home.source = GetMapsSource::Find;
    app.set_login_complete(true);
    app.home.find.cycle_special(true); // farm — nzbasic forcer, not gated
    app.home.find.cycle_genre(true); // a gated osu forcer
    app.home.focus = HomeField::FindGenre;
    app
}

/// Positive control for the two tests below: the same fixture, same dimension
/// (the supporter flag), a transition that LEAVES the gate open. Without this a
/// clear-on-every-transition would read as a passing gate test.
#[test]
fn a_transition_that_keeps_supporter_keeps_the_facets() {
    use crate::app::{FindRoute, HomeField};
    let mut app = supporter_mid_edit();
    app.set_login_complete(true);
    assert_eq!(app.home.find.genre_label(), "unspecified");
    assert_eq!(app.home.focus, HomeField::FindGenre);
    assert!(matches!(
        app.home.find.resolved_route(),
        FindRoute::Conflict { .. }
    ));
}

/// Both doors that shut the gate must clear only the two still-gated facets
/// (rank, played). The four ungated facets (genre, language, extra, explicit)
/// were confirmed to work for non-supporters on 2026-08-03, so clearing them
/// would silently undo a deliberate choice. The route and disclosure stay,
/// reflecting that those ungated facets are still active osu-forcers.
#[test]
fn the_gate_closing_clears_only_gated_facets_and_leaves_ungated_alone() {
    use crate::app::{FindRoute, HomeField};
    for (door, close) in [
        ("logout", App::set_logged_out as fn(&mut App)),
        ("failed login", App::set_login_failed as fn(&mut App)),
    ] {
        let mut app = supporter_mid_edit();
        assert!(app.config.supporter());
        assert_ne!(app.home.find.genre_label(), "any", "{door}: precondition");
        assert!(
            app.home.find.show_advanced_filters(),
            "{door}: precondition"
        );
        // Set a rank filter too — this one IS gated and must be cleared.
        app.home.find.rank.toggle();

        close(&mut app);

        // Genre is ungated: stays as-is. A non-supporter CAN set it, and it
        // still forces osu, so the route stays Conflict (vs the farm special).
        assert_eq!(
            app.home.find.genre_label(),
            "unspecified",
            "{door}: genre persists"
        );
        assert!(
            matches!(app.home.find.resolved_route(), FindRoute::Conflict { .. }),
            "{door}: farm+genre still conflict; genre was not cleared"
        );
        // Rank IS gated: cleared.
        assert!(app.home.find.rank.is_empty(), "{door}: rank cleared");
        // Focus: genre is no longer supporter-only, so no clamp — focus stays.
        assert_eq!(
            app.home.focus,
            HomeField::FindGenre,
            "{door}: focus stays on an ungated row"
        );
    }
}

/// The settle runs at the FLIP, not at the next keypress. Focus stays on genre
/// (it is no longer supporter-only after the re-classification), and the
/// ungated facet keeps its value — a non-supporter CAN set it.
#[test]
fn the_gate_settles_without_waiting_for_a_keypress() {
    use crate::app::HomeField;
    let mut app = supporter_mid_edit();
    assert_eq!(app.home.focus, HomeField::FindGenre, "precondition");
    app.set_logged_out();
    assert_eq!(
        app.home.focus,
        HomeField::FindGenre,
        "focus stays — genre is no longer supporter-only, so no clamp fires"
    );
    assert_eq!(
        app.home.find.genre_label(),
        "unspecified",
        "genre persists — a non-supporter CAN set it"
    );
}

// ── the find route moving off the loaded results ──────────────────────────────
//
// Model-level coverage of the invalidation itself is in `app_find_source.rs`.
// These pin the two ways it reaches the user: the key loop, and the auth event
// that resets the supporter facets with no keypress behind it.

/// The find form as a landed nzbasic run leaves it: the `special` chip that
/// forced the route, rows, both picked, and the recorded backend. Focus parked
/// on that chip, which is what the user walks back to clear it.
fn find_with_nzbasic_results() -> App {
    use crate::app::find_source::BrowseRow;
    use crate::app::{FindBackend, GetMapsSource, HomeField};
    use std::collections::HashMap;

    let mut app = App::new(Config::default());
    app.active_tab = Tab::Home;
    app.home.source = GetMapsSource::Find;
    app.home.find.cycle_special(true); // farm — the nzbasic forcer
    app.home.find.browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
        ],
        &HashMap::new(),
    );
    app.home.find.browse.set_all_selected(true);
    app.home.find.note_results_backend(FindBackend::Nzbasic);
    app.home.find.mark_results_current();
    app.home.focus = HomeField::FindSpecial;
    app
}

/// Clearing the chip through the key handler drops the rows AND everything the
/// download button reads, in the same press. The button's enabled state and its
/// `download (N)` count both come off the selection, so a clear that left either
/// standing would put the form back to advertising a run it can no longer make.
///
/// The walk back to `none` passes through the other nzbasic-forcing values,
/// which are not route moves — the loop asserts the results survive those, so a
/// settle that fired on any criteria edit rather than a route change reds here.
#[test]
fn clearing_the_forcing_chip_resets_the_download_button() {
    use crate::app::{FindBackend, HomeField};
    let mut app = find_with_nzbasic_results();
    assert!(
        app.home
            .button_enabled(HomeField::Download, app.osu_official_unlocked()),
        "precondition: two picked rows"
    );
    assert!(
        app.home
            .button_enabled(HomeField::FindBrowse, app.osu_official_unlocked()),
        "precondition: fresh results to reopen"
    );

    for _ in 0..8 {
        if app.home.find.special_label() == "none" {
            break;
        }
        app.handle_key(key(KeyCode::Char(' ')));
        if app.home.find.special_label() != "none" {
            assert_eq!(
                app.home.find.browse.rows.len(),
                2,
                "{} still forces nzbasic — not a route move",
                app.home.find.special_label()
            );
        }
    }
    assert_eq!(app.home.find.special_label(), "none", "chip never cleared");

    assert!(app.home.find.browse.rows.is_empty(), "rows");
    assert_eq!(app.home.find.browse.selected_count(), 0, "checks");
    assert!(
        !app.home
            .button_enabled(HomeField::Download, app.osu_official_unlocked()),
        "the button must not stay live over results that are gone"
    );
    assert!(
        !app.home
            .button_enabled(HomeField::FindBrowse, app.osu_official_unlocked()),
        "`view N mapsets` has nothing to reopen"
    );
    assert_eq!(
        app.home.find.checked_known_bytes(),
        0,
        "the button's `· ~X` size suffix sums the checked sets"
    );
    assert_eq!(
        app.home.find.run_backend(),
        FindBackend::Osu,
        "the dispatch now names the backend the indicator shows"
    );
    // Rows vanishing with no explanation is its own defect, so the cue names
    // what went and the way back — and names the backend the indicator does.
    let toast = app
        .toasts
        .iter()
        .next()
        .expect("no cue for the dropped rows");
    assert_eq!(toast.title(), "find results cleared");
    assert_eq!(
        toast.detail(),
        Some("criteria now route via osu! api · run find again")
    );
}

/// The controller-level negative: the same fixture and the same kind of key
/// (a chip cycle), differing only in whether the chip it moves steers the route.
#[test]
fn cycling_a_route_neutral_chip_keeps_the_download_button_live() {
    use crate::app::HomeField;
    let mut app = find_with_nzbasic_results();
    // `mode` is expressible on both backends, so `special = farm` still decides.
    app.home.focus = HomeField::FindMode;
    app.handle_key(key(KeyCode::Char(' ')));

    assert_eq!(app.home.find.browse.rows.len(), 2, "rows");
    assert_eq!(app.home.find.browse.selected_count(), 2, "checks");
    assert!(
        app.home
            .button_enabled(HomeField::Download, app.osu_official_unlocked())
    );
    assert!(
        app.toasts.is_empty(),
        "nothing was lost, so nothing to announce"
    );
}

/// The guarantee that does not rest on a convention: the check sits where every
/// find dispatch converges, so a drifted route refuses the run whatever reached
/// it. Today no key both edits the criteria and dispatches in one press — the
/// compiler does not enforce that, and a handler that did would otherwise tag the
/// run with a backend the form stopped showing and drop it in that backend's
/// directory. The criteria are moved here WITHOUT the key loop, standing in for
/// exactly that handler.
#[test]
fn a_find_dispatch_cannot_run_on_a_route_the_form_left() {
    let mut app = find_with_nzbasic_results();
    app.home.directory.value = "/tmp/osu-collect-test".to_string();
    assert!(
        app.request_find_download().is_some(),
        "precondition: the run dispatches while the route still matches"
    );
    let queued = app.downloads.len();

    let mut app = find_with_nzbasic_results();
    app.home.directory.value = "/tmp/osu-collect-test".to_string();
    app.home.find.cycle_special(false); // back to `none` — the route is osu now
    assert!(
        app.request_find_download().is_none(),
        "a run tagged with the backend the form left must not dispatch"
    );
    assert_eq!(app.downloads.len(), queued - 1, "no run page was pushed");
}

/// The one path a keypress cannot cover: an auth event closing the supporter
/// gate resets the six facets, every one of them an osu-forcer, which can hand
/// the route to nzbasic on its own. The conflict leg on the way in is the
/// carve-out — a form that cleared its results there would lose them to the
/// first keystroke of this very edit.
#[test]
fn the_supporter_gate_closing_settles_the_find_route() {
    use crate::app::find_source::BrowseRow;
    use crate::app::{FindBackend, FindRoute, GetMapsSource, HomeField};
    use std::collections::HashMap;

    let mut app = App::new(Config::default());
    app.active_tab = Tab::Home;
    app.home.source = GetMapsSource::Find;
    app.set_login_complete(true);
    app.home.find.cycle_genre(true); // genre → osu
    app.home
        .find
        .browse
        .set_rows(vec![BrowseRow { id: 10, meta: None }], &HashMap::new());
    app.home.find.browse.set_all_selected(true);
    app.home.find.note_results_backend(FindBackend::Osu);
    app.home.find.mark_results_current();

    // `farm` over the facet is a conflict, so the results ride through it.
    app.home.focus = HomeField::FindSpecial;
    app.handle_key(key(KeyCode::Char(' ')));
    assert!(matches!(
        app.home.find.resolved_route(),
        FindRoute::Conflict { .. }
    ));
    assert_eq!(
        app.home.find.browse.rows.len(),
        1,
        "a conflict is not a move"
    );

    // No key from here. Genre is ungated (2026-08-03), so it stays set — the
    // route settle fires but finds Conflict (farm + genre still clash) and
    // returns early. The results survive because the criteria have not changed
    // in a way that moves the route.
    app.set_logged_out();
    assert!(
        matches!(app.home.find.resolved_route(), FindRoute::Conflict { .. }),
        "farm+genre still conflict; genre was not cleared"
    );
    assert_eq!(
        app.home.find.browse.rows.len(),
        1,
        "a conflict is not a move — results survive"
    );
    assert_eq!(
        app.home.find.results_backend(),
        Some(FindBackend::Osu),
        "backend unchanged"
    );
    // Results survive, non-osu-official mirrors are available, and rows are
    // selected — the download button stays enabled.
    assert!(
        app.home
            .button_enabled(HomeField::Download, app.osu_official_unlocked())
    );
}

/// The osu! official mirror is auth-gated: the download run drops it when there
/// is no valid login. The mirror count, the built list, and the button's
/// enabled-state must all agree with that — counting it while logged out
/// advertises a run that creates an output directory and then dies with no
/// mirrors.
#[test]
fn osu_official_excluded_from_count_and_button_while_logged_out() {
    use crate::app::find_source::BrowseRow;
    use crate::app::{FindBackend, GetMapsSource, HomeField};
    use crate::auth::AUTH_ENV_PATH;
    use crate::mirrors::MirrorKind;
    use crate::test_env::TempEnvVar;
    use std::collections::HashMap;

    // Point auth at a nonexistent path so App::new starts logged out regardless
    // of the developer's real stored login.
    let _auth = TempEnvVar::set(AUTH_ENV_PATH, "/dev/null/no-such-auth");
    let mut app = App::new(Config::default());
    // Starts logged out (no stored auth). Enable ONLY osu! official.
    app.home.nerinyan = false;
    app.home.osu_direct = false;
    app.home.sayobot = false;
    app.home.nekoha = false;
    app.home.beatconnect = false;
    app.home.osudl = false;
    app.home.catboy = false;
    app.home.osu_official = true;

    // Give the find arm a selection so the button's only obstacle is the mirror
    // gate. Seed rows the production way: a backend + currency, not bare
    // `set_rows` that leaves `results_backend == None` (a test-only state).
    app.home.source = GetMapsSource::Find;
    app.home
        .find
        .browse
        .set_rows(vec![BrowseRow { id: 10, meta: None }], &HashMap::new());
    app.home.find.browse.set_all_selected(true);
    app.home.find.note_results_backend(FindBackend::Osu);
    app.home.find.mark_results_current();

    assert_eq!(
        app.home.mirror_count(app.osu_official_unlocked()),
        0,
        "osu! official must not count while logged out"
    );
    assert!(
        app.home
            .build_mirror_list(app.osu_official_unlocked())
            .is_empty(),
        "build_mirror_list must omit the auth-gated mirror while logged out"
    );
    assert!(
        !app.home
            .button_enabled(HomeField::Download, app.osu_official_unlocked()),
        "button must follow the effective mirror list, not the raw toggle"
    );

    // log in: the mirror unlocks
    app.set_login_complete(true);
    assert_eq!(
        app.home.mirror_count(app.osu_official_unlocked()),
        1,
        "unlocked, it counts"
    );
    let mirrors = app.home.build_mirror_list(app.osu_official_unlocked());
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].kind(), MirrorKind::OsuApi);
    assert!(
        app.home
            .button_enabled(HomeField::Download, app.osu_official_unlocked()),
        "with a valid login the button is live"
    );

    // log out again: the mirror goes dark
    app.set_logged_out();
    assert_eq!(
        app.home.mirror_count(app.osu_official_unlocked()),
        0,
        "logging out must drop the auth-gated mirror from the count"
    );
    assert!(
        app.home
            .build_mirror_list(app.osu_official_unlocked())
            .is_empty()
    );
    assert!(
        !app.home
            .button_enabled(HomeField::Download, app.osu_official_unlocked()),
        "button must die again on logout"
    );
}

/// D1: a stored token that loads and deserializes but lacks the `*` (lazer-tier)
/// scope still yields `login_state == LoggedIn`, so the old predicate lit the
/// button and counted the mirror. The run would then create an output directory
/// and die with `NoMirrors` — the exact defect this pins. `has_lazer_scope` is
/// the gate, not mere token presence.
#[test]
fn osu_official_stays_dead_with_a_narrow_scope_token() {
    use crate::app::find_source::BrowseRow;
    use crate::app::{AuthLoginState, FindBackend, GetMapsSource, HomeField};
    use crate::auth::{AUTH_ENV_PATH, StoredAuth};
    use crate::test_env::TempEnvVar;
    use std::collections::HashMap;

    // A token that loads fine but carries only `public` scope — no download
    // privilege. Built through `StoredAuth` so a field rename is a compile error.
    let narrow = StoredAuth {
        client_id: "5".to_string(),
        client_secret: "secret".to_string(),
        redirect_uri: String::new(),
        access_token: "token".to_string(),
        refresh_token: None,
        expires_at: u64::MAX,
        scopes: vec!["public".to_string()],
        supporter: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    std::fs::write(&path, serde_json::to_string(&narrow).unwrap()).unwrap();
    let _auth = TempEnvVar::set(AUTH_ENV_PATH, path.to_str().unwrap());

    let mut app = App::new(Config::default());
    // Logged in (token loads) but `osu_official_unlocked` must be false: the
    // scope gate catches what `login_state` alone missed.
    assert!(
        matches!(app.config.login_state, AuthLoginState::LoggedIn),
        "precondition: the token loads, so login_state is LoggedIn"
    );
    assert!(
        !app.osu_official_unlocked(),
        "a narrow-scope token must not unlock the official mirror"
    );

    app.home.nerinyan = false;
    app.home.osu_direct = false;
    app.home.sayobot = false;
    app.home.nekoha = false;
    app.home.beatconnect = false;
    app.home.osudl = false;
    app.home.catboy = false;
    app.home.osu_official = true;

    app.home.source = GetMapsSource::Find;
    app.home
        .find
        .browse
        .set_rows(vec![BrowseRow { id: 10, meta: None }], &HashMap::new());
    app.home.find.browse.set_all_selected(true);
    app.home.find.note_results_backend(FindBackend::Osu);
    app.home.find.mark_results_current();

    assert_eq!(
        app.home.mirror_count(app.osu_official_unlocked()),
        0,
        "a narrow-scope token must leave the count at zero"
    );
    assert!(
        !app.home
            .button_enabled(HomeField::Download, app.osu_official_unlocked()),
        "the button must stay dead — the run would die with no mirrors"
    );
}
