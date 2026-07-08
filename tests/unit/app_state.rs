use crate::{
    app::{App, AppCommand, Tab},
    config::Config,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
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
