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
