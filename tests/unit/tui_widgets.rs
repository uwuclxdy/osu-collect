use super::{
    ButtonProminence, ListRows, button_item, button_item_with_loading_cue,
    download_button_label_with_size, input_cursor_col, input_item, message_style, render_list,
    render_scrollbar, render_windowed_list, set_panel_cursor, truncate_to_width,
};
use crate::app::InputField;
use crate::download::BeatmapStage;
use crate::tui::{
    FILL_BLOCK, FILL_SHADE, FILL_SPACE, GLYPH_BLOCK, GLYPH_SHADE, GLYPH_SPACE, accent, bg_hover,
    danger, glyph_fill, success, text, text_dim, text_faint, warning,
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect, style::Modifier, widgets::ListItem};

/// Drives [`set_panel_cursor`] through a real frame and reads back the terminal
/// caret. Returns `None` when the frame left the cursor hidden, or `Some((x, y))`
/// when it was positioned (ratatui applies the request after the buffer flush).
fn panel_cursor(
    inner: Rect,
    focused_index: usize,
    start: usize,
    end: usize,
    cursor_col: Option<u16>,
) -> Option<(u16, u16)> {
    let mut terminal = Terminal::new(TestBackend::new(64, 24)).expect("test backend");
    terminal
        .draw(|frame| set_panel_cursor(frame, inner, focused_index, start, end, cursor_col))
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

#[test]
fn panel_cursor_none_when_no_column() {
    let inner = Rect::new(2, 3, 40, 10);
    assert_eq!(panel_cursor(inner, 5, 0, 10, None), None);
}

#[test]
fn panel_cursor_none_when_row_scrolled_out() {
    let inner = Rect::new(2, 3, 40, 10);
    assert_eq!(panel_cursor(inner, 12, 0, 10, Some(4)), None);
}

#[test]
fn panel_cursor_maps_row_and_clamps_column() {
    let inner = Rect::new(2, 3, 10, 10); // x=2, width=10 → last col 11
    // focused row 4, window starts at 2 → visible row 2 → y = 3 + 2 = 5
    assert_eq!(panel_cursor(inner, 4, 2, 10, Some(5)), Some((7, 5)));
    // column past the edge clamps to inner.x + width - 1 = 11
    assert_eq!(panel_cursor(inner, 4, 2, 10, Some(99)), Some((11, 5)));
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
    assert_eq!(
        download_button_label_with_size(3, 0),
        ("download (3)".to_string(), true)
    );
}

#[test]
fn download_size_label_none_selected_stays_disabled_and_plain() {
    // No picks → disabled bare `download`, matching `download_button_label`.
    assert_eq!(
        download_button_label_with_size(0, 0),
        ("download".to_string(), false)
    );
}

#[test]
fn download_size_label_renders_mib_below_a_gib() {
    let (label, enabled) = download_button_label_with_size(2, 512 * 1024 * 1024);
    assert!(enabled);
    assert_eq!(label, "download (2) · ~512.0 MiB");
}

#[test]
fn download_size_label_renders_gib_at_the_boundary() {
    // Exactly 1 GiB crosses into GiB; one byte under stays MiB (the MB/GB edge).
    let (gib, _) = download_button_label_with_size(5, 1024 * 1024 * 1024);
    assert_eq!(gib, "download (5) · ~1.00 GiB");
    let (mib, _) = download_button_label_with_size(5, 1024 * 1024 * 1024 - 1);
    assert_eq!(mib, "download (5) · ~1024.0 MiB");
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
