use super::{AuthLoginState, ConfigField, ConfigTab};
use crate::config::Config;
use crate::download::ArchiveValidation;

fn tab_logged_out() -> ConfigTab {
    let mut tab = ConfigTab::new(&Config::default());
    tab.login_state = AuthLoginState::LoggedOut;
    tab
}

fn tab_logged_in() -> ConfigTab {
    let mut tab = ConfigTab::new(&Config::default());
    tab.login_state = AuthLoginState::LoggedIn;
    tab
}

#[test]
fn login_flow_marks_in_progress_without_message() {
    let mut tab = tab_logged_out();
    tab.set_login_in_progress();
    assert_eq!(tab.login_state, AuthLoginState::InProgress(String::new()));
    assert!(tab.message.is_none());
}

#[test]
fn login_flow_success() {
    let mut tab = tab_logged_out();
    tab.set_login_in_progress();
    tab.set_login_complete();
    assert_eq!(tab.login_state, AuthLoginState::LoggedIn);
}

#[test]
fn login_flow_error_returns_to_logged_out() {
    let mut tab = tab_logged_out();
    tab.set_login_in_progress();
    tab.set_login_failed();
    assert_eq!(tab.login_state, AuthLoginState::LoggedOut);
}

#[test]
fn cancel_login_returns_to_logged_out_and_clears_loading() {
    let mut tab = tab_logged_out();
    tab.set_loading("logging in...");
    assert!(matches!(tab.login_state, AuthLoginState::InProgress(_)));
    assert!(
        tab.message.is_some(),
        "loading message set while in progress"
    );

    // The cancel toast itself is pushed at the App level; the tab's job is to
    // clear its in-progress loading status on the terminal transition.
    tab.set_login_failed();
    assert_eq!(tab.login_state, AuthLoginState::LoggedOut);
    assert!(tab.message.is_none(), "terminal transition clears loading");
}

#[test]
fn logout_clears_state() {
    let mut tab = tab_logged_in();
    tab.set_logged_out();
    assert_eq!(tab.login_state, AuthLoginState::LoggedOut);
}

#[test]
fn logout_sets_loading_message() {
    let mut tab = tab_logged_in();
    tab.set_loading("logging out...");
    let msg = tab.message.as_ref().unwrap();
    assert_eq!(msg.text, "logging out...");
}

#[test]
fn next_field_cycles_through_auth_chip() {
    let mut tab = tab_logged_in();
    tab.focus = ConfigField::Prereleases;
    tab.next_field();
    assert_eq!(tab.focus, ConfigField::AuthChip);
    tab.next_field();
    assert_eq!(tab.focus, ConfigField::Theme);
    tab.next_field();
    assert_eq!(tab.focus, ConfigField::VimKeys);
    tab.next_field();
    assert_eq!(tab.focus, ConfigField::MirrorOsuDirect);
}

#[test]
fn prev_field_cycles_through_auth_chip() {
    let mut tab = tab_logged_in();
    tab.focus = ConfigField::MirrorOsuDirect;
    tab.prev_field();
    assert_eq!(tab.focus, ConfigField::VimKeys);
    tab.prev_field();
    assert_eq!(tab.focus, ConfigField::Theme);
    tab.prev_field();
    assert_eq!(tab.focus, ConfigField::AuthChip);
    tab.prev_field();
    assert_eq!(tab.focus, ConfigField::Prereleases);
}

#[test]
fn auth_chip_present_when_logged_out() {
    let mut tab = tab_logged_out();
    tab.focus = ConfigField::AuthChip;
    tab.next_field();
    assert_eq!(tab.focus, ConfigField::Theme);
    tab.prev_field();
    assert_eq!(tab.focus, ConfigField::AuthChip);
}

#[test]
fn all_fields_form_complete_cycle() {
    let mut tab = tab_logged_in();
    let start = tab.focus;
    // Cycling the full field count must return to the starting field.
    let total = tab.fields().len();
    for _ in 0..total {
        tab.next_field();
    }
    assert_eq!(tab.focus, start, "next_field must complete a full cycle");
}

#[test]
fn cycle_archive_validation_wraps_through_all_variants() {
    let mut tab = tab_logged_in();
    tab.archive_validation = ArchiveValidation::Off;
    tab.cycle_archive_validation();
    assert_eq!(tab.archive_validation, ArchiveValidation::Magic);
    tab.cycle_archive_validation();
    assert_eq!(tab.archive_validation, ArchiveValidation::Eocd);
    tab.cycle_archive_validation();
    assert_eq!(tab.archive_validation, ArchiveValidation::Off);
}

#[test]
fn config_threads_stepper_increments_by_one() {
    use crate::app::ConfigField;

    let mut tab = tab_logged_in();
    tab.focus = ConfigField::DownloadThreads;
    tab.threads.value = "2".to_string();

    tab.step_up();

    assert_eq!(tab.resolved_threads(), 3);
}

#[test]
fn config_threads_stepper_decrements_by_one() {
    use crate::app::ConfigField;

    let mut tab = tab_logged_in();
    tab.focus = ConfigField::DownloadThreads;
    tab.threads.value = "4".to_string();

    tab.step_down();

    assert_eq!(tab.resolved_threads(), 3);
}

#[test]
fn config_threads_stepper_does_not_go_below_one() {
    let mut tab = tab_logged_in();
    tab.threads.value = "1".to_string();

    tab.step_down();

    assert_eq!(tab.resolved_threads(), 1);
}

#[test]
fn config_threads_stepper_does_not_exceed_default_threads() {
    let mut tab = tab_logged_in();
    let max = tab.default_threads;
    tab.threads.value = max.to_string();

    tab.step_up();

    assert_eq!(tab.resolved_threads(), max);
}

#[test]
fn config_threads_digit_key_does_not_mutate_value() {
    use crate::app::ConfigField;

    let mut tab = tab_logged_in();
    tab.focus = ConfigField::DownloadThreads;
    tab.threads.value = "3".to_string();

    tab.handle_char('9');

    assert_eq!(tab.threads.value, "3");
}

#[test]
fn config_download_threads_is_not_text_input() {
    use crate::app::ConfigField;
    assert!(!ConfigField::DownloadThreads.is_text_input());
    assert!(ConfigField::DownloadThreads.is_stepper());
}

#[test]
fn reorder_focused_mirror_moves_row_and_keeps_focus() {
    use crate::mirrors::MirrorKind;

    let mut tab = ConfigTab::new(&Config::default());
    // Nerinyan is the second built-in in the default order; move it up.
    tab.focus = ConfigField::MirrorNerinyan;
    assert!(
        tab.reorder_focused_mirror(true),
        "moving a mid-list mirror up must report a change"
    );
    assert_eq!(tab.mirror_order[0], MirrorKind::Nerinyan);
    assert_eq!(tab.mirror_order[1], MirrorKind::OsuDirect);
    assert_eq!(
        tab.focus,
        ConfigField::MirrorNerinyan,
        "focus follows the moved mirror"
    );
}

#[test]
fn reorder_focused_mirror_is_noop_at_edge_and_off_mirror() {
    let mut tab = ConfigTab::new(&Config::default());
    // The top built-in cannot move further up.
    tab.focus = ConfigField::MirrorOsuDirect;
    assert!(!tab.reorder_focused_mirror(true));
    // A non-mirror row is never part of the reorder set.
    tab.focus = ConfigField::Theme;
    assert!(!tab.focus_is_builtin_mirror());
    assert!(!tab.reorder_focused_mirror(false));
}

#[test]
fn build_config_writes_reordered_order_and_omits_default() {
    use crate::mirrors::MirrorKind;

    let default_tab = ConfigTab::new(&Config::default());
    assert!(
        default_tab.build_config().unwrap().mirror.order.is_empty(),
        "an untouched order must serialize empty (BUILTINS default)"
    );

    let mut tab = ConfigTab::new(&Config::default());
    tab.focus = ConfigField::MirrorNerinyan;
    tab.reorder_focused_mirror(true);
    let built = tab.build_config().unwrap();
    assert_eq!(
        built.mirror.order.first().map(|s| s.as_ref()),
        Some(MirrorKind::Nerinyan.host()),
        "a reordered tab writes the host-key order"
    );
    // The written order reconstructs the same ranking.
    assert_eq!(built.mirror.ordered_builtins()[0], MirrorKind::Nerinyan);
}

#[test]
fn nav_order_follows_reordered_mirrors() {
    let mut tab = ConfigTab::new(&Config::default());
    tab.focus = ConfigField::MirrorNerinyan;
    tab.reorder_focused_mirror(true); // Nerinyan becomes the first mirror row
    // Stepping down from vim-keys lands on the new first mirror.
    tab.focus = ConfigField::VimKeys;
    tab.next_field();
    assert_eq!(tab.focus, ConfigField::MirrorNerinyan);
}
