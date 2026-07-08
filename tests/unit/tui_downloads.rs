//! Downloads-tab render tests: empty state, run-list rows + counts meta, and
//! the history-record preview panel.

use crate::app::{App, CollectionPage, Tab};
use crate::config::Config;
use crate::download::{DownloadStage, DownloadSummary};
use ratatui::{Terminal, backend::TestBackend};

fn render_app(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");
    terminal
        .draw(|frame| {
            crate::tui::draw(frame, app);
        })
        .expect("app should render");
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

fn make_app() -> App {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Downloads;
    app
}

fn cancelled_record(app: &mut App, id: u64, title: &str) {
    let mut page = CollectionPage::new(id, title.to_string(), 2);
    page.stage = DownloadStage::Downloading;
    page.total_maps = 40;
    page.stats.downloaded = 12;
    app.downloads.push(page);
    app.handle_cancel_result(id, true);
}

#[test]
fn empty_state_renders_a_placeholder() {
    let app = make_app();

    let output = render_app(&app, 100, 24);

    assert!(output.contains("DOWNLOADS"), "panel title renders");
    assert!(
        output.contains("no downloads yet"),
        "empty list must say so: {output}"
    );
}

#[test]
fn list_shows_active_and_past_rows_with_counts_meta() {
    let mut app = make_app();
    let mut active = CollectionPage::new(1, "fresh run".to_string(), 2);
    active.stage = DownloadStage::Downloading;
    active.total_maps = 40;
    active.stats.downloaded = 10;
    app.downloads.push(active);
    let mut settled = CollectionPage::new(2, "old run".to_string(), 2);
    settled.stage = DownloadStage::Completed;
    settled.total_maps = 5;
    settled.stats.downloaded = 5;
    settled.summary = Some(DownloadSummary {
        downloaded: 5,
        skipped: 0,
        failed: 0,
        unverified: 0,
    });
    app.downloads.push(settled);

    let output = render_app(&app, 100, 24);

    assert!(output.contains("fresh run"), "active row renders: {output}");
    assert!(output.contains("old run"), "settled row renders");
    assert!(
        output.contains("1 active") && output.contains("1 past"),
        "counts meta must render: {output}"
    );
    assert!(output.contains("10/40"), "active row carries progress");
}

#[test]
fn record_preview_shows_summary_kv_rows() {
    let mut app = make_app();
    cancelled_record(&mut app, 1, "Tekno Collection");
    app.downloads_tab.selected = 0;

    let output = render_app(&app, 100, 24);

    assert!(
        output.contains("TEKNO COLLECTION"),
        "preview panel is named after the run: {output}"
    );
    assert!(output.contains("cancelled"), "stage pill renders");
    assert!(output.contains("12/40"), "downloaded ratio renders");
    assert!(output.contains("just now"), "age renders");
}

#[test]
fn narrow_terminal_collapses_to_the_focused_pane() {
    let mut app = make_app();
    cancelled_record(&mut app, 1, "Tekno Collection");
    app.downloads_tab.selected = 0;
    app.downloads_tab.preview_focused = true;

    // Below the split threshold only the focused (preview) pane renders.
    let output = render_app(&app, 56, 24);

    assert!(
        output.contains("TEKNO COLLECTION"),
        "preview owns the narrow body: {output}"
    );
    assert!(
        !output.contains("1 past"),
        "list meta must not render while collapsed to the preview"
    );
}
