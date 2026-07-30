use super::{
    COVER_GAP, COVER_WIDTH_MIN, MIN_TEXT_WIDTH, MasterDetail, Pane, PreviewWidths, render,
    seam_row, square_cover_width, wide_cover_width, wrap_to_lines,
};
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
    preview_max_offset: &'a Cell<usize>,
) -> MasterDetail<'a> {
    MasterDetail {
        status: None,
        list_title: LIST_TITLE.into(),
        list_meta: None,
        list_len: 3,
        list_items: Box::new(|window| window.map(|i| ListItem::new(format!("row {i}"))).collect()),
        list_selected: Some(0),
        list_offset,
        preview_title: PREVIEW_TITLE.into(),
        preview_meta: None,
        preview_items: Box::new(|_| sample_items(2)),
        preview_selected: Some(0),
        preview_offset,
        preview_max_offset,
        preview_image: None,
        preview_lead: None,
        focused: Pane::List,
    }
}

#[test]
fn wide_area_shows_both_panes() {
    let list_offset = Cell::new(0);
    let preview_offset = Cell::new(0);
    let preview_max = Cell::new(0);
    let view = sample_view(&list_offset, &preview_offset, &preview_max);

    let output = render_to_string(80, 20, &view);
    assert!(output.contains("ITEMS"), "list title should render");
    assert!(output.contains("PREVIEW"), "preview title should render");
}

#[test]
fn narrow_area_shows_only_focused_pane() {
    let list_offset = Cell::new(0);
    let preview_offset = Cell::new(0);
    let preview_max = Cell::new(0);
    let view = sample_view(&list_offset, &preview_offset, &preview_max);

    let output = render_to_string(40, 20, &view);
    assert!(output.contains("ITEMS"), "focused list title should render");
    assert!(
        !output.contains("PREVIEW"),
        "blurred preview pane should not render in single-pane fallback"
    );
}

#[test]
fn the_preview_row_builder_is_called_with_the_pane_text_width() {
    let list_offset = Cell::new(0);
    let preview_offset = Cell::new(0);
    let preview_max = Cell::new(0);
    let mut view = sample_view(&list_offset, &preview_offset, &preview_max);
    view.preview_items = Box::new(|w| {
        vec![ListItem::new(format!(
            "built {} {} {}",
            w.beside, w.full, w.seam
        ))]
    });

    // 80 wide → a 32-column list pane, 48 for the preview, 44 inside its border
    // and padding. No cover, so both bands are that width and no row is beside
    // one.
    let output = render_to_string(80, 20, &view);
    assert!(
        output.contains("built 44 44 0"),
        "rows are built at the preview's resolved text width:\n{output}"
    );
}

#[test]
fn the_seam_discounts_the_title_lead() {
    // The image spans 5 screen rows from the pane's top; the lead owns the first,
    // so the builder's rows 0..4 are the ones beside it.
    assert_eq!(seam_row(5, 1), 4, "one lead line shifts the seam up one");
    assert_eq!(seam_row(5, 2), 3, "a wrapped lead shifts it up two");
    assert_eq!(
        seam_row(1, 2),
        0,
        "a cover shorter than the lead leaves no beside-the-cover rows"
    );
}

#[test]
fn preview_widths_size_a_row_by_the_band_it_lands_in() {
    let widths = PreviewWidths {
        beside: 20,
        full: 44,
        seam: 2,
        below: 6,
    };
    assert_eq!(
        widths.at(0),
        20,
        "the first band row takes the cover column"
    );
    assert_eq!(widths.at(1), 20, "so does the last one before the seam");
    assert_eq!(
        widths.at(2),
        44,
        "the seam row itself is clear of the image"
    );
    assert_eq!(widths.at(9), 44, "and everything below it");
}

#[test]
fn a_straddling_block_waits_for_the_seam_only_when_it_fits_below() {
    let widths = PreviewWidths {
        beside: 20,
        full: 44,
        seam: 4,
        below: 6,
    };
    assert_eq!(
        widths.pushdown(2, 6),
        2,
        "a block starting two rows short of the seam waits those two rows out"
    );
    assert_eq!(
        widths.pushdown(4, 6),
        0,
        "a block already clear of the image waits for nothing"
    );
    assert_eq!(
        widths.pushdown(2, 7),
        0,
        "one row too tall for the space below the cover stays where it is"
    );
}

#[test]
fn empty_list_items_does_not_panic() {
    let list_offset = Cell::new(0);
    let preview_offset = Cell::new(0);
    let preview_max_offset = Cell::new(0);
    let view = MasterDetail {
        status: Some(Line::from("0 of 0 selected")),
        list_title: LIST_TITLE.into(),
        list_meta: None,
        list_len: 0,
        list_items: Box::new(|_| Vec::new()),
        list_selected: None,
        list_offset: &list_offset,
        preview_title: PREVIEW_TITLE.into(),
        preview_meta: None,
        preview_items: Box::new(|_| Vec::new()),
        preview_selected: None,
        preview_offset: &preview_offset,
        preview_max_offset: &preview_max_offset,
        preview_image: None,
        preview_lead: None,
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
    let preview_max = Cell::new(0);
    let mut view = sample_view(&list_offset, &preview_offset, &preview_max);
    view.list_meta = Some(Line::from("651 new maps"));

    let output = render_to_string(80, 20, &view);
    assert!(
        output.contains("651 new maps"),
        "list title-right meta should render in the panel's top border break"
    );
}

#[test]
fn both_columns_never_eat_into_the_text_floor() {
    // Every width either column can offer must leave MIN_TEXT_WIDTH intact — that
    // invariant is why `place_cover` doesn't re-check the text width.
    for inner_width in 0..=2000u16 {
        for offer in [
            wide_cover_width(inner_width),
            square_cover_width(inner_width),
        ] {
            let Some(w) = offer else { continue };
            assert!(
                inner_width - w - COVER_GAP >= MIN_TEXT_WIDTH,
                "inner {inner_width} offered {w} columns, starving the text"
            );
        }
    }
}

#[test]
fn the_square_column_is_narrower_than_the_wide_so_collapsing_frees_text() {
    // Wherever both are offered, the square (two fifths) is at most the wide
    // (three fifths) — collapsing to it never widens the cover.
    for inner_width in 0..=2000u16 {
        if let (Some(wide), Some(square)) = (
            wide_cover_width(inner_width),
            square_cover_width(inner_width),
        ) {
            assert!(
                square <= wide,
                "inner {inner_width}: square {square} must not exceed wide {wide}"
            );
        }
    }
}

#[test]
fn wide_column_tracks_three_fifths_above_its_threshold() {
    assert_eq!(wide_cover_width(100), Some(60), "100/5*3");
    assert_eq!(wide_cover_width(200), Some(120), "200/5*3");
    // Below WIDE_COVER_WIDTH the wide column bows out (the square is used).
    assert_eq!(
        wide_cover_width(40),
        None,
        "40 → 40/5*3=24, below the wide threshold"
    );
    assert_eq!(
        wide_cover_width(u16::MAX),
        Some(u16::MAX / 5 * 3),
        "the widest possible pane must not overflow"
    );
}

#[test]
fn square_column_tracks_two_fifths_down_to_its_minimum() {
    assert_eq!(square_cover_width(100), Some(40), "100/5*2");
    // Floor-bound narrow pane: whatever's left after the text keeps MIN_TEXT_WIDTH.
    assert_eq!(square_cover_width(40), Some(16), "40-2-22, floor-bound");
    let floor = COVER_WIDTH_MIN + COVER_GAP + MIN_TEXT_WIDTH;
    assert_eq!(
        square_cover_width(floor),
        Some(COVER_WIDTH_MIN),
        "at the floor the square is exactly its minimum"
    );
    assert_eq!(
        square_cover_width(floor - 1),
        None,
        "one column short: no cover, text-only"
    );
    assert_eq!(square_cover_width(0), None, "an empty pane has no cover");
}

#[test]
fn wrap_to_lines_keeps_a_short_title_on_one_line() {
    assert_eq!(wrap_to_lines("Short Title", 40, 2), vec!["Short Title"]);
}

#[test]
fn wrap_to_lines_wraps_at_word_boundaries_within_the_budget() {
    // Fits in two lines exactly: clean word-boundary break, no ellipsis. This is
    // the real case — a long title in the ~22-col text column beside the cover.
    let lines = wrap_to_lines("one two three", 9, 2);
    assert_eq!(lines, vec!["one two", "three"]);
}

#[test]
fn wrap_to_lines_ellipsises_the_last_line_when_it_overruns() {
    // The title needs three lines at this width; capping at two marks the cut.
    let lines = wrap_to_lines("alpha beta gamma delta epsilon", 11, 2);
    assert_eq!(lines.len(), 2);
    assert!(
        lines[1].ends_with('…'),
        "the final kept line marks the truncation, got {:?}",
        lines[1]
    );
    assert!(
        lines.iter().all(|l| super::display_width(l) <= 11),
        "no wrapped line may exceed the width"
    );
}

#[test]
fn wrap_to_lines_hard_splits_a_word_longer_than_the_width() {
    // A single unbroken word (a URL-like title) still gets chopped to fit.
    let lines = wrap_to_lines("aaaaaaaaaaaa", 4, 2);
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|l| super::display_width(l) <= 4));
    assert!(lines[1].ends_with('…'), "the overrun is still marked");
}

#[test]
fn wrap_to_lines_measures_full_width_glyphs_by_columns() {
    // Full-width CJK glyphs are two columns each; a common osu title case.
    let lines = wrap_to_lines("ありがとうございます", 8, 2);
    assert!(
        lines.iter().all(|l| super::display_width(l) <= 8),
        "CJK titles must wrap by display columns, not byte or char count: {lines:?}"
    );
}

#[test]
fn wrap_to_lines_is_empty_for_a_zero_budget() {
    assert!(wrap_to_lines("anything", 0, 2).is_empty());
    assert!(wrap_to_lines("anything", 40, 0).is_empty());
}

#[test]
fn owned_preview_title_renders() {
    let list_offset = Cell::new(0);
    let preview_offset = Cell::new(0);
    let preview_max = Cell::new(0);
    let mut view = sample_view(&list_offset, &preview_offset, &preview_max);
    // A proper-noun preview title (original case preserved) — the collection
    // name in the update browse.
    view.preview_title = "AIM GYM MEGAPACK".to_string().into();

    let output = render_to_string(80, 20, &view);
    assert!(
        output.contains("AIM GYM MEGAPACK"),
        "an owned preview title should render, case preserved"
    );
}
