use super::{
    ButtonProminence, ItemHeights, ListRows, button_item, button_item_with_loading_cue, cycle_item,
    download_button_label_with_size, input_cursor_col, input_item, message_style, multi_chip_item,
    panel_content_width, render_list, render_scrollable_panel, render_scrollbar,
    render_windowed_list, search_box_cursor_col, search_box_item, set_panel_cursor,
    truncate_to_width,
};
use crate::app::InputField;
use crate::download::BeatmapStage;
use crate::tui::{
    FILL_BLOCK, FILL_SHADE, FILL_SPACE, GLYPH_BLOCK, GLYPH_SHADE, GLYPH_SPACE, accent, bg_hover,
    danger, glyph_fill, success, text, text_dim, text_faint, warning,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    layout::Rect,
    style::Modifier,
    text::{Line, Text},
    widgets::ListItem,
};

/// `n` list items, each `height` lines tall. Heights are read back off the real
/// [`ListItem::height`] the renderer uses, so a fixture can't claim a shape
/// ratatui would not agree with.
fn items_tall(n: usize, height: usize) -> Vec<ListItem<'static>> {
    (0..n)
        .map(|index| ListItem::new(Text::from(vec![Line::from(format!("row {index}")); height])))
        .collect()
}

/// One string per buffer row, symbols only. `assert_buffer_lines` compares
/// styles too, and every colour here comes from the runtime theme, so a
/// style-aware assertion would pin the palette instead of the layout.
fn buffer_rows(buf: &ratatui::buffer::Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

/// Drives [`set_panel_cursor`] through a real frame and reads back the terminal
/// caret. Returns `None` when the frame left the cursor hidden, or `Some((x, y))`
/// when it was positioned (ratatui applies the request after the buffer flush).
///
/// `heights` is the row height of each item, in item order.
fn panel_cursor(
    inner: Rect,
    heights: &[usize],
    focused_index: usize,
    start: usize,
    cursor_col: Option<u16>,
) -> Option<(u16, u16)> {
    let items: Vec<ListItem<'static>> = heights
        .iter()
        .flat_map(|&height| items_tall(1, height))
        .collect();
    let heights = ItemHeights::of(&items);
    let mut terminal = Terminal::new(TestBackend::new(64, 24)).expect("test backend");
    terminal
        .draw(|frame| set_panel_cursor(frame, inner, &heights, focused_index, start, cursor_col))
        .expect("frame renders");
    let backend = terminal.backend();
    backend
        .cursor_visible()
        .then(|| (backend.cursor_position().x, backend.cursor_position().y))
}

#[test]
fn input_cursor_col_counts_prefix_label_and_value() {
    // `new` parks the caret at the end, so the column lands past the full value.
    let field = InputField::new("Threads", "ab", ""); // label lowercases to "threads" (7)
    // focus marker (2) + label cell (7 + 2-space gap = 9) + caret offset (2) = 13
    assert_eq!(input_cursor_col(&field, 0), 13);
}

#[test]
fn input_cursor_col_pads_label_to_group_width() {
    // A wider group column left-pads the label, pushing the value column right.
    let field = InputField::new("Threads", "ab", ""); // "threads" (7)
    // focus marker (2) + label cell (10 + 2-space gap = 12) + caret (2) = 16
    assert_eq!(input_cursor_col(&field, 10), 16);
}

#[test]
fn input_cursor_col_tracks_caret_offset_not_value_length() {
    let mut field = InputField::new("Threads", "ab", "");
    field.caret_home();
    // caret at 0: focus marker (2) + label cell (7 + 2 = 9) + 0 = 11
    assert_eq!(input_cursor_col(&field, 0), 11);
}

/// Every row one line tall — the shape every form list had before the boxed
/// search input, and the shape the parity pins below hold fixed.
const FLAT: &[usize] = &[1; 16];

#[test]
fn panel_cursor_none_when_no_column() {
    let inner = Rect::new(2, 3, 40, 10);
    assert_eq!(panel_cursor(inner, FLAT, 5, 0, None), None);
}

#[test]
fn panel_cursor_none_when_row_scrolled_out() {
    let inner = Rect::new(2, 3, 40, 10);
    assert_eq!(panel_cursor(inner, FLAT, 12, 0, Some(4)), None);
}

#[test]
fn panel_cursor_maps_row_and_clamps_column() {
    let inner = Rect::new(2, 3, 10, 10); // x=2, width=10 → last col 11
    // focused row 4, window starts at 2 → visible row 2 → y = 3 + 2 = 5
    assert_eq!(panel_cursor(inner, FLAT, 4, 2, Some(5)), Some((7, 5)));
    // column past the edge clamps to inner.x + width - 1 = 11
    assert_eq!(panel_cursor(inner, FLAT, 4, 2, Some(99)), Some((11, 5)));
}

#[test]
fn panel_cursor_counts_rows_not_items_past_a_tall_row() {
    let inner = Rect::new(2, 3, 10, 10);
    // Item 1 is the 3-row search box: item 3 starts 2 rows further down than a
    // flat list would put it (1 + 3 + 1 = 5 rows in, not 3).
    let heights = &[1, 3, 1, 1, 1];
    assert_eq!(panel_cursor(inner, heights, 3, 0, Some(4)), Some((6, 8)));
    // The tall row's own caret parks on its middle line, not its top border.
    assert_eq!(panel_cursor(inner, heights, 1, 0, Some(4)), Some((6, 5)));
}

/// The parity pin for the flat path: a panel whose rows are all one line tall
/// must render byte-for-byte and park its caret on the same cell as it did
/// before list rows could be taller than a line. Every index→row conversion in
/// `ItemHeights` runs here, so a change that only holds for multi-row items
/// reds this instead of silently shifting Config, Login, and Downloads.
#[test]
fn one_line_form_renders_and_parks_the_caret_unchanged() {
    let offset = std::cell::Cell::new(0);
    let mut terminal = Terminal::new(TestBackend::new(20, 8)).expect("test backend");
    terminal
        .draw(|frame| {
            render_scrollable_panel(
                frame,
                Rect::new(0, 0, 20, 8),
                " FORM ",
                None,
                items_tall(6, 1),
                2,
                true,
                Some(3),
                true,
                true,
                &offset,
            );
        })
        .expect("frame renders");
    assert_eq!(
        buffer_rows(terminal.backend().buffer()),
        [
            "╭ FORM ────────────╮",
            "│ row 0            │",
            "│ row 1            │",
            "│ row 2            │",
            "│ row 3            │",
            "│ row 4            │",
            "│ row 5            │",
            "╰──────────────────╯",
        ]
    );
    let backend = terminal.backend();
    assert!(backend.cursor_visible(), "a caret column parks the cursor");
    // inner starts at (2, 1) (border + left padding); focused item 2 with the
    // window at the top sits 2 rows in, and the caret column is 3 past that.
    assert_eq!(
        (backend.cursor_position().x, backend.cursor_position().y),
        (5, 3)
    );
    assert_eq!(offset.get(), 0, "a form that fits never scrolls");
}

#[test]
fn panel_cursor_hidden_when_a_tall_row_does_not_fit() {
    // A 2-row viewport can't hold the 3-row box at all, so ratatui draws none of
    // it and no caret is placed — a flat row count would have said row 1 fits.
    let inner = Rect::new(2, 3, 10, 2);
    assert_eq!(panel_cursor(inner, &[1, 3], 1, 0, Some(4)), None);
}

#[test]
fn panel_content_width_is_the_block_inner() {
    // Two border cells plus the panel's 1-cell padding on each side.
    assert_eq!(panel_content_width(Rect::new(0, 0, 24, 5)), 20);
    // Narrower than its own chrome collapses to nothing rather than underflowing.
    assert_eq!(panel_content_width(Rect::new(0, 0, 3, 5)), 0);
}

#[test]
fn search_box_frames_the_value_across_the_panel_width() {
    let mut field = InputField::new("query", "", "artist, title, mapper, tags…");
    field.set_value("nekodex");
    let offset = std::cell::Cell::new(0);
    let mut terminal = Terminal::new(TestBackend::new(24, 4)).expect("test backend");
    terminal
        .draw(|frame| {
            let _ = render_list(
                frame,
                Rect::new(0, 0, 24, 4),
                vec![search_box_item(&field, true, true, 24)],
                Some(0),
                false,
                &offset,
            );
        })
        .expect("frame renders");
    let rows = buffer_rows(terminal.backend().buffer());
    assert_eq!(
        rows,
        [
            "  ╭────────────────────╮",
            "✎ │ nekodex            │",
            "  ╰────────────────────╯",
            "                        ",
        ]
    );
    // The caret column is what the panel hands `set_panel_cursor`, so it has to
    // land on the value the row actually drew, not merely agree with itself.
    field.caret_home();
    let col = search_box_cursor_col(&field) as usize;
    assert_eq!(rows[1].chars().nth(col), Some('n'));
}

#[test]
fn search_box_clips_a_value_wider_than_its_frame() {
    let mut field = InputField::new("query", "", "");
    field.set_value("a very long free text query that overruns the box");
    let offset = std::cell::Cell::new(0);
    let mut terminal = Terminal::new(TestBackend::new(20, 3)).expect("test backend");
    terminal
        .draw(|frame| {
            let _ = render_list(
                frame,
                Rect::new(0, 0, 20, 3),
                vec![search_box_item(&field, false, false, 20)],
                Some(0),
                false,
                &offset,
            );
        })
        .expect("frame renders");
    // The frame owns the right edge: the value is cut with an ellipsis rather
    // than pushing the border off the row.
    assert_eq!(
        buffer_rows(terminal.backend().buffer())[1],
        "  │ a very long fr…│"
    );
}

#[test]
fn render_scrollbar_draws_in_right_padding_column() {
    // The bar lives in the panel's 1-cell right padding column at
    // `inner.x + inner.width`, never further right. With overflow content
    // (total > visible) a thumb glyph must appear in exactly that column.
    let mut terminal = Terminal::new(TestBackend::new(20, 10)).expect("test backend");
    let inner = Rect::new(2, 0, 10, 8); // padding column = x 12
    terminal
        .draw(|frame| render_scrollbar(frame, inner, 0, 40))
        .expect("frame renders");
    let buf = terminal.backend().buffer();
    let bar_col = inner.x + inner.width; // 12
    let drew_in_col =
        (inner.y..inner.y + inner.height).any(|y| matches!(buf[(bar_col, y)].symbol(), "┃" | "┊"));
    assert!(
        drew_in_col,
        "scrollbar must draw in the right padding column ({bar_col})"
    );
    // And nothing past it (the old `..inner` width pushed the bar off to the right).
    let drew_past = (bar_col + 1..20).any(|x| {
        (inner.y..inner.y + inner.height).any(|y| matches!(buf[(x, y)].symbol(), "┃" | "┊"))
    });
    assert!(
        !drew_past,
        "scrollbar must not draw past the padding column"
    );
}

#[test]
fn render_scrollbar_hidden_when_content_fits() {
    let mut terminal = Terminal::new(TestBackend::new(20, 10)).expect("test backend");
    let inner = Rect::new(2, 0, 10, 8);
    terminal
        .draw(|frame| render_scrollbar(frame, inner, 0, 8))
        .expect("frame renders");
    let buf = terminal.backend().buffer();
    let any_bar = buf
        .content()
        .iter()
        .any(|cell| matches!(cell.symbol(), "┃" | "┊"));
    assert!(!any_bar, "no scrollbar when total <= visible");
}

#[test]
fn render_scrollbar_thumb_sized_to_visible_ratio_and_reaches_bottom() {
    // 17 items, 14 visible: the thumb must cover most of the track (~visible/total)
    // and reach the bottom row at max scroll. Guards the `content_length =
    // max_offset + 1` setup — passing `total` undersized the thumb (≈half) and
    // parked it short of the end since our offset never reaches `total - 1`.
    let bar_col = 12u16; // inner.x + inner.width
    let inner = Rect::new(2, 0, 10, 14); // visible = 14
    let thumb_rows = |start: usize| -> Vec<u16> {
        let mut terminal = Terminal::new(TestBackend::new(20, 14)).expect("test backend");
        terminal
            .draw(|frame| render_scrollbar(frame, inner, start, 17))
            .expect("frame renders");
        let buf = terminal.backend().buffer();
        (inner.y..inner.y + inner.height)
            .filter(|&y| buf[(bar_col, y)].symbol() == "┃")
            .collect()
    };
    let top = thumb_rows(0);
    assert!(
        top.len() >= 10,
        "thumb should cover most of the track (~14/17), got {}",
        top.len()
    );
    assert_eq!(
        top.first().copied(),
        Some(0),
        "thumb anchored at top when scrolled to top"
    );
    let bottom = thumb_rows(3); // max offset = total - visible
    assert_eq!(
        bottom.last().copied(),
        Some(inner.height - 1),
        "thumb reaches the bottom row at max scroll"
    );
}

#[test]
fn truncate_to_width_handles_zero() {
    assert_eq!(truncate_to_width("hello", 0).0, "");
}

#[test]
fn truncate_to_width_one_returns_ellipsis() {
    assert_eq!(truncate_to_width("hello", 1).0, "…");
}

#[test]
fn truncate_to_width_unicode_safe() {
    // Each CJK char is display-width 2. Budget 4 → reserve 1 for "…" → 3 cols for chars.
    // "こ" = 2 cols fits; "こん" = 4 cols exceeds 3 → result is "こ…" (3 cols total).
    assert_eq!(truncate_to_width("こんにちは世界", 4).0, "こ…");
    // Budget 7 → 6 cols for chars → "こんに" (6 cols) fits → "こんに…" (7 cols total).
    assert_eq!(truncate_to_width("こんにちは世界", 7).0, "こんに…");
    // ASCII still works: budget 5 → "hell…"
    assert_eq!(truncate_to_width("hello world", 5).0, "hell…");
}

#[test]
fn truncate_to_width_reports_the_width_it_rendered() {
    use unicode_width::UnicodeWidthStr as _;
    // Callers pad by `budget - reported` to right-align what follows, so a
    // reported width the string does not actually occupy lands that suffix off
    // by a column. A cut landing mid double-width glyph is where the two diverge:
    // "\u{3053}\u{2026}" occupies 3 of a budget of 4.
    for (text, budget) in [
        (
            "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}\u{4e16}\u{754c}",
            4u16,
        ),
        (
            "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}\u{4e16}\u{754c}",
            7,
        ),
        ("hello world", 5),
        ("hello", 9),
    ] {
        let (out, reported) = truncate_to_width(text, budget);
        assert_eq!(
            reported as usize,
            out.width(),
            "{text:?} at budget {budget} reports what it rendered ({out:?})"
        );
    }
}

#[test]
fn message_style_rate_limited_overrides() {
    use ratatui::style::Style;
    assert_eq!(
        message_style(BeatmapStage::Success, true),
        Style::default().fg(warning())
    );
}

#[test]
fn message_style_stage_classification() {
    use ratatui::style::Style;
    assert_eq!(
        message_style(BeatmapStage::Success, false),
        Style::default().fg(success())
    );
    assert_eq!(
        message_style(BeatmapStage::Skipped, false),
        Style::default().fg(text_faint())
    );
    assert_eq!(
        message_style(BeatmapStage::Failed, false),
        Style::default().fg(danger())
    );
    assert_eq!(
        message_style(BeatmapStage::Aborted, false),
        Style::default().fg(danger())
    );
    assert_eq!(
        message_style(BeatmapStage::Downloading, false),
        Style::default().fg(text_dim())
    );
    assert_eq!(
        message_style(BeatmapStage::Pending, false),
        Style::default().fg(text_dim())
    );
    assert_eq!(
        message_style(BeatmapStage::Verifying, false),
        Style::default().fg(text_dim())
    );
}

/// The focused-row contract: `List::highlight_style` lays the edge-to-edge
/// `BG_HOVER` tint over the selected row, but ONLY the label span promotes to
/// `TEXT + bold`. The value (and any other span) keeps its own color/weight —
/// the selection must not recolor or embolden the whole line.
#[test]
fn focused_row_promotes_only_label_keeps_value_color_and_full_bg() {
    use ratatui::style::Modifier;

    // Two rows so row 0 is the focused/selected one and row 1 is a blurred sibling.
    // A non-empty value renders in ACCENT (an empty value would be TEXT_FAINT).
    let focused = input_item(&InputField::new("Threads", "abc", ""), true, false, 0);
    let blurred = input_item(&InputField::new("Connections", "xyz", ""), false, false, 0);

    let mut terminal = Terminal::new(TestBackend::new(40, 4)).expect("test backend");
    let inner = Rect::new(0, 0, 40, 4);
    terminal
        .draw(|frame| {
            let _ = render_list(
                frame,
                inner,
                vec![focused, blurred],
                Some(0),
                true,
                &std::cell::Cell::new(0),
            );
        })
        .expect("frame renders");
    let buf = terminal.backend().buffer();

    // Row layout: focus marker "❯ " (cols 0..2), label cell "threads  " (cols 2..),
    // then the value. Sample a label cell and the first value cell.
    let label_cell = &buf[(2, 0)]; // first label char ('t')
    assert_eq!(label_cell.symbol(), "t", "label cell sampled at col 2");
    assert_eq!(
        label_cell.fg,
        text(),
        "focused label promotes to TEXT (205,214,244)"
    );
    assert!(
        label_cell.modifier.contains(Modifier::BOLD),
        "focused label is bold"
    );

    // "threads" (7) + 2-space gap = 9 cols after the 2-col marker → value at col 11.
    let value_cell = &buf[(11, 0)];
    assert_eq!(value_cell.symbol(), "a", "value cell sampled at col 11");
    assert_eq!(
        value_cell.fg,
        accent(),
        "value keeps its own ACCENT color, not recolored to TEXT"
    );
    assert!(
        !value_cell.modifier.contains(Modifier::BOLD),
        "value is NOT emboldened by the selection"
    );

    // The BG_HOVER tint spans the focused row edge-to-edge (first and last cell).
    assert_eq!(
        buf[(0, 0)].bg,
        bg_hover(),
        "row bg is BG_HOVER at the left edge"
    );
    assert_eq!(
        buf[(39, 0)].bg,
        bg_hover(),
        "row bg is BG_HOVER at the right edge (edge-to-edge tint)"
    );

    // The blurred sibling carries neither the tint nor the bold label.
    assert_ne!(
        buf[(0, 1)].bg,
        bg_hover(),
        "blurred row has no BG_HOVER tint"
    );
    assert!(
        !buf[(2, 1)].modifier.contains(Modifier::BOLD),
        "blurred row label is not bold"
    );
}

/// Cloudy-tui contract: the focus caret `❯` is `ACCENT + bold`, and action
/// buttons split by prominence at rest — the primary CTA (`download`) stays
/// `ACCENT + bold`, while a secondary action (`view maps` / `scan`) drops to a
/// quieter `TEXT_DIM` so it doesn't shout as loud as the primary beside it. The
/// primary is the form's last enabled action button; a disabled pill is faint
/// regardless of prominence.
#[test]
fn action_button_prominence_and_caret_weight() {
    let render = |item: ListItem<'static>| {
        let mut terminal = Terminal::new(TestBackend::new(30, 1)).expect("test backend");
        terminal
            .draw(|frame| {
                let _ = render_list(
                    frame,
                    Rect::new(0, 0, 30, 1),
                    vec![item],
                    None,
                    false,
                    &std::cell::Cell::new(0),
                );
            })
            .expect("frame renders");
        terminal.backend().buffer().clone()
    };

    // Blurred (button focused = false): cols 0..2 are the blank caret pad, col 2
    // the pill's leading space, col 3 the label's first char carrying the at-rest
    // pill style.
    let primary = render(button_item(
        "download",
        false,
        true,
        ButtonProminence::Primary,
    ));
    assert_eq!(primary[(3, 0)].symbol(), "d");
    assert_eq!(
        primary[(3, 0)].fg,
        accent(),
        "primary CTA fill stays ACCENT at rest"
    );
    assert!(
        primary[(3, 0)].modifier.contains(Modifier::BOLD),
        "primary CTA stays bold at rest"
    );

    let view = render(button_item_with_loading_cue(
        "view maps",
        false,
        true,
        false,
        0,
        ButtonProminence::Secondary,
    ));
    assert_eq!(view[(3, 0)].symbol(), "v");
    assert_eq!(
        view[(3, 0)].fg,
        text_dim(),
        "secondary view button drops to TEXT_DIM at rest"
    );
    assert!(
        !view[(3, 0)].modifier.contains(Modifier::BOLD),
        "secondary button is not bold at rest"
    );

    let scan = render(button_item(
        "scan",
        false,
        true,
        ButtonProminence::Secondary,
    ));
    assert_eq!(
        scan[(3, 0)].fg,
        text_dim(),
        "secondary scan button drops to TEXT_DIM at rest"
    );

    // Focused (button focused = true): the caret glyph at col 0 is ACCENT + bold.
    let focused = render(button_item(
        "download",
        true,
        true,
        ButtonProminence::Primary,
    ));
    assert_eq!(focused[(0, 0)].symbol(), "❯");
    assert_eq!(focused[(0, 0)].fg, accent(), "focus caret is ACCENT");
    assert!(
        focused[(0, 0)].modifier.contains(Modifier::BOLD),
        "focus caret is bold (cloudy-tui hierarchy)"
    );
}

/// When the viewport grows (menu closes / terminal resizes taller) while
/// scrolled down with a selection near the bottom, the offset is pulled up to
/// the last full page so the viewport refills instead of leaving blank rows.
///
/// ratatui alone only keeps the *selected* row visible: with a stale offset of 5
/// in a now-8-row viewport it would render rows 5..10 (three blank rows below).
/// The `render_list` clamp snaps the offset to `max_offset` (10 − 8 = 2).
#[test]
fn stale_offset_pulls_up_to_fill_viewport() {
    use ratatui::widgets::ListItem;

    let items: Vec<ListItem<'static>> = ["Arow", "Brow", "Crow", "Drow", "Erow", "Frow", "Grow"]
        .into_iter()
        .chain(["Hrow", "Irow", "Jrow"])
        .map(ListItem::new)
        .collect();
    let offset = std::cell::Cell::new(5);
    let inner = Rect::new(0, 0, 10, 8);
    let mut terminal = Terminal::new(TestBackend::new(10, 8)).expect("test backend");
    terminal
        .draw(|frame| {
            // Selection on the 8th row (index 7) — visible at the stale offset, so
            // ratatui would not move it; only the clamp pulls the page up.
            let _ = render_list(frame, inner, items, Some(7), false, &offset);
        })
        .expect("frame renders");

    assert_eq!(
        offset.get(),
        2,
        "offset clamps to the last full page (10 - 8)"
    );
    // Items C..J fill all 8 rows, in order, with nothing blank below. A whole-buffer
    // pin works here only because these `ListItem`s are unstyled and `highlight:
    // false` leaves the selected row's style untouched — a real surface carries the
    // palette on every cell and can't be expressed as plain lines.
    terminal.backend().assert_buffer_lines([
        "Crow      ",
        "Drow      ",
        "Erow      ",
        "Frow      ",
        "Grow      ",
        "Hrow      ",
        "Irow      ",
        "Jrow      ",
    ]);
}

#[test]
fn glyph_fill_zero_is_empty() {
    assert_eq!(glyph_fill(&FILL_BLOCK, GLYPH_BLOCK, 0).as_ref(), "");
    assert_eq!(glyph_fill(&FILL_SHADE, GLYPH_SHADE, 0).as_ref(), "");
    assert_eq!(glyph_fill(&FILL_SPACE, GLYPH_SPACE, 0).as_ref(), "");
}

#[test]
fn glyph_fill_matches_repeat_for_all_glyphs() {
    for n in [1, 4, 12, 80, 160, 220, 256] {
        assert_eq!(
            glyph_fill(&FILL_BLOCK, GLYPH_BLOCK, n).as_ref(),
            GLYPH_BLOCK.repeat(n),
            "BLOCK n={n}"
        );
        assert_eq!(
            glyph_fill(&FILL_SHADE, GLYPH_SHADE, n).as_ref(),
            GLYPH_SHADE.repeat(n),
            "SHADE n={n}"
        );
        assert_eq!(
            glyph_fill(&FILL_SPACE, GLYPH_SPACE, n).as_ref(),
            GLYPH_SPACE.repeat(n),
            "SPACE n={n}"
        );
    }
}

#[test]
fn glyph_fill_fallback_above_max_width() {
    let n = 257;
    assert_eq!(
        glyph_fill(&FILL_BLOCK, GLYPH_BLOCK, n).as_ref(),
        GLYPH_BLOCK.repeat(n)
    );
    assert_eq!(
        glyph_fill(&FILL_SHADE, GLYPH_SHADE, n).as_ref(),
        GLYPH_SHADE.repeat(n)
    );
}

#[test]
fn download_size_label_zero_known_is_plain() {
    // Nothing probed yet → the bare `download (N)`, no `~` suffix.
    assert_eq!(download_button_label_with_size(3, 0), "download (3)");
}

#[test]
fn download_size_label_none_selected_drops_the_count() {
    // No picks → the bare `download`, matching `download_button_label`. Whether
    // the button is pressable is not this helper's to say (`button_enabled`).
    assert_eq!(download_button_label_with_size(0, 0), "download");
}

#[test]
fn download_size_label_renders_mib_below_a_gib() {
    assert_eq!(
        download_button_label_with_size(2, 512 * 1024 * 1024),
        "download (2) · ~512.0 MiB"
    );
}

#[test]
fn download_size_label_renders_gib_at_the_boundary() {
    // Exactly 1 GiB crosses into GiB; one byte under stays MiB (the MB/GB edge).
    assert_eq!(
        download_button_label_with_size(5, 1024 * 1024 * 1024),
        "download (5) · ~1.00 GiB"
    );
    assert_eq!(
        download_button_label_with_size(5, 1024 * 1024 * 1024 - 1),
        "download (5) · ~1024.0 MiB"
    );
}

/// One distinguishable row per index, so a buffer comparison catches a window
/// slid by even one row.
fn numbered_row(i: usize) -> ListItem<'static> {
    ListItem::new(format!("r{i:04}"))
}

/// Renders `total` rows through [`render_list`] (ratatui resolves the offset
/// from the full item set) and returns `(resolved offset, buffer)`.
fn full_list_frame(
    total: usize,
    height: u16,
    selected: Option<usize>,
    seed: usize,
) -> (usize, ratatui::buffer::Buffer) {
    let offset = std::cell::Cell::new(seed);
    let inner = Rect::new(0, 0, 8, height);
    let mut terminal = Terminal::new(TestBackend::new(8, height)).expect("test backend");
    terminal
        .draw(|frame| {
            let items: Vec<ListItem<'static>> = (0..total).map(numbered_row).collect();
            let _ = render_list(frame, inner, items, selected, true, &offset);
        })
        .expect("frame renders");
    (offset.get(), terminal.backend().buffer().clone())
}

/// The same fixture through [`render_windowed_list`], which resolves the offset
/// itself and builds only the visible slice.
fn windowed_list_frame(
    total: usize,
    height: u16,
    selected: Option<usize>,
    seed: usize,
) -> (usize, ratatui::buffer::Buffer) {
    let offset = std::cell::Cell::new(seed);
    let inner = Rect::new(0, 0, 8, height);
    let mut terminal = Terminal::new(TestBackend::new(8, height)).expect("test backend");
    terminal
        .draw(|frame| {
            let rows: ListRows<'_> = Box::new(|window| window.map(numbered_row).collect());
            let _ = render_windowed_list(frame, inner, total, &rows, selected, true, &offset);
        })
        .expect("frame renders");
    (offset.get(), terminal.backend().buffer().clone())
}

/// The windowed list resolves the scroll offset itself instead of handing
/// ratatui every row, so its rule has to stay ratatui's rule. Both paths run the
/// same fixture; a divergence in either the offset or a single rendered cell is
/// a scroll glitch nobody would trace back to a ratatui bump.
#[test]
fn windowed_list_matches_the_full_list() {
    // Heights spanning under, at, and over twice the scroll padding (3); counts
    // spanning shorter-than-viewport, exactly-viewport, and far longer; seeds
    // covering a fresh list, a mid-list page, and a stale over-scrolled offset.
    for total in [0usize, 1, 4, 7, 8, 9, 40, 500] {
        for height in [1u16, 4, 7, 8, 20] {
            for seed in [0usize, 3, 20, 499] {
                // Every row for the short lists; for the long one the two ends,
                // the padding band around each seed, and the middle — plus the
                // no-cursor and past-the-end cases either way.
                let sampled: Vec<usize> = if total <= 40 {
                    (0..total).collect()
                } else {
                    (0..10)
                        .chain(total / 2 - 1..total / 2 + 2)
                        .chain(seed.saturating_sub(4)..(seed + 5).min(total))
                        .chain(total - 10..total)
                        .collect()
                };
                let cursors =
                    sampled
                        .into_iter()
                        .map(Some)
                        .chain([None, Some(total), Some(total + 5)]);
                for selected in cursors {
                    let (want_offset, want_buffer) = full_list_frame(total, height, selected, seed);
                    let (got_offset, got_buffer) =
                        windowed_list_frame(total, height, selected, seed);
                    assert_eq!(
                        got_offset, want_offset,
                        "offset diverged at total={total} height={height} \
                         seed={seed} selected={selected:?}"
                    );
                    assert_eq!(
                        got_buffer, want_buffer,
                        "render diverged at total={total} height={height} \
                         seed={seed} selected={selected:?}"
                    );
                }
            }
        }
    }
}

// ── cycle_item chip wrapping ─────────────────────────────────────────────────

/// The real `categories` chip set: labels of wildly different widths, two of
/// them carrying an internal space. A fixture of same-width, space-free chips
/// would hold constant the very dimension these tests measure.
const CHIPS: &[&str] = &[
    "any",
    "has leaderboard",
    "ranked",
    "approved",
    "qualified",
    "loved",
    "pending",
    "wip",
    "graveyard",
    "unranked",
];

/// The find form's own label geometry: `LABEL_WIDTH` is `"favourites".len()`.
const CHIP_LABEL: &str = "categories";
const CHIP_LABEL_WIDTH: usize = 10;

/// Renders one [`cycle_item`] into a `width`-wide viewport and returns its rows,
/// trailing blanks trimmed. `viewport` is the row budget the list gets, kept
/// generous so a wrap is never mistaken for a viewport clip.
fn chip_rows(width: u16, focused: bool) -> Vec<String> {
    draw_rows(
        cycle_item(
            CHIP_LABEL,
            CHIPS,
            "has leaderboard",
            focused,
            CHIP_LABEL_WIDTH,
            width,
        ),
        width,
        focused,
    )
}

/// Draws one form row into a `width`-wide viewport and returns its lines,
/// trailing blanks trimmed. The viewport's row budget is kept generous so a wrap
/// is never mistaken for a clip.
fn draw_rows(item: ListItem<'static>, width: u16, focused: bool) -> Vec<String> {
    let inner = Rect::new(0, 0, width.max(1), CHIP_VIEWPORT);
    let mut terminal =
        Terminal::new(TestBackend::new(width.max(1), CHIP_VIEWPORT)).expect("test backend");
    terminal
        .draw(|frame| {
            render_list(
                frame,
                inner,
                vec![item],
                focused.then_some(0),
                focused,
                &std::cell::Cell::new(0),
            );
        })
        .expect("frame renders");
    let mut rows = buffer_rows(terminal.backend().buffer());
    while rows.last().is_some_and(|row| row.trim().is_empty()) {
        rows.pop();
    }
    rows.iter().map(|row| row.trim_end().to_string()).collect()
}

/// Every chip that appears in the render, in order, split on the two-space chip
/// gap. A chip cut across a line break shows up here as a fragment.
fn chips_in(rows: &[String]) -> Vec<String> {
    rows.iter()
        .flat_map(|row| {
            let value: String = row.chars().skip(CHIP_INDENT).collect();
            value
                .split("  ")
                .filter(|part| !part.is_empty())
                .map(|part| part.trim_matches(['[', ']']).to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The narrowest width that still holds the focused row on one line: prefix
/// (2 focus + 12 label cell) + every chip + a 2-cell gap between each, with the
/// selected chip bracketed.
const CHIP_ONE_LINE_WIDTH: u16 = 107;
/// Column the value (and every continuation line) starts at: 2-cell focus
/// marker + the padded `categories` label cell.
const CHIP_INDENT: usize = 14;
/// Rows the fixture's list gets. Tall enough that even one-chip-per-line at the
/// narrowest swept width fits whole — ratatui drops an item that overruns the
/// viewport rather than clipping it, which would read as a lost chip.
const CHIP_VIEWPORT: u16 = 24;

#[test]
fn a_chip_row_with_room_to_spare_stays_one_line() {
    // Byte-identical to the pre-wrap render: one line, chips separated by two
    // spaces, nothing indented.
    assert_eq!(
        chip_rows(CHIP_ONE_LINE_WIDTH + 20, true),
        vec![
            "\u{276f} categories  any  [has leaderboard]  ranked  approved  qualified  loved  pending  wip  graveyard  unranked"
        ]
    );
}

#[test]
fn a_chip_row_wraps_to_the_value_column_when_it_overflows() {
    let rows = chip_rows(64, true);
    assert_eq!(
        rows,
        vec![
            "\u{276f} categories  any  [has leaderboard]  ranked  approved",
            "              qualified  loved  pending  wip  graveyard",
            "              unranked",
        ]
    );
    // Continuation lines start at the value column, so the chips column-align.
    for row in &rows[1..] {
        assert!(
            row.starts_with(&" ".repeat(CHIP_INDENT)),
            "continuation line is not indented to the value column: {row:?}"
        );
    }
}

#[test]
fn wrapping_never_splits_a_chip() {
    // Floor: the narrowest width at which the widest chip fits after the indent
    // at all. Below it the TERMINAL truncates the cell row and no layout can
    // help, which is exactly what it did before wrapping existed.
    let floor = CHIP_INDENT
        + 2 // the selected chip's [brackets]
        + CHIPS.iter().map(|c| c.len()).max().expect("chips are non-empty");
    // Sweep every width from there up past the one-line fit: at no width may a
    // chip label be cut, and none may go missing.
    for width in floor as u16..=CHIP_ONE_LINE_WIDTH + 4 {
        let rows = chip_rows(width, true);
        assert_eq!(
            chips_in(&rows),
            CHIPS.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
            "chip set differs at width {width}: {rows:?}"
        );
    }
}

#[test]
fn the_one_line_boundary_is_exact() {
    assert_eq!(
        chip_rows(CHIP_ONE_LINE_WIDTH, true).len(),
        1,
        "the narrowest fitting width must stay one line"
    );
    assert_eq!(
        chip_rows(CHIP_ONE_LINE_WIDTH - 1, true),
        vec![
            "\u{276f} categories  any  [has leaderboard]  ranked  approved  qualified  loved  pending  wip  graveyard",
            "              unranked",
        ],
        "one column narrower spills exactly the last chip"
    );
}

// ── multi_chip_item wrapping ─────────────────────────────────────────────────

/// The real `rank` row — the widest multi-select the form has, and the one whose
/// pick marks add the most cells relative to its chip widths (`[x]XH` is three
/// cells of mark on two of label). A two-chip `extra` fixture would never wrap.
const RANK_CHIPS: &[&str] = &["XH", "X", "SH", "S", "A", "B", "C", "D"];
const RANK_LABEL: &str = "rank";

/// Renders the `rank` row at `width` with chips 0 and 2 picked and the cursor on
/// chip 1, so a wrap has to carry a picked chip, an unpicked one, and the
/// cursor's caret slot across the break. `descended` puts the row in edit mode,
/// where every chip reserves that slot.
fn rank_rows(width: u16, descended: bool) -> Vec<String> {
    render_rank(1, descended, width, width)
}

/// The same row with the cursor anywhere, laid out against `layout_width` and
/// drawn into a `draw_width`-wide viewport — a `layout_width` of 0 asks for one
/// line whatever its length, which is how the row's own width gets measured.
fn render_rank(cursor: usize, descended: bool, layout_width: u16, draw_width: u16) -> Vec<String> {
    draw_rows(
        multi_chip_item(
            RANK_LABEL,
            RANK_CHIPS,
            |idx| idx == 0 || idx == 2,
            cursor,
            true,
            descended,
            CHIP_LABEL_WIDTH,
            layout_width,
        ),
        draw_width,
        true,
    )
}

/// The marks widen every chip by three cells, so the wrap math has to count
/// them — and it counts spans now, not one string. Pinned as the exact render
/// rather than a property, since this is the row the chip states are read off.
#[test]
fn a_marked_chip_row_wraps_between_chips() {
    assert_eq!(
        rank_rows(70, false),
        vec!["\u{276f} rank        [x]XH  [ ]X  [x]SH  [ ]S  [ ]A  [ ]B  [ ]C  [ ]D"],
        "with room to spare it stays one line"
    );
    // 40 cells: 14 of indent leaves 26, and four marked chips with their gaps
    // are 24 of it — a fifth would overrun, so the break falls there.
    let rows = rank_rows(40, false);
    assert_eq!(
        rows,
        vec![
            "\u{276f} rank        [x]XH  [ ]X  [x]SH  [ ]S",
            "              [ ]A  [ ]B  [ ]C  [ ]D",
        ]
    );
    for row in &rows[1..] {
        assert!(
            row.starts_with(&" ".repeat(CHIP_INDENT)),
            "continuation line is not indented to the value column: {row:?}"
        );
    }
}

/// Descending widens the row exactly once: the label cell gives up a cell to the
/// first chip's caret slot (so the value column holds) and every later chip gains
/// one (so the gaps go 2 → 3). A continuation line indents to the same narrowed
/// column, landing its chip's own text back on the value column.
#[test]
fn a_descended_row_reserves_a_caret_slot_on_every_chip() {
    assert_eq!(
        rank_rows(70, true),
        vec!["\u{270e} rank        [x]XH  \u{276f}[ ]X   [x]SH   [ ]S   [ ]A   [ ]B   [ ]C   [ ]D"],
        "one slot per chip, filled only under the cursor"
    );
    let rows = rank_rows(40, true);
    assert_eq!(
        rows,
        vec![
            "\u{270e} rank        [x]XH  \u{276f}[ ]X   [x]SH",
            "              [ ]S   [ ]A   [ ]B   [ ]C",
            "              [ ]D",
        ]
    );
    for row in &rows {
        assert_eq!(
            mark_column(row),
            CHIP_INDENT,
            "a wrapped chip's mark left the value column: {row:?}"
        );
    }
}

/// The column every line's first mark sits at, whatever came before it. This is
/// what "indented to the value column" means once caret slots exist: the slot
/// belongs to its chip, so a line whose first chip holds the cursor opens one
/// cell further left and the MARK is what stays put.
fn mark_column(row: &str) -> usize {
    row.chars()
        .position(|c| c == '[')
        .unwrap_or_else(|| panic!("no chip mark in {row:?}"))
}

/// The fixture above holds the cursor at chip 1, which never lands first on a
/// wrapped line — so the column property it pins is only exercised on one of the
/// two shapes a line can have. Sweep the cursor across every chip at every
/// wrapping width, which is the dimension `the_row_width_is_the_same_under_every
/// _cursor_position` cannot reach (it never wraps).
#[test]
fn every_line_holds_the_value_column_at_every_cursor_and_width() {
    let floor = CHIP_INDENT + 4 + RANK_CHIPS.iter().map(|c| c.len()).max().expect("non-empty");
    for descended in [false, true] {
        for cursor in 0..RANK_CHIPS.len() {
            for width in floor as u16..=70 {
                for row in render_rank(cursor, descended, width, width) {
                    assert_eq!(
                        mark_column(&row),
                        CHIP_INDENT,
                        "cursor {cursor} at width {width} (descended: {descended}): {row:?}"
                    );
                }
            }
        }
    }
}

/// `descended` only means anything on a focused row, and the render folds focus
/// into it so a blurred row can never grow caret slots however a caller wires
/// the two. Unreachable from the find form — reachable through this public
/// widget, which is what makes the property total instead of caller-dependent.
#[test]
fn a_blurred_row_grows_no_slots_however_it_is_asked() {
    let asked_descended = draw_rows(
        multi_chip_item(
            RANK_LABEL,
            RANK_CHIPS,
            |idx| idx == 0 || idx == 2,
            1,
            false,
            true,
            CHIP_LABEL_WIDTH,
            0,
        ),
        90,
        false,
    );
    let at_rest = draw_rows(
        multi_chip_item(
            RANK_LABEL,
            RANK_CHIPS,
            |idx| idx == 0 || idx == 2,
            1,
            false,
            false,
            CHIP_LABEL_WIDTH,
            0,
        ),
        90,
        false,
    );
    assert_eq!(
        asked_descended,
        vec!["  rank        [x]XH  [ ]X  [x]SH  [ ]S  [ ]A  [ ]B  [ ]C  [ ]D"],
        "a blurred row keeps the pad gutter and the undescended geometry"
    );
    assert_eq!(
        asked_descended, at_rest,
        "the two must be indistinguishable"
    );
}

/// Walking the cursor may not move a single cell of the row. The slots are what
/// buy that, and a row that only drew a slot under the cursor would still pass
/// every other assertion here.
#[test]
fn the_row_width_is_the_same_under_every_cursor_position() {
    let widths: Vec<usize> = (0..RANK_CHIPS.len())
        .map(|cursor| {
            let rows = render_rank(cursor, true, 0, 90);
            assert_eq!(rows.len(), 1, "the measurement row wrapped");
            rows[0].chars().count()
        })
        .collect();
    // 69, hand-counted off the diagram: 2 lead + 11 label cell + 8 chips whose
    // slot+mark+label runs total 43, with seven 2-cell gaps.
    assert_eq!(widths, vec![69; RANK_CHIPS.len()], "the row breathes");
}

/// A break must never fall between a chip's mark and its label: that would read
/// as a chip that isn't there. (A caret orphaned from its mark is invisible to
/// `marked_chips`, which reads `[`-anchored text — the exact-render pins in
/// [`a_descended_row_reserves_a_caret_slot_on_every_chip`] are what cover that.)
#[test]
fn a_marked_chip_never_splits_across_the_break() {
    let floor = CHIP_INDENT + 4 + RANK_CHIPS.iter().map(|c| c.len()).max().expect("non-empty");
    for descended in [false, true] {
        for width in floor as u16..=70 {
            let rows = rank_rows(width, descended);
            assert_eq!(
                marked_chips(&rows),
                [
                    "[x]XH", "[ ]X", "[x]SH", "[ ]S", "[ ]A", "[ ]B", "[ ]C", "[ ]D"
                ],
                "chip set differs at width {width} (descended: {descended}): {rows:?}"
            );
        }
    }
}

/// Every chip in the render, in order, read off its `[x]` / `[ ]` mark rather
/// than off the spacing — which the caret slots change. A chip cut across a
/// break loses its label here, and an orphaned mark reads as an empty one.
fn marked_chips(rows: &[String]) -> Vec<String> {
    rows.iter()
        .flat_map(|row| {
            row.match_indices('[')
                .map(|(at, _)| {
                    let rest = &row[at..];
                    let label: String = rest[3..]
                        .chars()
                        .take_while(|c| !c.is_whitespace())
                        .collect();
                    format!("{}{label}", &rest[..3])
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn a_zero_width_chip_row_never_wraps() {
    // No width to lay out against: the row renders on one line as it always did.
    let item = cycle_item(CHIP_LABEL, CHIPS, "any", false, CHIP_LABEL_WIDTH, 0);
    assert_eq!(item.height(), 1);
}

#[test]
fn the_focus_highlight_covers_every_line_of_a_wrapped_row() {
    let width = 64u16;
    let inner = Rect::new(0, 0, width, 6);
    let mut terminal = Terminal::new(TestBackend::new(width, 6)).expect("test backend");
    terminal
        .draw(|frame| {
            render_list(
                frame,
                inner,
                vec![cycle_item(
                    CHIP_LABEL,
                    CHIPS,
                    "has leaderboard",
                    true,
                    CHIP_LABEL_WIDTH,
                    width,
                )],
                Some(0),
                true,
                &std::cell::Cell::new(0),
            );
        })
        .expect("frame renders");
    let buf = terminal.backend().buffer().clone();
    // Three lines tall (pinned by `a_chip_row_wraps_to_the_value_column_when_it_overflows`);
    // the tint has to reach the last cell of each, not just the first line.
    for y in 0..3 {
        assert_eq!(buf[(0, y)].bg, bg_hover(), "left edge of wrapped row {y}");
        assert_eq!(
            buf[(width - 1, y)].bg,
            bg_hover(),
            "right edge of wrapped row {y}"
        );
    }
    assert_ne!(buf[(0, 3)].bg, bg_hover(), "the row below is untinted");
}
