//! Settled download-tab close + `x` toast-dismiss tests.
//!
//! Settled tabs close via `esc`/`q` (close-in-place, no `CancelDownload`);
//! `x` is reserved for toast dismissal and never closes a download tab.
//! Covers settled close, in-progress cancel routing, static-tab no-op, and
//! adjacent-tab navigation. Footer-hint and help-overlay surface checks live
//! in `tests/unit/tui_footer.rs` and `tests/unit/tui_modal.rs` respectively,
//! since those modules are crate-private.

use crate::{
    app::{App, Tab, collection::CollectionPage},
    config::Config,
    download::{DownloadId, DownloadStage},
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

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

/// Push a download page in the given stage and focus its tab.
fn push_page(app: &mut App, id: DownloadId, stage: DownloadStage) {
    let mut page = CollectionPage::new(id, format!("col {id}"), 2);
    page.stage = stage;
    app.downloads.push(page);
    app.active_tab = Tab::Download(app.downloads.len() - 1);
}

// ── esc/q on settled tabs removes the page ───────────────────────────────────

#[test]
fn esc_on_completed_tab_removes_page_and_focuses_left() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    assert_eq!(app.downloads.len(), 1);
    let closed_index = app.active_tab;

    let cmd = app.handle_key(press(KeyCode::Esc));

    assert!(cmd.is_none(), "esc must not emit a command");
    assert!(app.downloads.is_empty(), "completed page must be removed");
    // Only one download tab existed, so focus falls back to config (rightmost
    // static tab — the tab immediately to the left of the closed one).
    assert_eq!(app.active_tab, Tab::from_index(closed_index.to_index() - 1));
    assert_eq!(app.active_tab, Tab::Config);
}

#[test]
fn esc_on_failed_tab_removes_page_and_focuses_left() {
    let mut app = make_app();
    push_page(&mut app, 7, DownloadStage::Failed);
    assert_eq!(app.downloads.len(), 1);

    let cmd = app.handle_key(press(KeyCode::Esc));

    assert!(cmd.is_none());
    assert!(app.downloads.is_empty(), "failed page must be removed");
    assert_eq!(app.active_tab, Tab::Config);
}

#[test]
fn q_closes_middle_download_tab_and_lands_on_previous_download_tab() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    push_page(&mut app, 2, DownloadStage::Completed);
    push_page(&mut app, 3, DownloadStage::Completed);
    // focus the middle download tab (id=2 → Tab::Download(1))
    app.active_tab = Tab::Download(1);

    app.handle_key(press(KeyCode::Char('q')));

    assert_eq!(app.downloads.len(), 2);
    let remaining_ids: Vec<_> = app.downloads.iter().map(|p| p.id).collect();
    assert_eq!(remaining_ids, vec![1, 3]);
    // The tab immediately left of the closed one was the first download tab
    // (id=1), which keeps its Tab::Download(0) slot.
    assert_eq!(app.active_tab, Tab::Download(0));
}

// ── x never closes a download tab ────────────────────────────────────────────

#[test]
fn x_on_completed_tab_is_noop() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    let tab = app.active_tab;

    let cmd = app.handle_key(press(KeyCode::Char('x')));

    assert!(cmd.is_none(), "x must not emit a command");
    assert_eq!(app.downloads.len(), 1, "completed page must persist");
    assert_eq!(app.active_tab, tab, "active tab must not move");
}

#[test]
fn x_on_failed_tab_is_noop() {
    let mut app = make_app();
    push_page(&mut app, 7, DownloadStage::Failed);

    app.handle_key(press(KeyCode::Char('x')));

    assert_eq!(app.downloads.len(), 1, "failed page must persist");
}

#[test]
fn x_on_downloading_tab_is_noop() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Downloading);
    let tab = app.active_tab;

    let cmd = app.handle_key(press(KeyCode::Char('x')));

    assert!(cmd.is_none());
    assert_eq!(app.downloads.len(), 1, "downloading page must persist");
    assert_eq!(app.active_tab, tab, "active tab must not move");
}

#[test]
fn x_on_resolving_tab_is_noop() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Resolving);

    app.handle_key(press(KeyCode::Char('x')));

    assert_eq!(app.downloads.len(), 1);
}

#[test]
fn x_on_rechecking_tab_is_noop() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Rechecking);

    app.handle_key(press(KeyCode::Char('x')));

    assert_eq!(app.downloads.len(), 1);
}

#[test]
fn x_on_pending_tab_is_noop() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Pending);

    app.handle_key(press(KeyCode::Char('x')));

    assert_eq!(app.downloads.len(), 1);
}

// ── x on static tabs has no close effect ──────────────────────────────────────

#[test]
fn x_on_home_tab_does_not_remove_any_download() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    app.active_tab = Tab::Home;

    app.handle_key(press(KeyCode::Char('x')));

    assert_eq!(app.downloads.len(), 1, "downloads must be untouched");
    assert_eq!(app.active_tab, Tab::Home);
}

#[test]
fn x_on_updates_tab_does_not_remove_any_download() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    app.active_tab = Tab::Updates;

    app.handle_key(press(KeyCode::Char('x')));

    assert_eq!(app.downloads.len(), 1);
    assert_eq!(app.active_tab, Tab::Updates);
}

#[test]
fn x_on_config_tab_does_not_remove_any_download() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    app.active_tab = Tab::Config;

    app.handle_key(press(KeyCode::Char('x')));

    assert_eq!(app.downloads.len(), 1);
    assert_eq!(app.active_tab, Tab::Config);
}

// ── q on settled tabs closes in place (no CancelDownload command) ────────────

#[test]
fn q_on_completed_tab_closes_in_place_without_command() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    let closed_index = app.active_tab;

    let cmd = app.handle_key(press(KeyCode::Char('q')));

    assert!(
        cmd.is_none(),
        "q on a completed tab must not emit a CancelDownload command"
    );
    assert!(
        app.downloads.is_empty(),
        "completed page must be removed by q"
    );
    assert_eq!(app.active_tab, Tab::from_index(closed_index.to_index() - 1));
}

#[test]
fn q_on_failed_tab_closes_in_place_without_command() {
    let mut app = make_app();
    push_page(&mut app, 9, DownloadStage::Failed);

    let cmd = app.handle_key(press(KeyCode::Char('q')));

    assert!(cmd.is_none());
    assert!(app.downloads.is_empty());
}

#[test]
fn q_on_running_tab_still_emits_cancel_command() {
    use crate::app::AppCommand;
    let mut app = make_app();
    push_page(&mut app, 4, DownloadStage::Downloading);

    let cmd = app.handle_key(press(KeyCode::Char('q')));

    assert!(
        matches!(cmd, Some(AppCommand::CancelDownload { id: 4 })),
        "q on a running download must request cancellation, got {cmd:?}"
    );
    assert_eq!(app.downloads.len(), 1, "page must stay until runtime acks");
}

// ── x dismisses toasts and never closes the tab behind them ──────────────────

#[test]
fn x_dismisses_toast_and_leaves_settled_tab_open() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    let tab_before = app.active_tab;
    // a visible toast is dismissed by `x`; the settled tab is untouched
    app.toast_err("network unreachable");

    let cmd = app.handle_key(press(KeyCode::Char('x')));

    assert!(cmd.is_none(), "x must not emit a command");
    assert!(app.toasts.is_empty(), "x must dismiss the topmost toast");
    assert_eq!(
        app.downloads.len(),
        1,
        "settled tab must stay open while the toast was dismissed"
    );
    assert_eq!(app.active_tab, tab_before);
}

#[test]
fn x_after_dismiss_does_not_close_settled_tab() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    app.toast_err("network unreachable");

    // first `x`: dismisses the toast, tab stays
    app.handle_key(press(KeyCode::Char('x')));
    assert!(app.toasts.is_empty());
    assert_eq!(app.downloads.len(), 1);

    // second `x`: no toast left, but `x` is toast-only — the tab still stays
    app.handle_key(press(KeyCode::Char('x')));
    assert_eq!(
        app.downloads.len(),
        1,
        "x must never close a settled download tab"
    );
}

#[test]
fn x_on_static_tab_without_error_is_unchanged() {
    let mut app = make_app();
    app.active_tab = Tab::Config;
    // no error toast — x must remain a no-op on static tabs as before
    let cmd = app.handle_key(press(KeyCode::Char('x')));
    assert!(cmd.is_none());
    assert_eq!(app.active_tab, Tab::Config);
}

// ── help overlay surface ──────────────────────────────────────────────────────

#[test]
fn help_overlay_lists_close_completed_tab() {
    use ratatui::{Terminal, backend::TestBackend};

    let mut app = make_app();
    // Help is per-tab now; the close-tab binding lives in the download section,
    // so focus a download tab before opening the overlay.
    push_page(&mut app, 1, DownloadStage::Completed);
    app.help_open = true;

    let backend = TestBackend::new(80, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            crate::tui::draw(frame, &app);
        })
        .expect("render");

    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();

    assert!(
        rendered.contains("close completed tab"),
        "help overlay must list `close completed tab` action"
    );
}
