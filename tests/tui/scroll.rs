/// Tests for `ListState`-driven panel scrolling.
///
/// These verify that the focused row follows the viewport at small terminal
/// sizes — the `ListState` scroll target is decoupled from the highlight, so the
/// focused row scrolls into view even when it styles itself (CTA / auth chip).
use osu_collect::app::update_source::CollectionEntry;
use osu_collect::app::{App, UpdateField};
use osu_collect::config::Config;
use osu_collect::tui::draw;
use ratatui::{Terminal, backend::TestBackend};

fn make_app() -> App {
    App::new(Config::default())
}

fn render_content(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn updates_tab_renders_at_minimum_size() {
    let mut app = make_app();
    app.next_tab();
    // A long expanded collection list overflows an 80×18 viewport. With the
    // cursor on the last entry, the `ListState` scroll target follows it: the
    // bottom row is visible and the top row has scrolled out of view.
    app.home.update.selection.focus = UpdateField::Collections;
    app.home.update.selection.in_collection_list = true;
    for i in 0..20u64 {
        app.home
            .update
            .selection
            .local_collections
            .push(CollectionEntry {
                name: format!("coll-{i:02}"),
                collection_id: Some(i),
                beatmap_count: 1,
                selected: false,
                removed_count: 0,
            });
    }
    app.home.update.selection.collections_state = Some(19);

    let content = render_content(&app, 80, 18);
    assert!(
        content.contains("coll-19"),
        "the focused bottom row must follow the viewport: {content}"
    );
    assert!(
        !content.contains("coll-00"),
        "the window must have scrolled down past the top row: {content}"
    );
}
