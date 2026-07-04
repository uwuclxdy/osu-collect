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
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Video; // non-text so ←/→ switch screens
    assert_eq!(app.active_tab(), 0);
    app.handle_key(press(KeyCode::Right));
    assert_eq!(app.active_tab(), 1);
}

#[test]
fn left_arrow_wraps_to_last_tab() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Video;
    app.handle_key(press(KeyCode::Left));
    // wraps to the last static tab (2 = config) since no downloads
    assert_eq!(app.active_tab(), 2);
}

#[test]
fn tab_and_backtab_switch_screens() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Video;
    assert_eq!(app.active_tab(), 0);
    app.handle_key(press(KeyCode::Tab));
    assert_eq!(app.active_tab(), 1, "tab cycles to the next tab");
    app.handle_key(press(KeyCode::BackTab));
    assert_eq!(app.active_tab(), 0, "shift+tab cycles to the previous tab");
}

#[test]
fn tab_completes_directory_while_editing_it() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    // Editing the directory field: tab completes the path, never switches tabs.
    app.home.focus = HomeField::Directory;
    app.editing = true;
    app.handle_key(press(KeyCode::Tab));
    assert_eq!(
        app.active_tab(),
        0,
        "tab must complete the path, not switch tabs, while editing the directory"
    );
}

#[test]
fn s_jumps_to_download_button_on_home() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    // Default focus is the collection text field, selected-not-editing.
    assert_eq!(app.home.focus, HomeField::Collection);
    app.handle_key(press(KeyCode::Char('s')));
    assert_eq!(
        app.home.focus,
        HomeField::Download,
        "s jumps focus to the download button"
    );
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
    assert_eq!(app.home.focus, HomeField::Collection);
    app.handle_key(press(KeyCode::Up));
    // should wrap to last field (the download button)
    assert_eq!(app.home.focus, HomeField::Download);
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
        matches!(cmd, Some(AppCommand::ResolveCollectionUrl { value }) if value == "12345"),
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
    use osu_collect::app::{AuthLoginState, ConfigField};
    use osu_collect::config::constants::CONFIG_TAB_INDEX;
    let mut app = make_app();
    app.config.login_state = AuthLoginState::LoggedOut;
    app.config.osu_official = false;
    app.active_tab = CONFIG_TAB_INDEX;
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
    use osu_collect::app::{AuthLoginState, ConfigField};
    use osu_collect::config::constants::CONFIG_TAB_INDEX;
    // Sandbox the config path so the toggle's auto-save never touches the real
    // user config.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    unsafe { std::env::set_var("OSU_COLLECT_CONFIG", path.to_str().unwrap()) };

    let mut app = make_app();
    app.config.login_state = AuthLoginState::LoggedIn;
    app.config.osu_official = false;
    app.active_tab = CONFIG_TAB_INDEX;
    app.config.focus = ConfigField::MirrorOsuOfficial;

    app.handle_key(press(KeyCode::Enter));

    unsafe { std::env::remove_var("OSU_COLLECT_CONFIG") };

    assert!(
        app.config.osu_official,
        "logged in, the osu! official toggle works normally"
    );
}

#[test]
fn enter_on_home_mirrors_summary_jumps_to_config_mirrors() {
    use osu_collect::app::{ConfigField, HomeField};
    use osu_collect::config::constants::CONFIG_TAB_INDEX;
    let mut app = make_app();
    app.home.focus = HomeField::Mirrors;

    app.handle_key(press(KeyCode::Enter));

    assert_eq!(
        app.active_tab, CONFIG_TAB_INDEX,
        "enter on the mirrors summary opens the config tab"
    );
    // Focus lands on the first built-in mirror in the default try-order.
    assert_eq!(app.config.focus, ConfigField::MirrorOsuDirect);
}

#[test]
fn config_mirror_toggle_syncs_home_count() {
    use osu_collect::app::ConfigField;
    use osu_collect::config::constants::CONFIG_TAB_INDEX;
    // Sandbox the config path so the toggle's auto-save never touches the real
    // user config.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    unsafe { std::env::set_var("OSU_COLLECT_CONFIG", path.to_str().unwrap()) };

    let mut app = make_app();
    let before = app.home.mirror_count();
    app.active_tab = CONFIG_TAB_INDEX;
    // Nerinyan is enabled by default; toggling it off must lower the Get Maps
    // count, since the summary derives from the Config tab now.
    app.config.focus = ConfigField::MirrorNerinyan;
    app.handle_key(press(KeyCode::Enter));

    unsafe { std::env::remove_var("OSU_COLLECT_CONFIG") };

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
fn space_inside_collection_list_toggles_focused_item() {
    let mut app = make_app();
    app.next_tab();
    // seed one collection and drop into the list
    app.updates
        .set_collections(vec![osu_collect::osu_db::LocalCollection {
            name: "test - 1234".to_string(),
            beatmap_checksums: Vec::new().into(),
        }]);
    app.updates.selection.in_collection_list = true;
    app.updates.selection.collections_state = Some(0);

    let before = app.updates.selection.local_collections[0].selected;
    app.handle_key(press(KeyCode::Char(' ')));
    assert_eq!(
        app.updates.selection.local_collections[0].selected, !before,
        "space toggles the focused list selection"
    );
}

// ── enter on home tab ─────────────────────────────────────────────────────────

#[test]
fn recheck_failed_key_dispatches_on_updates_tab() {
    let mut app = make_app();
    app.next_tab();
    app.updates.set_failed_beatmapset_count(2);

    let cmd = app.handle_key(press(KeyCode::Char('r')));

    assert!(matches!(cmd, Some(AppCommand::RecheckFailedMaps)));
}

#[test]
fn recheck_failed_key_ignored_without_failed_maps() {
    let mut app = make_app();
    app.next_tab();

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

#[test]
fn enter_on_config_chip_opens_login_tab() {
    use osu_collect::app::{ConfigField, HomeField};
    use osu_collect::config::constants::CONFIG_TAB_INDEX;

    let mut app = make_app();
    // Focus a non-text field so Right switches tabs rather than moving the caret.
    app.home.focus = HomeField::Video;
    app.handle_key(press(KeyCode::Right));
    app.handle_key(press(KeyCode::Right));
    assert_eq!(app.active_tab(), CONFIG_TAB_INDEX);
    app.config.focus = ConfigField::AuthChip;

    // The chip is a navigation affordance: enter opens the dedicated login tab
    // and dispatches no command (the tab owns the actual login flow).
    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(cmd.is_none(), "opening the login tab dispatches no command");
    assert!(app.login.is_some(), "login tab must be created");
    assert_eq!(
        Some(app.active_tab()),
        app.login_tab_index(),
        "focus must move to the login tab"
    );
}

#[test]
fn space_on_auth_chip_does_nothing() {
    use osu_collect::app::{ConfigField, HomeField};
    use osu_collect::config::constants::CONFIG_TAB_INDEX;

    let mut app = make_app();
    // Focus a non-text field so Right switches tabs rather than moving the caret.
    app.home.focus = HomeField::Video;
    app.handle_key(press(KeyCode::Right));
    app.handle_key(press(KeyCode::Right));
    assert_eq!(app.active_tab(), CONFIG_TAB_INDEX);
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
    use osu_collect::config::constants::CONFIG_TAB_INDEX;
    let mut app = make_app();
    app.active_tab = CONFIG_TAB_INDEX;
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
    use osu_collect::config::constants::{CONFIG_TAB_INDEX, UPDATES_TAB_INDEX};
    let mut app = config_app_vim(true);
    app.handle_key(press(KeyCode::Char('h')));
    assert_eq!(
        app.active_tab(),
        UPDATES_TAB_INDEX,
        "h switches to the prev tab"
    );
    app.handle_key(press(KeyCode::Char('l')));
    assert_eq!(
        app.active_tab(),
        CONFIG_TAB_INDEX,
        "l switches to the next tab"
    );
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

// ── updates tab: enter does not exit lists ────────────────────────────────────

#[test]
fn enter_inside_collection_list_is_no_op() {
    let mut app = make_app();
    app.next_tab();
    app.updates.selection.in_collection_list = true;

    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(cmd.is_none());
    assert!(
        app.updates.selection.in_collection_list,
        "enter must not close the collection list"
    );
}

// ── config tab: mirror reorder ────────────────────────────────────────────────

#[test]
fn shift_arrow_reorders_config_mirror_and_syncs_pipeline() {
    use osu_collect::app::ConfigField;
    use osu_collect::config::constants::CONFIG_TAB_INDEX;
    use osu_downloader::MirrorKind;

    // Sandbox the config path so the reorder's auto-save never touches the real
    // user config.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    unsafe { std::env::set_var("OSU_COLLECT_CONFIG", path.to_str().unwrap()) };

    let mut app = make_app();
    app.active_tab = CONFIG_TAB_INDEX;
    // Nerinyan is the second built-in in the default order.
    app.config.focus = ConfigField::MirrorNerinyan;
    app.handle_key(shift(KeyCode::Up));

    unsafe { std::env::remove_var("OSU_COLLECT_CONFIG") };

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
    use osu_collect::config::constants::CONFIG_TAB_INDEX;

    let mut app = make_app();
    app.active_tab = CONFIG_TAB_INDEX;
    // Theme is not a mirror row, so shift+down behaves like plain focus movement.
    app.config.focus = ConfigField::Theme;
    app.handle_key(shift(KeyCode::Down));
    assert_eq!(
        app.config.focus,
        ConfigField::VimKeys,
        "shift+arrow off a mirror row moves focus like a plain arrow"
    );
}
