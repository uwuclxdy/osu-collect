use super::{MasterDetail, Pane, render};
use ratatui::{Terminal, backend::TestBackend, text::Line, widgets::ListItem};
use std::cell::Cell;

const LIST_TITLE: &str = " ITEMS ";
const PREVIEW_TITLE: &str = " PREVIEW ";

fn render_to_string(width: u16, height: u16, view: &MasterDetail<'_>) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");
    terminal
        .draw(|frame| {
            let area = frame.area();
            render(frame, area, view);
        })
        .expect("view should render");
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

fn sample_items(n: usize) -> Vec<ListItem<'static>> {
    (0..n).map(|i| ListItem::new(format!("row {i}"))).collect()
}

fn sample_view<'a>(
    list_offset: &'a Cell<usize>,
    preview_offset: &'a Cell<usize>,
) -> MasterDetail<'a> {
    MasterDetail {
        status: None,
        list_title: LIST_TITLE.into(),
        list_meta: None,
        list_items: sample_items(3),
        list_selected: Some(0),
        list_offset,
        preview_title: PREVIEW_TITLE.into(),
        preview_meta: None,
        preview_items: sample_items(2),
        preview_selected: Some(0),
        preview_offset,
        preview_image: None,
        focused: Pane::List,
    }
}

#[test]
fn wide_area_shows_both_panes() {
    let list_offset = Cell::new(0);
    let preview_offset = Cell::new(0);
    let view = sample_view(&list_offset, &preview_offset);

    let output = render_to_string(80, 20, &view);
    assert!(output.contains("ITEMS"), "list title should render");
    assert!(output.contains("PREVIEW"), "preview title should render");
}

#[test]
fn narrow_area_shows_only_focused_pane() {
    let list_offset = Cell::new(0);
    let preview_offset = Cell::new(0);
    let view = sample_view(&list_offset, &preview_offset);

    let output = render_to_string(40, 20, &view);
    assert!(output.contains("ITEMS"), "focused list title should render");
    assert!(
        !output.contains("PREVIEW"),
        "blurred preview pane should not render in single-pane fallback"
    );
}

#[test]
fn empty_list_items_does_not_panic() {
    let list_offset = Cell::new(0);
    let preview_offset = Cell::new(0);
    let view = MasterDetail {
        status: Some(Line::from("0 of 0 selected")),
        list_title: LIST_TITLE.into(),
        list_meta: None,
        list_items: Vec::new(),
        list_selected: None,
        list_offset: &list_offset,
        preview_title: PREVIEW_TITLE.into(),
        preview_meta: None,
        preview_items: Vec::new(),
        preview_selected: None,
        preview_offset: &preview_offset,
        preview_image: None,
        focused: Pane::List,
    };

    // Only asserting no panic; the empty-list scroll/highlight path is the
    // regression surface here.
    let _ = render_to_string(80, 20, &view);
}

#[test]
fn list_meta_renders_in_top_border() {
    let list_offset = Cell::new(0);
    let preview_offset = Cell::new(0);
    let mut view = sample_view(&list_offset, &preview_offset);
    view.list_meta = Some(Line::from("651 new maps"));

    let output = render_to_string(80, 20, &view);
    assert!(
        output.contains("651 new maps"),
        "list title-right meta should render in the panel's top border break"
    );
}

#[test]
fn owned_preview_title_renders() {
    let list_offset = Cell::new(0);
    let preview_offset = Cell::new(0);
    let mut view = sample_view(&list_offset, &preview_offset);
    // A proper-noun preview title (original case preserved) — the collection
    // name in the update browse.
    view.preview_title = "AIM GYM MEGAPACK".to_string().into();

    let output = render_to_string(80, 20, &view);
    assert!(
        output.contains("AIM GYM MEGAPACK"),
        "an owned preview title should render, case preserved"
    );
}
