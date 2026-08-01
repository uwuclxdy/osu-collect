/// Keybind dispatch tests.
///
/// Verifies that key events produce the expected AppCommand or state change
/// without running the full runtime.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use osu_collect::{
    app::{App, AppCommand},
    config::Config,
};

fn make_app() -> App {
    App::new(Config::default())
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn shift(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::SHIFT,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

// ── ctrl shortcuts ────────────────────────────────────────────────────────────

#[test]
fn ctrl_c_quits() {
    let mut app = make_app();
    let cmd = app.handle_key(ctrl(KeyCode::Char('c')));
    assert!(matches!(cmd, Some(AppCommand::Quit)));
}

#[test]
fn ctrl_w_deletes_word_in_text_field() {
    let mut app = make_app();
    app.editing = true; // word-delete edits only in edit mode
    // set_value parks the caret at the end so word-delete acts on "world".
    app.home.collection.set_value("hello world");
    app.handle_key(ctrl(KeyCode::Char('w')));
    assert_eq!(app.home.collection.value, "hello ");
}

#[test]
fn ctrl_backspace_as_ctrl_h_deletes_word_not_types_h() {
    // many terminals deliver ctrl+backspace as ^H (ctrl+h)
    let mut app = make_app();
    app.editing = true;
    app.home.collection.set_value("hello world");
    app.handle_key(ctrl(KeyCode::Char('h')));
    assert_eq!(
        app.home.collection.value, "hello ",
        "ctrl+h must delete a word, never type a literal 'h'"
    );
}

// ── tab navigation ────────────────────────────────────────────────────────────

#[test]
fn right_arrow_moves_to_next_tab() {
    use osu_collect::app::{HomeField, Tab};
    let mut app = make_app();
    app.home.focus = HomeField::Video; // non-text so ←/→ switch screens
    assert_eq!(app.active_tab(), Tab::Home);
    app.handle_key(press(KeyCode::Right));
    assert_eq!(app.active_tab(), Tab::Downloads);
}

#[test]
fn left_arrow_wraps_to_last_tab() {
    use osu_collect::app::{HomeField, Tab};
    let mut app = make_app();
    app.home.focus = HomeField::Video;
    app.handle_key(press(KeyCode::Left));
    // wraps to the last static tab (2 = config) since no downloads
    assert_eq!(app.active_tab(), Tab::Config);
}

#[test]
fn tab_and_backtab_switch_screens() {
    use osu_collect::app::{HomeField, Tab};
    let mut app = make_app();
    app.home.focus = HomeField::Video;
    assert_eq!(app.active_tab(), Tab::Home);
    app.handle_key(press(KeyCode::Tab));
    assert_eq!(
        app.active_tab(),
        Tab::Downloads,
        "tab cycles to the next tab"
    );
    app.handle_key(press(KeyCode::BackTab));
    assert_eq!(
        app.active_tab(),
        Tab::Home,
        "shift+tab cycles to the previous tab"
    );
}

#[test]
fn tab_completes_directory_while_editing_it() {
    use osu_collect::app::{HomeField, Tab};
    let mut app = make_app();
    // Editing the directory field: tab completes the path, never switches tabs.
    app.home.focus = HomeField::Directory;
    app.editing = true;
    app.handle_key(press(KeyCode::Tab));
    assert_eq!(
        app.active_tab(),
        Tab::Home,
        "tab must complete the path, not switch tabs, while editing the directory"
    );
}

#[test]
fn s_falls_back_to_download_button_when_no_button_is_enabled() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    // Default focus is the collection text field, selected-not-editing. A fresh
    // collection form has no resolved collection and no picked subset, so `view N
    // maps` and `download` are both disabled — `s` falls back to the download
    // button.
    assert_eq!(app.home.focus, HomeField::Collection);
    app.handle_key(press(KeyCode::Char('s')));
    assert_eq!(
        app.home.focus,
        HomeField::Download,
        "s falls back to the download button when nothing is clickable"
    );
}

#[test]
fn s_jumps_to_last_enabled_button_per_source() {
    use osu_collect::app::{GetMapsSource, HomeField};
    // `s` lands on the furthest-along *enabled* button. On a fresh form that is
    // the primary CTA: find lands on `find`, update on `scan` (both enabled while
    // idle); their `view N maps` and `download` buttons are still disabled.
    for (source, expected) in [
        (GetMapsSource::Find, HomeField::FindRun),
        (GetMapsSource::Update, HomeField::UpdateScan),
    ] {
        let mut app = make_app();
        app.home.source = source;
        app.home.focus = HomeField::Source;
        app.handle_key(press(KeyCode::Char('s')));
        assert_eq!(
            app.home.focus, expected,
            "s jumps to the last enabled button on the {source:?} source"
        );
    }
}

#[test]
fn s_cycles_the_other_enabled_buttons_when_already_on_one() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Collection;
    // Non-empty collection + default mirrors → download enabled; a resolved
    // collection → `view N maps` enabled. Enabled buttons in field order are
    // [CollectionBrowse, Download].
    app.home.collection.set_value("123");
    app.home.set_resolved_collection(7, vec![10, 20, 30]);
    app.home.focus = HomeField::Collection;
    // First `s` (not on a button) jumps to the last enabled button.
    app.handle_key(press(KeyCode::Char('s')));
    assert_eq!(app.home.focus, HomeField::Download);
    // Repeat `s` cycles to the other enabled button, then wraps back.
    app.handle_key(press(KeyCode::Char('s')));
    assert_eq!(app.home.focus, HomeField::CollectionBrowse);
    app.handle_key(press(KeyCode::Char('s')));
    assert_eq!(app.home.focus, HomeField::Download);
}

#[test]
fn s_types_literally_while_editing() {
    let mut app = make_app();
    app.editing = true;
    app.handle_key(press(KeyCode::Char('s')));
    assert_eq!(
        app.home.collection.value, "s",
        "while editing, s types into the field instead of jumping"
    );
}

// ── quit key ─────────────────────────────────────────────────────────────────

#[test]
fn q_on_home_tab_shows_toast_first() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    // q only quits off a text field; the default focus is the collection input
    app.home.focus = HomeField::Video;
    let cmd = app.handle_key(press(KeyCode::Char('q')));
    assert!(cmd.is_none(), "first q must not quit immediately");
    assert!(app.home.quit_prompt, "first q must set the quit toast");
}

#[test]
fn q_twice_on_home_tab_quits() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Video;
    app.handle_key(press(KeyCode::Char('q')));
    let cmd = app.handle_key(press(KeyCode::Char('q')));
    assert!(matches!(cmd, Some(AppCommand::Quit)));
}

#[test]
fn q_while_editing_collection_field_types_instead_of_quitting() {
    let mut app = make_app();
    // collection field is focused by default; enter edit mode so q types
    app.editing = true;
    app.handle_key(press(KeyCode::Char('q')));
    assert!(
        !app.home.quit_prompt,
        "q must not quit while editing a field"
    );
    assert_eq!(app.home.collection.value, "q", "q must type into the field");
}

#[test]
fn esc_on_home_tab_never_quits_or_arms() {
    let mut app = make_app();
    let cmd = app.handle_key(press(KeyCode::Esc));
    assert!(cmd.is_none(), "esc on a static tab must not quit");
    assert!(
        !app.home.quit_prompt,
        "esc is back-only; it must never arm the quit prompt"
    );
}

#[test]
fn esc_twice_on_home_tab_never_quits() {
    let mut app = make_app();
    let first = app.handle_key(press(KeyCode::Esc));
    let second = app.handle_key(press(KeyCode::Esc));
    assert!(first.is_none() && second.is_none(), "esc must never quit");
    assert!(!app.home.quit_prompt);
}

#[test]
fn esc_cancels_a_q_armed_quit_prompt() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Video; // non-text so `q` is a hotkey
    app.handle_key(press(KeyCode::Char('q')));
    assert!(app.home.quit_prompt, "first q must arm the quit prompt");

    let cmd = app.handle_key(press(KeyCode::Esc));
    assert!(cmd.is_none(), "esc must not quit after a q-armed prompt");
    assert!(
        !app.home.quit_prompt,
        "esc must cancel the armed quit prompt"
    );
}

// ── field navigation ──────────────────────────────────────────────────────────

#[test]
fn down_moves_field_focus() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    assert_eq!(app.home.focus, HomeField::Collection);
    app.handle_key(press(KeyCode::Down));
    assert_ne!(app.home.focus, HomeField::Collection);
}

#[test]
fn up_from_first_field_wraps_to_last() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    // The source strip is the first focusable row; Up from it wraps to the last
    // collection field (the download button, tail of the shared download section).
    app.home.focus = HomeField::Source;
    app.handle_key(press(KeyCode::Up));
    assert_eq!(app.home.focus, HomeField::Download);
}

#[test]
fn up_from_collection_focuses_source_strip() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    // Default focus is the collection field; the strip sits directly above it.
    assert_eq!(app.home.focus, HomeField::Collection);
    app.handle_key(press(KeyCode::Up));
    assert_eq!(app.home.focus, HomeField::Source);
}

#[test]
fn space_and_enter_cycle_source_when_strip_focused() {
    use osu_collect::app::{GetMapsSource, HomeField, Tab};
    let mut app = make_app();
    app.home.focus = HomeField::Source;
    assert_eq!(app.home.source, GetMapsSource::Collection);
    // space / enter cycle forward, wrapping; the tab never changes. Arrows no
    // longer touch the source. Strip order is [find, collection, update].
    app.handle_key(press(KeyCode::Char(' ')));
    assert_eq!(app.home.source, GetMapsSource::Update);
    app.handle_key(press(KeyCode::Enter));
    assert_eq!(app.home.source, GetMapsSource::Find);
    assert_eq!(app.active_tab(), Tab::Home);
}

#[test]
fn arrows_switch_tab_from_the_strip() {
    use osu_collect::app::{GetMapsSource, HomeField, Tab};
    let mut app = make_app();
    // On the strip, ←/→ switch tabs (they no longer cycle the source).
    app.home.focus = HomeField::Source;
    app.handle_key(press(KeyCode::Right));
    assert_ne!(app.active_tab(), Tab::Home);
    assert_eq!(
        app.home.source,
        GetMapsSource::Collection,
        "arrows leave the source unchanged"
    );
}

#[test]
fn arrows_switch_tab_off_the_strip() {
    use osu_collect::app::{HomeField, Tab};
    let mut app = make_app();
    // Off the strip (collection field, not editing), ←/→ still switch tabs.
    app.home.focus = HomeField::Collection;
    app.handle_key(press(KeyCode::Right));
    assert_ne!(app.active_tab(), Tab::Home);
}

#[test]
fn find_source_down_focuses_first_field() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.focus = HomeField::Source;
    // One union form, no backend chip: the first row under the strip is the
    // free-text query input; Down lands there.
    app.handle_key(press(KeyCode::Down));
    assert_eq!(app.home.focus, HomeField::FindQuery);
    // (`s` jumps to the last enabled button per source — covered by
    // `s_jumps_to_last_enabled_button_per_source`. `d` stays collection-only.)
}

#[test]
fn switching_source_preserves_collection_input() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.collection.set_value("12345");
    // Cycle all the way around with space; keep-both keeps the input alive.
    app.home.focus = HomeField::Source;
    app.handle_key(press(KeyCode::Char(' '))); // collection → update
    app.handle_key(press(KeyCode::Char(' '))); // update → find
    app.handle_key(press(KeyCode::Char(' '))); // find → collection
    assert_eq!(app.home.source, GetMapsSource::Collection);
    assert_eq!(app.home.collection.value, "12345");
}

#[test]
fn digit_jumps_to_indexed_source() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.focus = HomeField::Collection;
    // `1` selects the first strip source, `2` the second; each returns focus to
    // the strip. Generic over ALL so it tracks the strip order.
    app.handle_key(press(KeyCode::Char('1')));
    assert_eq!(app.home.source, GetMapsSource::ALL[0]);
    assert_eq!(app.home.focus, HomeField::Source);
    app.handle_key(press(KeyCode::Char('2')));
    assert_eq!(app.home.source, GetMapsSource::ALL[1]);
}

#[test]
fn digit_does_not_jump_while_editing() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    // Editing the collection field: a digit types in, it never jumps the source.
    app.editing = true;
    app.handle_key(press(KeyCode::Char('2')));
    assert_eq!(app.home.source, GetMapsSource::Collection);
    assert_eq!(app.home.collection.value, "2");
}

#[test]
fn source_strip_cycles_three_sources() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.focus = HomeField::Source;
    // Three sources now (find / collection / update); space visits each and wraps.
    assert_eq!(GetMapsSource::ALL.len(), 3);
    let start = app.home.source;
    app.handle_key(press(KeyCode::Char(' ')));
    app.handle_key(press(KeyCode::Char(' ')));
    app.handle_key(press(KeyCode::Char(' ')));
    assert_eq!(
        app.home.source, start,
        "three space presses wrap back to start"
    );
}

// ── character input ───────────────────────────────────────────────────────────

#[test]
fn typing_into_collection_field_updates_value() {
    let mut app = make_app();
    // collection is focused by default; enter edit mode so keys type
    app.editing = true;
    app.handle_key(press(KeyCode::Char('1')));
    app.handle_key(press(KeyCode::Char('2')));
    app.handle_key(press(KeyCode::Char('3')));
    assert_eq!(app.home.collection.value, "123");
}

#[test]
fn backspace_removes_last_char() {
    let mut app = make_app();
    app.editing = true;
    app.handle_key(press(KeyCode::Char('a')));
    app.handle_key(press(KeyCode::Char('b')));
    app.handle_key(press(KeyCode::Backspace));
    assert_eq!(app.home.collection.value, "a");
}

// ── bracketed paste ───────────────────────────────────────────────────────────

#[test]
fn paste_into_collection_field_inserts_and_resolves() {
    let mut app = make_app();
    app.editing = true; // paste only types while editing
    let cmd = app.handle_paste("12345".to_string());
    assert_eq!(app.home.collection.value, "12345");
    assert!(
        matches!(cmd, Some(AppCommand::ResolveCollectionUrl { value, .. }) if value == "12345"),
        "pasting into the collection field re-resolves it"
    );
}

#[test]
fn paste_strips_newlines() {
    let mut app = make_app();
    app.editing = true;
    app.handle_paste("12\n34\n".to_string());
    assert_eq!(app.home.collection.value, "1234");
}

#[test]
fn paste_outside_edit_mode_is_inert() {
    let mut app = make_app();
    // editing is false by default → paste must not type into the field
    let cmd = app.handle_paste("nope".to_string());
    assert_eq!(app.home.collection.value, "");
    assert!(cmd.is_none());
}

// ── config tab: osu! official gating ─────────────────────────────────────────
// Mirror toggling lives entirely on the Config tab; the Get Maps tab only shows
// the enabled-mirror count.

#[test]
fn osu_official_toggle_blocked_and_notifies_when_logged_out() {
    use osu_collect::app::Tab;
    use osu_collect::app::{AuthLoginState, ConfigField};
    let mut app = make_app();
    app.config.login_state = AuthLoginState::LoggedOut;
    app.config.osu_official = false;
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::MirrorOsuOfficial;

    app.handle_key(press(KeyCode::Enter));
    assert!(
        !app.config.osu_official,
        "osu! official must not be enablable while logged out"
    );
    assert!(
        !app.toasts.is_empty(),
        "a logged-out enable attempt must notify the user to log in"
    );

    // `space` is gated the same way.
    app.handle_key(press(KeyCode::Char(' ')));
    assert!(!app.config.osu_official, "space must not enable it either");
}

#[test]
fn osu_official_toggle_works_when_logged_in() {
    use osu_collect::app::Tab;
    use osu_collect::app::{AuthLoginState, ConfigField};
    // Sandbox the config path so the toggle's auto-save never touches the real
    // user config.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let _env = osu_collect::test_env::TempEnvVar::set("OSU_COLLECT_CONFIG", path.to_str().unwrap());

    let mut app = make_app();
    app.config.login_state = AuthLoginState::LoggedIn;
    app.config.osu_official = false;
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::MirrorOsuOfficial;

    app.handle_key(press(KeyCode::Enter));

    assert!(
        app.config.osu_official,
        "logged in, the osu! official toggle works normally"
    );
}

#[test]
fn enter_on_home_mirrors_summary_jumps_to_config_mirrors() {
    use osu_collect::app::Tab;
    use osu_collect::app::{ConfigField, HomeField};
    let mut app = make_app();
    app.home.focus = HomeField::Mirrors;

    app.handle_key(press(KeyCode::Enter));

    assert_eq!(
        app.active_tab,
        Tab::Config,
        "enter on the mirrors summary opens the config tab"
    );
    // Focus lands on the first built-in mirror in the default try-order.
    assert_eq!(app.config.focus, ConfigField::MirrorOsuDirect);
}

#[test]
fn config_mirror_toggle_syncs_home_count() {
    use osu_collect::app::ConfigField;
    use osu_collect::app::Tab;
    // Sandbox the config path so the toggle's auto-save never touches the real
    // user config.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let _env = osu_collect::test_env::TempEnvVar::set("OSU_COLLECT_CONFIG", path.to_str().unwrap());

    let mut app = make_app();
    let before = app.home.mirror_count();
    app.active_tab = Tab::Config;
    // Nerinyan is enabled by default; toggling it off must lower the Get Maps
    // count, since the summary derives from the Config tab now.
    app.config.focus = ConfigField::MirrorNerinyan;
    app.handle_key(press(KeyCode::Enter));

    assert_eq!(
        app.home.mirror_count(),
        before - 1,
        "toggling a mirror off on the config tab lowers the Get Maps enabled count"
    );
}

#[test]
fn space_on_download_button_does_not_start_download() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.collection.value = "123".to_string();
    app.home.focus = HomeField::Download;

    // space is a toggle alias only; it must not activate the download button
    let cmd = app.handle_key(press(KeyCode::Char(' ')));
    assert!(
        cmd.is_none(),
        "space on the download button must not start a download"
    );
}

#[test]
fn space_in_update_browse_toggles_highlighted_collection() {
    use osu_collect::app::GetMapsSource;
    use osu_collect::app::update_source::{MissingBeatmapset, MissingStatus};
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    // seed one collection with an update (so it's selectable) and descend
    app.home
        .update
        .set_collections(vec![osu_collect::osu_db::LocalCollection {
            name: "test - 1234".to_string(),
            beatmap_checksums: Vec::new().into(),
        }]);
    app.home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 42,
            status: MissingStatus::NotInstalled,
            collection_id: 1234,
            collection_name: "test - 1234".to_string(),
            included: true,
            previously_deleted: false,
            checksums: Box::new([]),
            enrich_diff_id: None,
        }],
        &std::collections::HashMap::new(),
    );
    app.home.update.descend();

    let before = app.home.update.selection.local_collections[0].selected;
    app.handle_key(press(KeyCode::Char(' ')));
    assert_eq!(
        app.home.update.selection.local_collections[0].selected, !before,
        "space toggles the highlighted collection's checkbox"
    );
}

// ── enter on home tab ─────────────────────────────────────────────────────────

#[test]
fn recheck_failed_key_dispatches_on_update_source() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home.update.set_failed_beatmapset_count(2);

    let cmd = app.handle_key(press(KeyCode::Char('r')));

    assert!(matches!(cmd, Some(AppCommand::RecheckFailedMaps)));
}

#[test]
fn recheck_failed_key_ignored_without_failed_maps() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;

    let cmd = app.handle_key(press(KeyCode::Char('r')));

    assert!(cmd.is_none());
}

#[test]
fn enter_on_download_button_without_collection_input_produces_error() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    // clear any default value
    app.home.collection.value.clear();
    // focus the download button; enter there should fail to download
    app.home.focus = HomeField::Download;
    app.handle_key(press(KeyCode::Enter));
    // no command issued (error path), an error toast should be raised
    assert!(!app.toasts.is_empty());
}

#[test]
fn enter_on_collection_field_does_not_start_download() {
    let mut app = make_app();
    app.home.collection.value.clear();
    // collection field is focused by default; enter here only acts on the field,
    // it must not attempt a download (that lives on the button now)
    app.handle_key(press(KeyCode::Enter));
    assert!(
        app.toasts.is_empty(),
        "enter on the collection field must not trigger the download error path"
    );
}

// ── config tab key bindings ───────────────────────────────────────────────────

/// Move focus to the config auth chip, ready to open the login split. Pins a
/// logged-out state so the panel opens on the credentials phase regardless of
/// any osu! token stored on the host running the tests.
fn focus_config_auth_chip() -> osu_collect::app::App {
    use osu_collect::app::Tab;
    use osu_collect::app::{AuthLoginState, ConfigField, HomeField};

    let mut app = make_app();
    app.config.login_state = AuthLoginState::LoggedOut;
    // Focus a non-text field so Right switches tabs rather than moving the caret.
    app.home.focus = HomeField::Video;
    // Three static tabs: home → downloads → config.
    app.handle_key(press(KeyCode::Right));
    app.handle_key(press(KeyCode::Right));
    assert_eq!(app.active_tab(), Tab::Config);
    app.config.focus = ConfigField::AuthChip;
    app
}

#[test]
fn enter_on_config_chip_opens_login_split() {
    use osu_collect::app::Tab;

    let mut app = focus_config_auth_chip();

    // The chip opens the login split on the right and dispatches no command
    // (the panel owns the login flow). The active tab stays on Config — the
    // login split is a focus-trap panel, not a tab.
    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(
        cmd.is_none(),
        "opening the login split dispatches no command"
    );
    assert!(app.login_open(), "login split must open");
    assert_eq!(
        app.active_tab(),
        Tab::Config,
        "active tab stays on config while the login split is open"
    );
}

#[test]
fn esc_closes_login_split_and_stays_on_config() {
    use osu_collect::app::ConfigField;
    use osu_collect::app::Tab;

    let mut app = focus_config_auth_chip();
    app.handle_key(press(KeyCode::Enter));
    assert!(app.login_open());

    // esc closes the split in place and hands focus back to the auth chip.
    app.handle_key(press(KeyCode::Esc));
    assert!(!app.login_open(), "esc closes the login split");
    assert_eq!(app.active_tab(), Tab::Config);
    assert_eq!(app.config.focus, ConfigField::AuthChip);
}

#[test]
fn switching_tabs_closes_login_split() {
    let mut app = focus_config_auth_chip();
    app.handle_key(press(KeyCode::Enter));
    assert!(app.login_open());

    // A tab switch closes the split (it lives only on Config).
    app.handle_key(press(KeyCode::Right));
    assert!(!app.login_open(), "switching tabs closes the login split");
}

#[test]
fn typing_routes_to_login_field_while_split_open() {
    let mut app = focus_config_auth_chip();
    app.handle_key(press(KeyCode::Enter));
    // The username field is focused on open; enter descends into edit mode.
    app.handle_key(press(KeyCode::Enter));
    app.handle_key(press(KeyCode::Char('a')));
    app.handle_key(press(KeyCode::Char('b')));
    assert_eq!(
        app.login.as_ref().map(|l| l.username.value.as_str()),
        Some("ab"),
        "chars route into the focused login field while the split is open"
    );
}

#[test]
fn space_on_auth_chip_does_nothing() {
    use osu_collect::app::Tab;
    use osu_collect::app::{ConfigField, HomeField};

    let mut app = make_app();
    // Focus a non-text field so Right switches tabs rather than moving the caret.
    app.home.focus = HomeField::Video;
    // Three static tabs: home → downloads → config.
    app.handle_key(press(KeyCode::Right));
    app.handle_key(press(KeyCode::Right));
    assert_eq!(app.active_tab(), Tab::Config);
    app.config.focus = ConfigField::AuthChip;

    // space must not trigger any action on the chip — enter is the confirm key
    let cmd = app.handle_key(press(KeyCode::Char(' ')));
    assert!(
        cmd.is_none(),
        "space on auth chip must not issue any command"
    );
}

// ── help overlay ─────────────────────────────────────────────────────────────

#[test]
fn question_mark_opens_help_overlay() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    // ? types into a text field; move focus off the default collection input
    app.home.focus = HomeField::Video;
    assert!(!app.help_open);
    app.handle_key(press(KeyCode::Char('?')));
    assert!(app.help_open, "? must open the help overlay");
}

#[test]
fn question_mark_while_editing_text_field_types_instead_of_opening_help() {
    let mut app = make_app();
    // collection field is focused by default; enter edit mode so ? types
    app.editing = true;
    app.handle_key(press(KeyCode::Char('?')));
    assert!(!app.help_open, "? must not open help while editing a field");
    assert_eq!(app.home.collection.value, "?", "? must type into the field");
}

#[test]
fn question_mark_closes_open_help_overlay() {
    let mut app = make_app();
    app.help_open = true;
    app.handle_key(press(KeyCode::Char('?')));
    assert!(!app.help_open, "? must close an already-open help overlay");
}

#[test]
fn esc_closes_help_overlay_without_quitting() {
    let mut app = make_app();
    app.help_open = true;
    let cmd = app.handle_key(press(KeyCode::Esc));
    assert!(!app.help_open, "esc must close the help overlay");
    assert!(cmd.is_none(), "esc while help is open must not quit");
}

#[test]
fn q_closes_help_overlay_without_quitting() {
    let mut app = make_app();
    app.help_open = true;
    let cmd = app.handle_key(press(KeyCode::Char('q')));
    assert!(!app.help_open, "q must close the help overlay");
    assert!(cmd.is_none(), "q while help is open must not quit");
}

#[test]
fn question_mark_returns_no_command() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Video;
    let cmd = app.handle_key(press(KeyCode::Char('?')));
    assert!(cmd.is_none(), "? must not issue any AppCommand");
}

#[test]
fn down_up_scroll_the_open_help_overlay() {
    let mut app = make_app();
    app.help_open = true;
    assert_eq!(app.help_scroll.get(), 0);

    app.handle_key(press(KeyCode::Down));
    app.handle_key(press(KeyCode::Down));
    assert_eq!(
        app.help_scroll.get(),
        2,
        "down must scroll the overlay down"
    );

    app.handle_key(press(KeyCode::Up));
    assert_eq!(app.help_scroll.get(), 1, "up must scroll the overlay up");
}

#[test]
fn up_at_help_top_stays_pinned() {
    let mut app = make_app();
    app.help_open = true;
    app.handle_key(press(KeyCode::Up));
    assert_eq!(app.help_scroll.get(), 0, "up at the top must not underflow");
}

#[test]
fn keys_are_inert_while_help_open() {
    // `d` would normally jump to the output-dir field on the home tab; the help
    // overlay must swallow it so background actions never fire under the modal.
    let mut app = make_app();
    app.help_open = true;
    let cmd = app.handle_key(press(KeyCode::Char('d')));
    assert!(
        cmd.is_none(),
        "background hotkeys must be inert while help is open"
    );
    assert!(app.help_open, "an unrelated key must not close help");
}

#[test]
fn opening_help_resets_scroll() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Video;
    app.help_scroll.set(7);
    app.handle_key(press(KeyCode::Char('?')));
    assert!(app.help_open);
    assert_eq!(
        app.help_scroll.get(),
        0,
        "opening help must reset the scroll"
    );
}

// ── vim keymap (opt-in, off by default) ───────────────────────────────────────

fn config_app_vim(on: bool) -> App {
    use osu_collect::app::Tab;
    let mut app = make_app();
    app.active_tab = Tab::Config;
    app.config.vim_keys = on;
    app
}

#[test]
fn vim_off_letters_do_not_navigate() {
    use osu_collect::app::ConfigField;
    let mut app = config_app_vim(false);
    app.config.focus = ConfigField::Theme;
    app.handle_key(press(KeyCode::Char('j')));
    assert_eq!(
        app.config.focus,
        ConfigField::Theme,
        "with vim off, j is inert on the config form"
    );
}

#[test]
fn vim_jk_move_field_focus() {
    use osu_collect::app::ConfigField;
    let mut app = config_app_vim(true);
    app.config.focus = ConfigField::Theme;
    app.handle_key(press(KeyCode::Char('j')));
    assert_eq!(
        app.config.focus,
        ConfigField::VimKeys,
        "j moves down a field"
    );
    app.handle_key(press(KeyCode::Char('k')));
    assert_eq!(app.config.focus, ConfigField::Theme, "k moves up a field");
}

#[test]
fn vim_hl_switch_tabs() {
    use osu_collect::app::Tab;
    let mut app = config_app_vim(true);
    app.handle_key(press(KeyCode::Char('h')));
    assert_eq!(
        app.active_tab(),
        Tab::Downloads,
        "h switches to the prev tab"
    );
    app.handle_key(press(KeyCode::Char('l')));
    assert_eq!(app.active_tab(), Tab::Config, "l switches to the next tab");
}

#[test]
fn vim_gg_and_capital_g_jump_to_ends() {
    use osu_collect::app::ConfigField;
    let mut app = config_app_vim(true);
    app.config.focus = ConfigField::DownloadVideo;
    // A lone `g` latches and is swallowed; the second `g` forms `gg`.
    assert!(app.handle_key(press(KeyCode::Char('g'))).is_none());
    app.handle_key(press(KeyCode::Char('g')));
    assert_eq!(
        app.config.focus,
        ConfigField::AuthChip,
        "gg jumps to the first field"
    );
    app.handle_key(press(KeyCode::Char('G')));
    assert_eq!(
        app.config.focus,
        ConfigField::Prereleases,
        "G jumps to the last field"
    );
}

#[test]
fn vim_lone_g_then_motion_does_not_jump() {
    use osu_collect::app::ConfigField;
    let mut app = config_app_vim(true);
    app.config.focus = ConfigField::Theme;
    // `g` then `j`: the latch clears and `j` is a normal one-field move.
    app.handle_key(press(KeyCode::Char('g')));
    app.handle_key(press(KeyCode::Char('j')));
    assert_eq!(app.config.focus, ConfigField::VimKeys);
}

#[test]
fn vim_i_enters_edit_mode_then_typing_is_literal() {
    use osu_collect::app::ConfigField;
    let mut app = config_app_vim(true);
    app.config.focus = ConfigField::MirrorCustomUrl(0);
    app.handle_key(press(KeyCode::Char('i')));
    assert!(app.editing, "i descends into edit mode on a text field");
    // While editing, motion letters type literally — the vim layer is bypassed.
    app.handle_key(press(KeyCode::Char('j')));
    assert!(
        app.config
            .custom_mirrors
            .row(0)
            .unwrap()
            .value
            .contains('j'),
        "editing a field types literal chars, not vim motions"
    );
}

// ── update browse: enter toggles, does not ascend ─────────────────────────────

#[test]
fn enter_on_collection_toggles_and_stays_in_browse() {
    use osu_collect::app::GetMapsSource;
    use osu_collect::app::update_source::{MissingBeatmapset, MissingStatus};
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home
        .update
        .set_collections(vec![osu_collect::osu_db::LocalCollection {
            name: "test - 1234".to_string(),
            beatmap_checksums: Vec::new().into(),
        }]);
    app.home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 42,
            status: MissingStatus::NotInstalled,
            collection_id: 1234,
            collection_name: "test - 1234".to_string(),
            included: true,
            previously_deleted: false,
            checksums: Box::new([]),
            enrich_diff_id: None,
        }],
        &std::collections::HashMap::new(),
    );
    app.home.update.descend();
    let before = app.home.update.selection.local_collections[0].selected;

    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(cmd.is_none());
    assert_eq!(
        app.home.update.selection.local_collections[0].selected, !before,
        "enter toggles the highlighted collection"
    );
    assert!(
        app.home.update.is_browsing(),
        "enter on a collection stays in the browse"
    );
}

#[test]
fn enter_on_update_download_button_dispatches_selective() {
    use osu_collect::app::GetMapsSource;
    use osu_collect::app::HomeField;
    use osu_collect::app::update_source::{MissingBeatmapset, MissingStatus};

    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home
        .update
        .set_collections(vec![osu_collect::osu_db::LocalCollection {
            name: "test - 1234".to_string(),
            beatmap_checksums: Vec::new().into(),
        }]);
    // One missing set in the (selected-by-default) collection, so the
    // whole-collection selection resolves to a non-empty download set.
    app.home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 42,
            status: MissingStatus::NotInstalled,
            collection_id: 1234,
            collection_name: "test - 1234".to_string(),
            included: true,
            previously_deleted: false,
            checksums: Box::new([]),
            enrich_diff_id: None,
        }],
        &std::collections::HashMap::new(),
    );
    // The download now fires from the form's `download (N)` button, not an
    // in-browse action bar.
    app.home.focus = HomeField::Download;

    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(
        matches!(cmd, Some(AppCommand::StartSelectiveDownload { .. })),
        "enter on the update download button dispatches the selective download"
    );
}

// ── config tab: mirror reorder ────────────────────────────────────────────────

#[test]
fn shift_arrow_reorders_config_mirror_and_syncs_pipeline() {
    use osu_collect::app::ConfigField;
    use osu_collect::app::Tab;
    use osu_downloader::MirrorKind;

    // Sandbox the config path so the reorder's auto-save never touches the real
    // user config.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let _env = osu_collect::test_env::TempEnvVar::set("OSU_COLLECT_CONFIG", path.to_str().unwrap());

    let mut app = make_app();
    app.active_tab = Tab::Config;
    // Nerinyan is the second built-in in the default order.
    app.config.focus = ConfigField::MirrorNerinyan;
    app.handle_key(shift(KeyCode::Up));

    assert_eq!(
        app.config.mirror_order[0],
        MirrorKind::Nerinyan,
        "shift+up moves the focused mirror to the front of the try-order"
    );
    assert_eq!(
        app.home.mirror_order[0],
        MirrorKind::Nerinyan,
        "the Get Maps pipeline order syncs with the config reorder"
    );
    let first = app.home.build_mirror_list().first().map(|m| m.kind());
    assert_eq!(
        first,
        Some(MirrorKind::Nerinyan),
        "the pipeline tries the reordered mirror first"
    );
}

#[test]
fn shift_arrow_off_mirror_row_falls_through_to_focus_move() {
    use osu_collect::app::ConfigField;
    use osu_collect::app::Tab;

    let mut app = make_app();
    app.active_tab = Tab::Config;
    // Theme is not a mirror row, so shift+down behaves like plain focus movement.
    app.config.focus = ConfigField::Theme;
    app.handle_key(shift(KeyCode::Down));
    assert_eq!(
        app.config.focus,
        ConfigField::VimKeys,
        "shift+arrow off a mirror row moves focus like a plain arrow"
    );
}

// ── global client switch ──────────────────────────────────────────────────────

#[test]
fn c_switches_client_and_clears_scan_without_auto_scanning() {
    use osu_collect::app::Tab;

    let mut app = make_app();
    app.active_tab = Tab::Home;
    let before = app.library.client_type;

    let cmd = app.handle_key(press(KeyCode::Char('c')));

    assert_ne!(
        app.library.client_type, before,
        "c must flip the osu! client from any tab"
    );
    assert!(
        cmd.is_none(),
        "switching client must not auto-scan; the user scans manually"
    );
    assert!(
        app.home.update.selection.local_collections.is_empty(),
        "the prior client's scan data is cleared on switch"
    );
}

#[test]
fn c_types_literal_char_while_editing() {
    use osu_collect::app::HomeField;
    use osu_collect::app::Tab;

    let mut app = make_app();
    app.active_tab = Tab::Home;
    app.home.focus = HomeField::Collection;
    app.editing = true;
    let before = app.library.client_type;

    app.handle_key(press(KeyCode::Char('c')));

    assert_eq!(
        app.library.client_type, before,
        "c must not switch the client while typing into a field"
    );
}

#[test]
fn typing_into_updates_path_field_routes_to_library() {
    use osu_collect::app::{GetMapsSource, HomeField};

    // The osu! path field lives on the app-global library state now, but it is
    // still edited through the update source form. Typing must land on `library`.
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home.focus = HomeField::UpdateOsuPath;
    app.library.osu_path.set_value(String::new());
    app.editing = true;

    app.handle_key(press(KeyCode::Char('/')));
    app.handle_key(press(KeyCode::Char('o')));
    app.handle_key(press(KeyCode::Char('s')));
    app.handle_key(press(KeyCode::Backspace));

    assert_eq!(
        app.library.osu_path.value, "/o",
        "editing the updates path field must mutate the app-global library state"
    );
}

#[test]
fn request_find_download_osu_route_uses_query_label_and_ids() {
    use osu_collect::app::{BrowseRow, GetMapsSource};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.find.query.set_value("tekno");
    app.home.find.browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
            BrowseRow { id: 30, meta: None },
        ],
        &std::collections::HashMap::new(),
    );
    app.home.find.browse.set_all_selected(true);

    let (_, request) = app
        .request_find_download()
        .expect("a selection with mirrors enabled builds a request");
    // The download folder + page title derive from the query text.
    assert_eq!(request.label, "tekno");
    let mut ids = request.beatmapset_ids.clone();
    ids.sort_unstable();
    assert_eq!(ids, vec![10, 20, 30]);
}

#[test]
fn search_view_button_reopens_results_without_re_searching() {
    use crossterm::event::KeyCode;
    use osu_collect::app::{BrowseRow, FindStatusMsg, GetMapsSource, HomeField};

    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.find.query.set_value("tekno");
    app.home.find.status_msg = FindStatusMsg::ReadySearch { total: 2 };
    app.home.find.browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
        ],
        &std::collections::HashMap::new(),
    );
    // Mirror the Ready handler: the loaded rows are for the current inputs.
    app.home.find.mark_results_current();
    // On the form (not descended into the browse).
    assert!(!app.home.find.browse.is_browsing());

    app.home.focus = HomeField::FindBrowse;
    let cmd = app.handle_key(press(KeyCode::Enter));

    assert!(
        cmd.is_none(),
        "the view button must not re-run the search query"
    );
    assert!(
        app.home.find.browse.is_browsing(),
        "the view button reopens the results browse"
    );
    assert_eq!(
        app.home.find.status_msg,
        FindStatusMsg::ReadySearch { total: 2 },
        "the view button must not flip status back to Loading"
    );
}

#[test]
fn search_view_button_is_inert_once_the_query_diverges() {
    use crossterm::event::KeyCode;
    use osu_collect::app::{BrowseRow, FindStatusMsg, GetMapsSource, HomeField};

    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.find.query.set_value("tekno");
    app.home.find.status_msg = FindStatusMsg::ReadySearch { total: 2 };
    app.home.find.browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
        ],
        &std::collections::HashMap::new(),
    );
    app.home.find.mark_results_current();

    // Edit the query after results loaded: the snapshot no longer matches, so the
    // view button must go inert (no opening the now-stale results).
    app.home.find.query.set_value("teknoz");
    app.home.focus = HomeField::FindBrowse;
    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(cmd.is_none(), "stale view button fires nothing");
    assert!(
        !app.home.find.browse.is_browsing(),
        "a stale view button must not reopen the old results"
    );
}

#[test]
fn search_view_button_is_inert_until_results_load() {
    use crossterm::event::KeyCode;
    use osu_collect::app::{GetMapsSource, HomeField};

    // Idle (no results): the view button renders disabled, so focus can land on
    // it — Enter must be a no-op, never opening an empty browse.
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.focus = HomeField::FindBrowse;
    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(cmd.is_none(), "disabled view button fires nothing");
    assert!(
        !app.home.find.browse.is_browsing(),
        "an unloaded view button must not open an empty browse"
    );
}

#[test]
fn update_view_button_is_inert_until_a_scan_finds_updates() {
    use crossterm::event::KeyCode;
    use osu_collect::app::{GetMapsSource, HomeField};

    // No scan yet: the view button renders disabled; Enter is a no-op.
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home.focus = HomeField::UpdateBrowse;
    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(cmd.is_none(), "disabled view button fires nothing");
    assert!(
        !app.home.update.is_browsing(),
        "a pre-scan view button must not open an empty browse"
    );
}

#[test]
fn update_view_button_rekicks_enrichment_only_when_unfetched() {
    use crossterm::event::KeyCode;
    use osu_collect::app::update_source::{MissingBeatmapset, MissingStatus};
    use osu_collect::app::{EnrichSink, EnrichTarget, GetMapsSource, HomeField};

    let seed = |app: &mut osu_collect::app::App| {
        app.home.source = GetMapsSource::Update;
        app.home
            .update
            .set_collections(vec![osu_collect::osu_db::LocalCollection {
                name: "test - 100".to_string(),
                beatmap_checksums: Vec::new().into(),
            }]);
        // Seeds the pager at scan-land: one unfetched diff id, cursor 0.
        app.home.update.set_missing_beatmaps(
            vec![MissingBeatmapset {
                id: 10,
                status: MissingStatus::NotInstalled,
                collection_id: 100,
                collection_name: "test - 100".to_string(),
                included: true,
                previously_deleted: false,
                checksums: Box::new([]),
                enrich_diff_id: Some(1000),
            }],
            &std::collections::HashMap::new(),
        );
        app.home.focus = HomeField::UpdateBrowse;
    };

    // Nothing fetched yet (cursor 0) → the descend self-heals a missed prefetch.
    let mut app = make_app();
    seed(&mut app);
    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(
        matches!(
            cmd,
            Some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Update
            })
        ),
        "an unfetched scan re-kicks page 1 on descend, got {cmd:?}"
    );
    assert!(app.home.update.is_browsing());

    // A page already pulled (cursor > 0) → descend must NOT eager-fetch page 2.
    let mut app = make_app();
    seed(&mut app);
    let _ = app.home.update.next_enrich_page();
    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(
        cmd.is_none(),
        "cursor > 0 means page 1 already ran; descend never eager-fetches, got {cmd:?}"
    );
    assert!(app.home.update.is_browsing());
}

/// The dispatch names the collection the ROWS came from — the browse's own id —
/// never something a later write to `resolved_collection` installed. A late
/// resolve that moved it elsewhere is refused outright rather than dispatching
/// either id: the rows belong to 111, so a run tagged 999 would write 999's
/// `collection.db` over 111's sets, and a run tagged 111 would download a
/// collection the form has stopped showing.
#[test]
fn collection_pick_download_uses_snapshotted_id_not_late_resolve() {
    use osu_collect::app::{BrowseRow, GetMapsSource};

    fn browsing_111(resolved: u32) -> App {
        let mut app = make_app();
        app.home.source = GetMapsSource::Collection;
        // The state an `open_collection_browse` for 111 leaves: the field naming
        // it, the snapshot, and the browse bound to it.
        app.home.collection.set_value("111");
        app.home.collection_browse_id = Some(111);
        app.home.collection_browse.set_rows(
            vec![
                BrowseRow { id: 10, meta: None },
                BrowseRow { id: 20, meta: None },
            ],
            &std::collections::HashMap::new(),
        );
        app.home.collection_browse.set_all_selected(true);
        app.home.set_resolved_collection(resolved, vec![77, 88]);
        app
    }

    let (_, request) = browsing_111(111)
        .request_collection_pick_download()
        .expect("a selection with mirrors enabled builds a request");
    assert_eq!(request.collection_ids, vec![111]);
    let mut ids = request.beatmapset_ids.clone();
    ids.sort_unstable();
    assert_eq!(ids, vec![10, 20]);

    assert!(
        browsing_111(999)
            .request_collection_pick_download()
            .is_none(),
        "rows from 111 must not dispatch while the form has a different collection resolved"
    );
}

/// A part-picked collection carries the "skip already imported" toggle to the
/// run, so unchecking one map cannot silently turn the pre-skip off the way it
/// used to. Both legs vary only the config toggle, so a call site that hardcodes
/// either position reds one of them.
#[test]
fn collection_pick_download_carries_the_skip_imported_toggle() {
    use osu_collect::app::{BrowseRow, GetMapsSource};

    fn picked_request(skip_already_imported: bool) -> bool {
        let mut app = make_app();
        app.config.skip_already_imported = skip_already_imported;
        app.home.source = GetMapsSource::Collection;
        app.home.collection.set_value("111");
        app.home.set_resolved_collection(111, vec![10, 20]);
        app.home.collection_browse_id = Some(111);
        app.home.collection_browse.set_rows(
            vec![
                BrowseRow { id: 10, meta: None },
                BrowseRow { id: 20, meta: None },
            ],
            &std::collections::HashMap::new(),
        );
        app.home.collection_browse.set_all_selected(true);
        let (_, request) = app
            .request_collection_pick_download()
            .expect("a selection with mirrors enabled builds a request");
        request.skip_already_imported
    }

    assert!(picked_request(true));
    assert!(!picked_request(false));
}

#[test]
fn reopening_collection_browse_preserves_picks() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Collection;
    app.home.set_resolved_collection(7, vec![10, 20, 30]);
    app.home.focus = HomeField::CollectionBrowse;

    // Open browse&pick: a fresh collection defaults every set selected.
    app.handle_key(press(KeyCode::Enter));
    assert!(app.home.collection_browse.is_browsing());
    assert_eq!(app.home.collection_browse.selected_count(), 3);

    // Uncheck the highlighted row, then ascend back to the form.
    app.handle_key(press(KeyCode::Enter)); // toggle row 0 → 2 selected
    assert_eq!(app.home.collection_browse.selected_count(), 2);
    app.handle_key(press(KeyCode::Esc));
    assert!(!app.home.collection_browse.is_browsing());

    // Re-opening the SAME collection keeps the picks (no reset-to-all).
    app.handle_key(press(KeyCode::Enter));
    assert!(app.home.collection_browse.is_browsing());
    assert_eq!(
        app.home.collection_browse.selected_count(),
        2,
        "re-opening the same collection preserves the user's selection"
    );
}

/// A form with collection 42 resolved and a proper subset picked in browse&pick,
/// driven entirely through the key handler so the browse id and the picks are
/// whatever the real descend/toggle/ascend path writes.
fn collection_42_picked_through_the_keys() -> App {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Collection;
    app.home.collection.set_value("42");
    app.home.set_resolved_collection(42, vec![10, 20]);
    app.home.resolved_folder_name = Some("Farm-42".to_string());
    app.home.focus = HomeField::CollectionBrowse;

    app.handle_key(press(KeyCode::Enter)); // descend, all selected
    app.handle_key(press(KeyCode::Enter)); // uncheck the cursor row → 1 of 2
    app.handle_key(press(KeyCode::Esc)); // back to the form
    app
}

/// Typing one more digit into the URL field retargets the form in the SAME
/// press: 42 → 421 is a different collection, so the picked-subset run stops
/// being reachable at all. Before, both sides of the subset check had gone stale
/// together, so a press queued 42's picks under a field reading 421 — through
/// the whole debounce, and for good when the 421 fetch failed.
///
/// The direct call is the second half of the guard: `dispatch_form_download`
/// consulting `collection_subset_picked` first is a convention nothing enforces,
/// so the entry point every picked-subset run converges on refuses on its own.
#[test]
fn a_retyped_collection_id_cannot_dispatch_the_old_collections_picks() {
    use osu_collect::app::HomeField;

    let mut app = collection_42_picked_through_the_keys();
    assert_eq!(app.home.collection_browse_id, Some(42));
    assert_eq!(app.home.collection_browse.selected_count(), 1);
    assert!(
        app.home.collection_subset_picked(),
        "precondition: a proper subset of the resolved collection"
    );
    assert!(
        app.request_collection_pick_download().is_some(),
        "precondition: that subset dispatches while the field still names 42"
    );

    let mut app = collection_42_picked_through_the_keys();
    app.home.focus = HomeField::Collection;
    app.editing = true;
    let cmd = app.handle_key(press(KeyCode::Char('1')));
    assert_eq!(app.home.collection.value, "421");
    assert!(
        matches!(cmd, Some(AppCommand::ResolveCollectionUrl { ref value, .. }) if value == "421"),
        "the keystroke still schedules the new resolve, got {cmd:?}"
    );

    assert!(
        !app.home.collection_subset_picked(),
        "the router must not reach the picked-subset path for a collection the field left"
    );
    assert!(
        app.request_collection_pick_download().is_none(),
        "and the dispatch itself refuses, so no caller can queue 42's picks under 421"
    );
    assert!(
        app.downloads.is_empty(),
        "a refused dispatch queues no run page"
    );
}

/// The settle inside the dispatch, with nothing having run it first. Today every
/// collection-field edit funnels through `mutate_collection_then_resolve`, so
/// this pair is not reachable from the keyboard — that is exactly what the guard
/// is for, and a fixture built by hand is the only way to pin defence in depth.
/// Both halves are load-bearing here: `picked_collection_id` alone accepts this
/// pair (browse id and resolved id still agree on 42), and the settle alone
/// leaves the browse id standing for a reader that goes straight to it.
#[test]
fn the_picked_subset_dispatch_settles_before_it_reads_the_browse_id() {
    use osu_collect::app::{BrowseRow, GetMapsSource};

    let mut app = make_app();
    app.home.source = GetMapsSource::Collection;
    app.home.set_resolved_collection(42, vec![10, 20]);
    app.home.collection_browse_id = Some(42);
    app.home.collection_browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
        ],
        &std::collections::HashMap::new(),
    );
    app.home.collection_browse.set_all_selected(true);
    app.home.collection.set_value("421");

    assert!(
        app.request_collection_pick_download().is_none(),
        "the run must not be built against a collection the field left"
    );
    assert!(
        app.downloads.is_empty(),
        "a refused dispatch queues no run page"
    );
    assert_eq!(
        app.home.resolved_collection, None,
        "the settle ran, so the readers after it see the cleared snapshot too"
    );
    let toast = app.toasts.iter().next().expect("a refusal must say why");
    assert_eq!(toast.title(), "resolve a collection first");

    // Both guards failing at once is what fixes their order: the pairing failure
    // is the more fundamental one and owns the message, because pointing the user
    // at the selection would point them at the thing that is not wrong. Same
    // fixture, differing only in whether anything is checked.
    let mut app = make_app();
    app.home.source = GetMapsSource::Collection;
    app.home.set_resolved_collection(42, vec![10, 20]);
    app.home.collection_browse_id = Some(42);
    app.home.collection_browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
        ],
        &std::collections::HashMap::new(),
    );
    app.home.collection_browse.set_all_selected(false);
    app.home.collection.set_value("421");

    assert!(app.request_collection_pick_download().is_none());
    let toast = app.toasts.iter().next().expect("a refusal must say why");
    assert_eq!(
        toast.title(),
        "resolve a collection first",
        "an unpaired browse outranks an empty selection"
    );
}

/// The other half of the tradeoff, asserted end to end rather than as the
/// `fresh` predicate in isolation: once the field moves to another collection
/// and that collection resolves, reopening the browse rebinds
/// `collection_browse_id` and re-selects every set. That is exactly what
/// clearing the browse id on a mistype would have cost — it would have put the
/// user's own collection through this same path — so the case for parking the
/// picks rests on this behaving the way it is argued to.
#[test]
fn reopening_after_the_field_moves_rebinds_the_browse_and_reselects_all() {
    use osu_collect::app::HomeField;

    let mut app = collection_42_picked_through_the_keys();
    assert_eq!(app.home.collection_browse.selected_count(), 1);

    // Retype to another collection; the settle drops 42's snapshot in the press.
    app.home.focus = HomeField::Collection;
    app.editing = true;
    app.handle_key(press(KeyCode::Char('1')));
    app.editing = false;
    assert_eq!(app.home.resolved_collection, None);
    assert_eq!(
        app.home.collection_browse_id,
        Some(42),
        "the rows are still bound to the collection they came from"
    );

    // 421's own resolve lands.
    app.home.set_resolved_collection(421, vec![30, 40, 50]);

    app.home.focus = HomeField::CollectionBrowse;
    app.handle_key(press(KeyCode::Enter));

    assert!(app.home.collection_browse.is_browsing(), "browse opened");
    assert_eq!(
        app.home.collection_browse_id,
        Some(421),
        "the browse rebinds to the collection it was opened against"
    );
    assert_eq!(
        app.home.collection_browse.selected_ids(),
        vec![30, 40, 50],
        "a browse id that changed re-selects every set — the pick from 42 is gone"
    );
    assert!(
        !app.home.collection_subset_picked(),
        "all selected is a whole-collection download, not a subset"
    );
}

/// The picks are parked, not destroyed. Backspacing a mistyped digit away and
/// letting the re-resolve land restores the exact selection — the state a clear
/// would have thrown away with no undo, since reopening the browse re-selects
/// every set once the browse id stops matching.
///
/// This is the UNCACHED path — the fixture seeds `resolved_collection` directly,
/// so no session payload exists and the restore arrives with a landed fetch. The
/// cached path, where the keystroke itself re-arms with no request at all, is
/// pinned at `a_cached_id_rearms_the_picks_on_the_keystroke` (`tests/unit/home.rs`)
/// and `returning_to_a_cached_id_schedules_no_fetch` (`tests/unit/home_resolve.rs`),
/// both of which need crate-private access this integration crate does not have.
#[test]
fn backspacing_a_mistyped_digit_restores_the_picked_subset() {
    use osu_collect::app::HomeField;

    let mut app = collection_42_picked_through_the_keys();
    let picked = app.home.collection_browse.selected_ids();
    app.home.focus = HomeField::Collection;
    app.editing = true;

    app.handle_key(press(KeyCode::Char('1'))); // 42 → 421
    assert!(!app.home.collection_subset_picked());
    app.handle_key(press(KeyCode::Backspace)); // 421 → 42
    assert_eq!(app.home.collection.value, "42");

    // The debounced resolve for 42 lands.
    app.home.set_resolved_collection(42, vec![10, 20]);

    assert!(
        app.home.collection_subset_picked(),
        "the picks belong to 42 and the field names 42 again"
    );
    assert_eq!(
        app.home.collection_browse.selected_ids(),
        picked,
        "the same selection, not a re-selected default"
    );
    let (_, request) = app
        .request_collection_pick_download()
        .expect("the restored subset dispatches");
    assert_eq!(request.collection_ids, vec![42]);
}

// ── filter source ─────────────────────────────────────────────────────────────

#[test]
fn filter_cta_emits_run_filter_and_rejects_bad_ranges() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.focus = HomeField::FindRun;
    // A nzbasic-forcer (special) resolves the plan to the filter route, so the
    // single CTA dispatches a filter fetch rather than the default osu search.
    app.home.find.cycle_special(true); // → farm

    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(
        matches!(cmd, Some(AppCommand::RunFilter { .. })),
        "a nzbasic-forced find CTA dispatches a filter fetch, got {cmd:?}"
    );

    // An invalid range surfaces as a toast, nothing dispatches.
    app.home.find.stars.set_value("nope");
    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(cmd.is_none(), "a bad range must not dispatch, got {cmd:?}");
}

#[test]
fn filter_chips_cycle_on_space_and_presets_seed_fields() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;

    app.home.focus = HomeField::FindSpecial;
    app.handle_key(press(KeyCode::Char(' ')));
    assert_eq!(app.home.find.special_label(), "farm");

    // Preset cycling seeds the editable fields (space steps none → all ranked
    // → loved → farm; the farm seed pins mode to osu and resets special).
    app.home.focus = HomeField::FindPreset;
    for _ in 0..3 {
        app.handle_key(press(KeyCode::Char(' ')));
    }
    assert_eq!(app.home.find.preset_label(), "farm");
    assert_eq!(app.home.find.mode_label(), "osu!");
    assert_eq!(app.home.find.special_label(), "farm");
}

#[test]
fn m_in_filter_browse_loads_more_enrichment() {
    use osu_collect::app::{BrowseRow, EnrichSink, EnrichTarget, FindBackend, GetMapsSource};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    // `m` routes by the backend that produced the results, so mark it nzbasic.
    app.home.find.note_results_backend(FindBackend::Nzbasic);
    app.home.find.browse.set_rows(
        vec![BrowseRow { id: 10, meta: None }],
        &std::collections::HashMap::new(),
    );
    // 300 diff ids = two enrichment pages; the first auto-fetch pulled one.
    app.home.find.browse.seed_enrichment(
        (0..300).map(|d| (d, None)).collect(),
        &std::collections::HashMap::new(),
    );
    let _ = app.home.find.browse.next_enrich_page();
    app.home.find.browse.descend();

    let cmd = app.handle_key(press(KeyCode::Char('m')));
    assert!(
        matches!(
            cmd,
            Some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Find
            })
        ),
        "`m` in the nzbasic browse loads the next enrichment page, got {cmd:?}"
    );

    // Drain the pager: `m` becomes a no-op once every page was requested.
    let _ = app.home.find.browse.next_enrich_page();
    let cmd = app.handle_key(press(KeyCode::Char('m')));
    assert!(cmd.is_none(), "a dry pager must not dispatch, got {cmd:?}");
}

#[test]
fn m_in_update_browse_loads_more_missing_set_enrichment() {
    use osu_collect::app::update_source::{MissingBeatmapset, MissingStatus};
    use osu_collect::app::{EnrichTarget, GetMapsSource};
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 10,
            status: MissingStatus::NotInstalled,
            collection_id: 100,
            collection_name: "col".to_string(),
            included: true,
            previously_deleted: false,
            checksums: Box::new([]),
            enrich_diff_id: Some(1000),
        }],
        &std::collections::HashMap::new(),
    );
    // Seeding happens at scan-land (`set_missing_beatmaps`), so a page is already
    // waiting; descend is a pure descend and `m` loads the next page.
    app.home.update.descend();

    let cmd = app.handle_key(press(KeyCode::Char('m')));
    assert!(
        matches!(
            cmd,
            Some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Update
            })
        ),
        "`m` in the update browse backfills the next enrichment page, got {cmd:?}"
    );
}

/// The descend guard is a second gate in front of the browse, separate from the
/// button's enabled state — both have to read rows, or `enter` on a live button
/// silently does nothing and the held-back sets are stranded.
#[test]
fn enter_opens_the_update_browse_when_every_missing_set_is_held_back() {
    use osu_collect::app::update_source::{MissingBeatmapset, MissingStatus};
    use osu_collect::app::{GetMapsSource, HomeField};
    use osu_collect::osu_db::LocalCollection;

    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home.update.set_collections(vec![LocalCollection {
        name: "col - 100".to_string(),
        beatmap_checksums: Box::new([]),
    }]);
    app.home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 10,
            status: MissingStatus::NotInstalled,
            collection_id: 100,
            collection_name: "col".to_string(),
            included: false,
            previously_deleted: true,
            checksums: Box::new([]),
            enrich_diff_id: None,
        }],
        &std::collections::HashMap::new(),
    );
    app.home.focus = HomeField::UpdateBrowse;
    assert_eq!(app.home.update.total_new_count(), 0, "nothing to fetch");

    app.handle_key(press(KeyCode::Enter));
    assert!(
        app.home.update.is_browsing(),
        "`enter` on `view N mapsets` still descends, so the held-back row is reachable"
    );
}

/// The per-set re-include has to be reachable from the real key, not just the
/// model: `enter` on the preview pane's previously-deleted row. Space is the
/// documented alias, so it is driven through the same path.
#[test]
fn enter_and_space_re_include_a_held_back_set_in_the_update_browse() {
    use osu_collect::app::GetMapsSource;
    use osu_collect::app::update_source::{MissingBeatmapset, MissingStatus};
    use osu_collect::osu_db::LocalCollection;

    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home.update.set_collections(vec![LocalCollection {
        name: "col - 100".to_string(),
        beatmap_checksums: Box::new([]),
    }]);
    app.home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 10,
            status: MissingStatus::NotInstalled,
            collection_id: 100,
            collection_name: "col".to_string(),
            included: false,
            previously_deleted: true,
            checksums: Box::new([]),
            enrich_diff_id: None,
        }],
        &std::collections::HashMap::new(),
    );
    app.home.update.descend();
    app.home.update.focus_preview();
    assert!(
        app.home.update.preview_focused(),
        "the preview must be enterable, or the key never reaches the toggle"
    );
    assert!(
        app.home.update.selected_beatmapset_ids().is_empty(),
        "held back before the press"
    );

    app.handle_key(press(KeyCode::Enter));
    assert_eq!(
        app.home.update.selected_beatmapset_ids(),
        vec![10],
        "one `enter` on a previously-deleted preview row re-includes it AND undoes \
         the force-deselect the app applied while the collection was inert"
    );

    app.handle_key(press(KeyCode::Char(' ')));
    assert!(
        app.home.update.selected_beatmapset_ids().is_empty(),
        "`space` is the same toggle, so a second press holds it back again"
    );
}

/// The list pane's `enter` still owns the collection checkbox — the new preview
/// binding must not have retargeted it.
#[test]
fn enter_on_the_update_collections_pane_still_toggles_the_checkbox() {
    use osu_collect::app::GetMapsSource;
    use osu_collect::app::update_source::{MissingBeatmapset, MissingStatus};
    use osu_collect::osu_db::LocalCollection;

    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home.update.set_collections(vec![LocalCollection {
        name: "col - 100".to_string(),
        beatmap_checksums: Box::new([]),
    }]);
    app.home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 10,
            status: MissingStatus::NotInstalled,
            collection_id: 100,
            collection_name: "col".to_string(),
            included: true,
            previously_deleted: false,
            checksums: Box::new([]),
            enrich_diff_id: None,
        }],
        &std::collections::HashMap::new(),
    );
    app.home.update.descend();
    assert!(!app.home.update.preview_focused());
    assert!(app.home.update.selection.local_collections[0].selected);

    app.handle_key(press(KeyCode::Enter));
    assert!(
        !app.home.update.selection.local_collections[0].selected,
        "`enter` on the list pane still unchecks the collection"
    );
}

#[test]
fn request_find_download_nzbasic_route_uses_label_tag_and_ids() {
    use osu_collect::app::{BrowseRow, GetMapsSource};
    use osu_collect::download::IdsRunSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    // Seed the farm preset so the label + folder tag read "farm" — the preset is
    // a nzbasic-forcer, so the download routes to the filter subdir prefix.
    app.home.focus = osu_collect::app::HomeField::FindPreset;
    for _ in 0..3 {
        app.handle_key(press(KeyCode::Char(' ')));
    }
    app.home.find.browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
        ],
        &std::collections::HashMap::new(),
    );
    app.home.find.browse.set_all_selected(true);

    let (_, request) = app
        .request_find_download()
        .expect("a selection with mirrors enabled builds a request");
    assert_eq!(request.source, IdsRunSource::Filter);
    assert_eq!(request.label, "farm");
    assert_eq!(request.folder_tag, "farm");
    let mut ids = request.beatmapset_ids.clone();
    ids.sort_unstable();
    assert_eq!(ids, vec![10, 20]);
}

/// Tab order must walk the form top to bottom. The rendered order is pinned
/// separately by `find_form_groups_its_rows_under_section_eyebrows`, so a field
/// list that drifts from the eyebrows reds one of the two.
#[test]
fn find_form_tab_order_matches_the_rendered_order() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.focus = HomeField::Source;
    // Shut the supporter gate explicitly: `ConfigTab::new` seeds it from the
    // real stored auth, so on a box whose account has supporter this walked the
    // supporter order instead and failed on `FindExplicit`. This is the
    // NON-supporter order; both legs are pinned at the model level in
    // `app::home`'s tests.
    app.config.set_logged_out();
    for expected in [
        HomeField::FindQuery,
        HomeField::FindPreset,
        HomeField::FindMode,
        HomeField::FindStatus,
        HomeField::FindSpecial,
        HomeField::FindSort,
        HomeField::FindLimit,
        HomeField::FindAdvanced,
        HomeField::FindRun,
        HomeField::FindBrowse,
    ] {
        app.handle_key(press(KeyCode::Down));
        assert_eq!(app.home.focus, expected);
    }
}

/// With the disclosure open the 13 range inputs slot in after it and before the
/// CTAs, so the expanded list walks the same visual order. Non-supporter, so the
/// five gated facets that would otherwise open the section are absent.
#[test]
fn find_form_expanded_tab_order_runs_the_ranges_after_the_disclosure() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    // Same reason as the fixture above: pin the gate shut rather than inherit
    // whatever account this machine has stored.
    app.config.set_logged_out();
    app.home.focus = HomeField::FindAdvanced;
    app.handle_key(press(KeyCode::Char(' '))); // open the disclosure
    app.handle_key(press(KeyCode::Down));
    assert_eq!(app.home.focus, HomeField::FindStars);
    app.home.focus = HomeField::FindTitle; // the last range input
    app.handle_key(press(KeyCode::Down));
    assert_eq!(app.home.focus, HomeField::FindRun);
}

// ── multi-select chip rows (extra / rank) ─────────────────────────────────────

fn find_app_on(field: osu_collect::app::HomeField) -> App {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.config.set_login_complete(true);
    app.home.find.toggle_advanced_filters();
    app.home.focus = field;
    app
}

/// The contract the chip cursor had to survive: on a row AT REST, `←`/`→` keep
/// switching tabs exactly as everywhere else. Only a descended row carves them
/// out. Break this and the sub-cursor has eaten a global key.
#[test]
fn plain_arrows_still_switch_tabs_on_a_multi_select_row() {
    use osu_collect::app::{HomeField, Tab};
    let mut app = find_app_on(HomeField::FindRank);
    assert_eq!(app.active_tab(), Tab::Home);

    app.handle_key(press(KeyCode::Right));
    assert_ne!(app.active_tab(), Tab::Home, "→ must still leave the tab");

    let mut app = find_app_on(HomeField::FindExtra);
    app.handle_key(press(KeyCode::Left));
    assert_ne!(app.active_tab(), Tab::Home, "← must still leave the tab");

    // And the row itself is untouched by them — no cursor move, no pick.
    let mut app = find_app_on(HomeField::FindExtra);
    app.handle_key(press(KeyCode::Right));
    assert_eq!(app.home.find.extra.cursor(), 0);
    assert!(app.home.find.extra.is_empty());
}

#[test]
fn arrows_walk_the_chip_cursor_while_the_row_is_descended() {
    use osu_collect::app::{HomeField, Tab};
    let mut app = find_app_on(HomeField::FindRank);
    app.handle_key(press(KeyCode::Enter));

    app.handle_key(press(KeyCode::Right));
    assert_eq!(app.active_tab(), Tab::Home, "→ is consumed by the row");
    assert_eq!(app.home.find.rank.cursor(), 1);
    app.handle_key(press(KeyCode::Right));
    assert_eq!(app.home.find.rank.cursor(), 2);
    app.handle_key(press(KeyCode::Left));
    assert_eq!(app.home.find.rank.cursor(), 1);
    // Walking alone picks nothing.
    assert!(app.home.find.rank.is_empty());
}

/// `enter` and `space` both open the row's second stage; once it is down `space`
/// toggles the chip under the cursor and `enter` closes the stage again — the
/// same two-stage grammar a text input has, with `space` standing in for the
/// state-control key the row would otherwise carry.
#[test]
fn enter_and_space_both_descend_into_the_row() {
    use osu_collect::app::HomeField;
    for open in [KeyCode::Enter, KeyCode::Char(' ')] {
        let mut app = find_app_on(HomeField::FindExtra);
        app.handle_key(press(open));
        assert!(app.find_chip_editing(), "{open:?} must descend");
        // Only now do the arrows and `space` act on the row.
        app.handle_key(press(KeyCode::Right)); // cursor → storyboard
        app.handle_key(press(KeyCode::Char(' ')));
        assert!(
            !app.home.find.extra.contains(0) && app.home.find.extra.contains(1),
            "{open:?}: space toggled the wrong chip"
        );
        app.handle_key(press(KeyCode::Char(' ')));
        assert!(
            app.home.find.extra.is_empty(),
            "{open:?}: space must toggle back off"
        );
    }
}

/// `enter`, `esc` and a row move all leave the stage — and the move takes the
/// row with it, so focus never lands elsewhere with the mode still down.
#[test]
fn every_exit_from_a_descended_row_puts_the_mode_back() {
    use osu_collect::app::HomeField;
    for exit in [KeyCode::Enter, KeyCode::Esc] {
        let mut app = find_app_on(HomeField::FindRank);
        app.handle_key(press(KeyCode::Enter));
        app.handle_key(press(exit));
        assert!(!app.find_chip_editing(), "{exit:?} must leave the mode");
        assert_eq!(
            app.home.focus,
            HomeField::FindRank,
            "{exit:?} must not also leave the row"
        );
        // And the arrows are global again.
        app.handle_key(press(KeyCode::Right));
        assert_ne!(app.active_tab(), osu_collect::app::Tab::Home);
    }
    for (exit, landing) in [
        (KeyCode::Down, HomeField::FindPlayed),
        (KeyCode::Up, HomeField::FindExtra),
    ] {
        let mut app = find_app_on(HomeField::FindRank);
        app.handle_key(press(KeyCode::Enter));
        app.handle_key(press(exit));
        assert!(!app.find_chip_editing(), "{exit:?} must leave the mode");
        assert_eq!(app.home.focus, landing, "{exit:?} must leave the row too");
    }
}

/// Once the gate shuts, the row is gone — and with it the edit mode that was
/// down on it, or `editing` outlives its row and traps the arrows on a field
/// that is neither a text input nor a chip row.
///
/// The close runs through the path production takes (`App::set_logged_out`),
/// which is what moves focus off the row and drops the picks. Assigning the flag
/// alone would build a state the app cannot reach — focus parked on a row the
/// render already dropped — and pin nothing that ships.
#[test]
fn the_supporter_gate_closing_takes_the_descended_row_with_it() {
    use osu_collect::app::{HomeField, Tab};
    let mut app = find_app_on(HomeField::FindExtra);
    // The row is live and descended first, so the fall-through below is a change
    // and not the starting state.
    app.handle_key(press(KeyCode::Enter));
    app.handle_key(press(KeyCode::Right));
    app.handle_key(press(KeyCode::Char(' ')));
    assert_eq!(app.home.find.extra.cursor(), 1);
    assert!(app.home.find.extra.contains(1) && app.find_chip_editing());

    app.set_logged_out();
    assert_eq!(
        app.home.focus,
        HomeField::Source,
        "the row stops rendering, so focus has to leave it at the flip"
    );
    assert!(!app.editing, "the edit mode has to leave with the row");
    assert!(
        app.home.find.extra.is_empty() && app.home.find.extra.cursor() == 0,
        "and the pick goes with it — an invisible chip still rides into the query"
    );

    app.handle_key(press(KeyCode::Right));
    assert_ne!(
        app.active_tab(),
        Tab::Home,
        "with the gate shut the row is gone, so → falls through to the tab switch"
    );
    assert_eq!(app.home.find.extra.cursor(), 0, "and moves no cursor");
}
