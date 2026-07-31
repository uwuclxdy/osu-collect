use crate::{
    app::{ActiveDownloadLine, InputField},
    download::DownloadStage,
    utils::format_bytes,
};
use osu_downloader::search::BeatmapSetMeta;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, List, ListItem, ListState, Padding, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
};
use std::borrow::Cow;
use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Instant;

use super::theme::{Tier, stars_color, theme};
use super::{
    FILL_BLOCK, FILL_SHADE, FILL_SPACE, GLYPH_BLOCK, GLYPH_SHADE, GLYPH_SPACE, accent, accent_alt,
    bg, bg_hover, bg_raised, danger, focused_label, glyph_fill, info, line, line_strong,
    spinner_str, success, text_dim, text_faint, warning,
};

pub const FOCUS_MARK: &str = "❯ ";
/// Edit-mode glyph for a text-input row being actively edited.
pub const EDIT_MARK: &str = "✎ ";
pub const FOCUS_PAD: &str = "  ";
pub const EXPANDED: &str = "▼";
pub const COLLAPSED: &str = "▶";
pub const SEPARATOR: &str = "  ·  ";
/// Scrollbar track glyph (`LINE`) and thumb glyph (`TEXT_DIM`).
const SCROLLBAR_TRACK: &str = "┊";
const SCROLLBAR_THUMB: &str = "┃";
/// Rows of context ratatui keeps above/below the cursor while it scrolls.
/// Spacer/hint rows count as context, so a single padded row can read as blank;
/// three keeps real rows visible past the cursor.
const SCROLL_PADDING: usize = 3;

/// Selected-row highlight: the edge-to-edge `BG_HOVER` tint only.
/// Applied by [`render_list`] / [`render_scrollable_panel`] via
/// `List::highlight_style` over the `ListState`-selected row.
///
/// Deliberately carries **no** `fg`/bold: ratatui patches `highlight_style` onto
/// every cell of the selected row, so adding `TEXT + bold` here would recolor and
/// embolden the whole line (value, metadata, badges, icons). Only the label span
/// promotes to `TEXT + bold`, baked at build time per row via [`focused_label`].
pub fn highlight_style() -> Style {
    Style::new().bg(bg_hover())
}

pub struct Metric<'a> {
    pub label: &'a str,
    pub value: String,
    pub style: Style,
}

impl<'a> Metric<'a> {
    pub fn muted(label: &'a str, value: impl Into<String>) -> Self {
        Self::colored(label, value, text_dim())
    }

    pub fn colored(label: &'a str, value: impl Into<String>, color: Color) -> Self {
        Self {
            label,
            value: value.into(),
            style: Style::default().fg(color),
        }
    }
}

pub struct FormItems<T> {
    items: Vec<ListItem<'static>>,
    focus: T,
    focused_index: usize,
}

impl<T: Copy + PartialEq> FormItems<T> {
    pub fn new(focus: T) -> Self {
        Self {
            items: Vec::new(),
            focus,
            focused_index: 0,
        }
    }

    pub fn push(&mut self, item: ListItem<'static>) {
        self.items.push(item);
    }

    pub fn push_focusable(&mut self, field: T, item: ListItem<'static>) {
        if self.focus == field {
            self.focused_index = self.items.len();
        }
        self.items.push(item);
    }

    pub fn into_parts(self) -> (Vec<ListItem<'static>>, usize) {
        (self.items, self.focused_index)
    }
}

/// Row heights of a form list's items, in item order.
///
/// ratatui sizes each list item by [`ListItem::height`], so an item index stops
/// being a row offset the moment one row is taller than a line (the boxed search
/// input). Every index→row conversion in this module — the scroll clamp, the
/// visible window, the caret row — reads this map, so they cannot drift apart.
/// With every item one line tall each method reduces to the plain arithmetic it
/// replaced, so a form of single-line rows renders and parks its caret exactly
/// as before (`one_line_form_renders_and_parks_the_caret_unchanged`).
pub struct ItemHeights(Vec<usize>);

impl ItemHeights {
    pub fn of(items: &[ListItem<'_>]) -> Self {
        Self(items.iter().map(ListItem::height).collect())
    }

    /// Top offset that leaves the last item flush with the viewport's bottom
    /// edge: the smallest index whose tail fits in `visible` rows.
    fn last_page_offset(&self, visible: usize) -> usize {
        let mut used = 0usize;
        let mut first = self.0.len();
        for (index, height) in self.0.iter().enumerate().rev() {
            used += height;
            if used > visible {
                break;
            }
            first = index;
        }
        first
    }

    /// One past the last item fully inside a `visible`-row viewport topped at
    /// item `start`. An item that would straddle the bottom edge is excluded,
    /// matching `List::get_items_bounds`, which stops at the first item whose
    /// height overruns the remaining space rather than clipping it.
    fn visible_end(&self, start: usize, visible: usize) -> usize {
        let mut used = 0usize;
        let mut end = start.min(self.0.len());
        for height in &self.0[end..] {
            if used + height > visible {
                break;
            }
            used += height;
            end += 1;
        }
        end
    }

    /// Rows between the viewport's top edge (item `start`) and item `index`.
    fn rows_before(&self, start: usize, index: usize) -> usize {
        let len = self.0.len();
        self.0[start.min(len)..index.min(len).max(start.min(len))]
            .iter()
            .sum()
    }

    /// The row within item `index` that the text caret parks on: the item's
    /// vertical middle. Ceiling: the only taller-than-one row is the 3-row
    /// bordered search box, whose text line IS its middle row. An item with two
    /// content rows would need its caret row carried alongside the column.
    fn caret_row(&self, index: usize) -> usize {
        self.0.get(index).copied().unwrap_or(1).saturating_sub(1) / 2
    }
}

/// Renders an item list into `inner` with `ListState`-driven scrolling and,
/// when `highlight` is set, the [`highlight_style`] on the focused
/// row, then draws the overflow [`render_scrollbar`] in the panel's right
/// padding column.
///
/// `focused` is the row to scroll into view; `None` is a cursorless list (a
/// read-only preview), which renders the seeded offset as given so a scroll
/// nothing selects still holds. The scroll target is decoupled from the
/// highlight: when `highlight` is `false` (the focused row styles itself — the
/// CTA button, the auth chip) the row is still scrolled into view but the
/// `bg_hover` bar is suppressed so the row's own styling shows through.
/// `List::scroll_padding(3)` keeps three rows of context above/below while it
/// scrolls (spacer/hint rows count as context, so a single padded row can read
/// as blank; three keeps real rows visible past the cursor).
///
/// Returns the resolved top offset (`ListState::offset`) so the caller can map
/// the focused row to a screen position for the text caret.
pub(crate) fn render_list(
    frame: &mut Frame,
    inner: Rect,
    items: Vec<ListItem<'static>>,
    focused: Option<usize>,
    highlight: bool,
    offset: &Cell<usize>,
) -> usize {
    let total = items.len();
    let heights = ItemHeights::of(&items);
    // Persist the scroll offset across frames: seed the list with the previous
    // offset so ratatui only scrolls when the focused row falls outside the
    // viewport. A fresh `ListState::default()` each frame re-pins the selection
    // to the panel's bottom edge on every redraw (offset resets to 0, then
    // `select` scrolls the minimum to reveal it — always the bottom once past
    // the first page).
    //
    // Clamp the seed to the last full page: ratatui only scrolls to keep the
    // focused row visible, it never pulls the offset back up when the viewport
    // grows or the content shrinks (menu closes, terminal resizes taller), so a
    // stale large offset leaves blank rows below the last item. Clamping refills
    // the viewport from the bottom the moment the space appears.
    let max_offset = heights.last_page_offset(inner.height as usize);
    let mut state = ListState::default().with_offset(offset.get().min(max_offset));
    // Always scroll the focused row into view; only the highlight bar is gated.
    // `ListState::select(None)` zeroes the offset, so a cursorless list keeps its
    // seed by leaving the (already `None`) selection alone.
    if focused.is_some() {
        state.select(focused);
    }
    // A self-styling focused row (CTA / auth chip) keeps its own styling by
    // rendering a neutral highlight that leaves the row's spans untouched.
    let row_style = if highlight {
        highlight_style()
    } else {
        Style::default()
    };
    let list = List::new(items)
        .scroll_padding(SCROLL_PADDING)
        .highlight_symbol("")
        .highlight_style(row_style);
    frame.render_stateful_widget(list, inner, &mut state);
    let resolved = state.offset();
    offset.set(resolved);
    render_scrollbar(frame, inner, resolved, total);
    resolved
}

/// Builds the `start..end` slice of a windowed list's rows. Called once per
/// frame with the resolved viewport, so a browse of thousands of rows pays for
/// the visible handful.
pub(crate) type ListRows<'a> = Box<dyn Fn(std::ops::Range<usize>) -> Vec<ListItem<'static>> + 'a>;

/// [`render_list`]'s scrolling contract over a list whose rows are too costly to
/// build in full: resolves the offset from the row COUNT, then builds and
/// renders only the visible window.
///
/// Every row must be exactly one line high — that is what makes the offset
/// computable without the items (see [`resolve_list_offset`]). Returns the
/// resolved top offset, same as [`render_list`].
pub(crate) fn render_windowed_list(
    frame: &mut Frame,
    inner: Rect,
    total: usize,
    rows: &ListRows<'_>,
    focused: Option<usize>,
    highlight: bool,
    offset: &Cell<usize>,
) -> usize {
    let visible = inner.height as usize;
    // Clamped for the same reason `render_list` clamps: ratatui never pulls a
    // stale large offset back up when the viewport grows or the content shrinks.
    let max_offset = total.saturating_sub(visible);
    // ratatui pulls an out-of-range cursor back onto the last row rather than
    // dropping the highlight, so the window's own mapping has to do it too.
    let focused = focused.map(|row| row.min(total.saturating_sub(1)));
    let seed = offset.get().min(max_offset);
    let start = resolve_list_offset(total, visible, focused, seed);
    let end = (start + visible).min(total);
    // A window of at most `visible` one-line rows seeded at offset 0 leaves
    // ratatui nothing to scroll, so it renders the slice as given.
    let mut state = ListState::default();
    state.select(
        focused
            .filter(|row| (start..end).contains(row))
            .map(|row| row - start),
    );
    let row_style = if highlight {
        highlight_style()
    } else {
        Style::default()
    };
    let list = List::new(rows(start..end))
        .scroll_padding(SCROLL_PADDING)
        .highlight_symbol("")
        .highlight_style(row_style);
    frame.render_stateful_widget(list, inner, &mut state);
    offset.set(start);
    render_scrollbar(frame, inner, start, total);
    start
}

/// The top offset ratatui's [`List`] resolves for `total` one-line rows in a
/// `visible`-row viewport, given the previous frame's `offset` and the cursor.
///
/// Transcribed from `List`'s own `get_items_bounds` +
/// `apply_scroll_padding_to_selected_index` with every item height fixed at 1,
/// which is what lets it run without the items existing. The
/// `windowed_list_matches_the_full_list` differential test drives both paths
/// over one fixture, so a divergence from ratatui fails the gate rather than
/// showing up as a scroll glitch.
fn resolve_list_offset(
    total: usize,
    visible: usize,
    selected: Option<usize>,
    offset: usize,
) -> usize {
    if total == 0 || visible == 0 {
        return 0;
    }
    let last_valid = total - 1;
    let mut first = offset.min(last_valid);
    // Exclusive, matching ratatui's `last_visible_index`.
    let mut last = (first + visible).min(total);
    let mut height = last - first;

    let target = match selected {
        None => first,
        Some(selected) => {
            let selected = selected.min(last_valid);
            // Padding shrinks until the band around the cursor fits the viewport.
            let mut padding = SCROLL_PADDING;
            while padding > 0 {
                let lo = selected.saturating_sub(padding);
                let hi = selected.saturating_add(padding).min(last_valid);
                if hi - lo < visible {
                    break;
                }
                padding -= 1;
            }
            if selected.saturating_add(padding).min(last_valid) >= last {
                selected + padding
            } else if selected.saturating_sub(padding) < first {
                selected.saturating_sub(padding)
            } else {
                selected
            }
            .min(last_valid)
        }
    };

    while target >= last {
        height += 1;
        last += 1;
        while height > visible {
            height -= 1;
            first += 1;
        }
    }
    // Scrolling back up drops rows off the BOTTOM to stay within the viewport;
    // only `first` is returned, so ratatui's matching `last` decrement is dropped.
    while target < first {
        first -= 1;
        height += 1;
        while height > visible {
            height -= 1;
        }
    }
    first
}

/// Draw a scrollbar in a padded panel's right padding column.
///
/// Scrollbar: the bar lives in the panel's 1-cell right padding
/// column (`inner.x + inner.width`) so it never eats a content cell — content
/// width is unchanged whether the bar shows or not. Track `┊` (`LINE`), thumb
/// `┃` (`TEXT_DIM`), no begin/end arrows. Draws nothing when the content fits
/// (`total <= visible`). `start` is the scroll offset (top item index); `total`
/// is the item count.
pub(crate) fn render_scrollbar(frame: &mut Frame, inner: Rect, start: usize, total: usize) {
    let visible = inner.height as usize;
    if visible == 0 || inner.width == 0 || total <= visible {
        return;
    }
    // Single-column track at the right padding cell. `Scrollbar` (VerticalRight)
    // renders in the last column of its area, so the area must be exactly one
    // cell wide here — `..inner` would copy `inner.width` and push the bar far
    // off to the right (off-screen).
    let track = Rect {
        x: inner.x + inner.width,
        width: 1,
        ..inner
    };
    // ratatui sizes the thumb as viewport·track / ((content_length-1)+viewport)
    // and expects `position` to reach content_length-1. Our offset is clamped to
    // the last full page (0..=total-visible, no over-scroll), so content_length
    // must be the offset count (max_offset+1) — passing `total` undersizes the
    // thumb and parks it short of the bottom at max scroll.
    let max_offset = total - visible; // total > visible guaranteed above
    let mut state = ScrollbarState::new(max_offset + 1)
        .viewport_content_length(visible)
        .position(start.min(max_offset));
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(Some(SCROLLBAR_TRACK))
        .thumb_symbol(SCROLLBAR_THUMB)
        .track_style(Style::default().fg(line()))
        .thumb_style(Style::default().fg(text_dim()));
    frame.render_stateful_widget(scrollbar, track, &mut state);
}

/// Renders a scrollable form panel and positions the terminal caret on the
/// focused row when `cursor_col` is `Some` and that row is currently visible.
///
/// `cursor_col` is the column offset (within `inner`) of the caret on the
/// focused row — see [`input_cursor_col`]. When set and the row is on-screen,
/// the caret is placed via [`Frame::set_cursor_position`]; ratatui applies it
/// after the buffer flush, so a frame that never sets it leaves the cursor
/// hidden. `None` means no caret should be shown.
///
/// `focused`: this panel currently owns the keyboard cursor.
/// `first_panel`: this is the first bordered panel rendered on the screen body
/// (its title draws in `ACCENT_2`; subsequent panels use `TEXT_DIM`).
/// `highlight`: tint the focused row (`false` when the focused row styles itself
/// — the CTA button or the auth chip — so the row highlight never clobbers it).
#[allow(clippy::too_many_arguments)]
pub fn render_scrollable_panel(
    frame: &mut Frame,
    area: Rect,
    title: impl Into<Cow<'static, str>>,
    meta: Option<Line<'static>>,
    items: Vec<ListItem<'static>>,
    focused_index: usize,
    highlight: bool,
    cursor_col: Option<u16>,
    focused: bool,
    first_panel: bool,
    offset: &Cell<usize>,
) {
    let block = panel_block(title, meta, focused, first_panel);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let heights = ItemHeights::of(&items);
    let start = render_list(frame, inner, items, Some(focused_index), highlight, offset);

    set_panel_cursor(frame, inner, &heights, focused_index, start, cursor_col);
}

/// Content width of a [`panel_block`] drawn over `area` — the cells a form row
/// has to draw into. Read off the block itself so a border or padding change
/// can't leave a width-aware row (the boxed [`search_box_item`]) one cell off.
pub fn panel_content_width(area: Rect) -> u16 {
    panel_block("", None, false, false).inner(area).width
}

/// Column offset (within a panel's inner area) of the text caret for a focused
/// [`input_item`]: focus marker + padded label cell + the caret offset.
///
/// `label_width` is the group's shared label-column width (see [`label_cell`]);
/// the rendered label cell spans `max(label_width, label) + 2` cells, matching
/// [`input_item`].  The caret is a char index, so its column is the number of
/// chars to its left (`field.caret()`), not the full value width.
pub fn input_cursor_col(field: &InputField, label_width: usize) -> u16 {
    let label_len = field.label.to_lowercase().chars().count();
    let cell = label_width.max(label_len) + 2;
    // focus marker (2) + padded label cell + caret offset within the value
    (2 + cell + field.caret()) as u16
}

/// Positions the terminal caret for a focused row at `cursor_col` via
/// [`Frame::set_cursor_position`], or leaves it hidden when no caret is
/// requested or the row is scrolled out of view. The column is clamped to the
/// last cell of `inner` so a long value never parks the cursor past the panel
/// edge. ratatui applies the request after the buffer flush (no flash).
///
/// The row comes from `heights`, not from the item index: a multi-row item
/// above the focused one pushes it further down, and the focused item's own
/// caret sits on its middle row (see [`ItemHeights::caret_row`]).
///
/// `start` is the resolved top item (`ListState::offset`); the bottom edge is
/// derived from `heights` rather than passed in, so the caret cannot be placed
/// on a row the list did not draw.
pub fn set_panel_cursor(
    frame: &mut Frame,
    inner: Rect,
    heights: &ItemHeights,
    focused_index: usize,
    start: usize,
    cursor_col: Option<u16>,
) {
    let Some(col) = cursor_col else { return };
    let end = heights.visible_end(start, inner.height as usize);
    if inner.width == 0 || inner.height == 0 || focused_index < start || focused_index >= end {
        return;
    }
    let row = heights.rows_before(start, focused_index) + heights.caret_row(focused_index);
    let y = inner.y + row as u16;
    let max_x = inner.x + inner.width - 1;
    let x = (inner.x + col).min(max_x);
    frame.set_cursor_position((x, y));
}

/// Builds a bordered panel block.
///
/// `focused`: this panel currently owns the keyboard cursor — border renders
/// `LINE_STRONG`; a blurred or read-only panel uses `LINE`.
/// `first_panel`: the first bordered panel on the screen body gets its title in
/// `ACCENT_2` (orange); subsequent panels use `TEXT_DIM`.  Both always italic;
/// title is bold only while the panel is focused.
///
/// Callers pass an already-uppercased, space-padded title (e.g. `" OVERVIEW "`),
/// usually a module-level `PANEL_*` constant (`&'static str`, no allocation). A
/// dynamic proper-noun title — a master-detail preview named after the selected
/// item — passes an owned, original-case `String` (case preserved, per the
/// proper-noun title exception).
///
/// `meta`: an optional title-right meta line drawn in the border break just
/// before the top-right corner (`╭─ TITLE ─── meta ─╮`). Its spans carry their
/// own color (`TEXT_DIM` or a semantic — never bold/italic); the flanking `─`
/// cells keep the border token so chrome owns every dash.
pub fn panel_block(
    title: impl Into<Cow<'static, str>>,
    meta: Option<Line<'static>>,
    focused: bool,
    first_panel: bool,
) -> Block<'static> {
    let border_color = if focused { line_strong() } else { line() };
    let title_color = if first_panel {
        accent_alt()
    } else {
        text_dim()
    };
    let mut title_style = Style::default().fg(title_color).italic();
    if focused {
        title_style = title_style.bold();
    }
    let border_style = Style::default().fg(border_color);
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        // Tab title sits right after the rounded corner: `╭ TITLE ─`.
        .title(Line::from(vec![
            Span::styled(title.into(), title_style),
            Span::styled("─", border_style),
        ]))
        .padding(Padding::new(1, 1, 0, 0));
    if let Some(meta) = meta {
        // Right-aligned title is flush to the top-right corner; mirror the corner
        // dash on the trailing side so it reads ` meta ─╮` with a leading gap
        // separating it from the border fill.
        let mut spans = Vec::with_capacity(meta.spans.len() + 2);
        spans.push(Span::styled(" ", border_style));
        spans.extend(meta.spans);
        spans.push(Span::styled(" ─", border_style));
        block = block.title_top(Line::from(spans).right_aligned());
    }
    block
}

pub fn focus_span(focused: bool) -> Span<'static> {
    if focused {
        // Contract: the selection caret is ACCENT + bold (cloudy-tui hierarchy).
        FOCUS_MARK.fg(accent()).bold()
    } else {
        Span::raw(FOCUS_PAD)
    }
}

/// Checkbox marker spans for multi-select rows: `[x]` checked, `[ ]` unchecked.
///
/// Checkbox row: brackets in `TEXT_DIM`, the `x` in `ACCENT`.
/// Used by checkbox rows in the updates panel. For boolean toggle rows
/// (`row_item` / `row_item_with_suffix`) use [`toggle_spans`] instead.
pub fn checkbox_spans(state: bool) -> Vec<Span<'static>> {
    let bracket = Style::default().fg(text_dim());
    let inner = if state {
        "x".fg(accent())
    } else {
        Span::styled(" ", bracket)
    };
    vec![
        Span::styled("[", bracket),
        inner,
        Span::styled("]", bracket),
    ]
}

/// Tier-aware slide-toggle spans for a boolean `row_item`.
///
/// Full tier: `─●` (on) / `○─` (off).
/// Compatible tier: `[on]` / `[off]`.
///
/// This is the glyph set for **toggle rows only** (boolean on/off).  Checkbox
/// rows (multi-select) continue to use [`check_marker`].
fn toggle_spans(on: bool) -> Vec<Span<'static>> {
    match theme().tier() {
        Tier::Full => {
            if on {
                vec!["─".fg(line()), "●".fg(accent())]
            } else {
                vec!["○".fg(text_dim()), "─".fg(line())]
            }
        }
        Tier::Compatible => {
            if on {
                vec!["[".fg(text_dim()), "on".fg(accent()), "]".fg(text_dim())]
            } else {
                vec!["[".fg(text_dim()), "off".fg(text_dim()), "]".fg(text_dim())]
            }
        }
    }
}

/// Leading glyph for a text-input row: `✎` when the row is being edited, `❯`
/// when selected-not-editing, two-space pad when blurred.
pub fn input_focus_span(focused: bool, editing: bool) -> Span<'static> {
    // Contract: the caret `❯` and edit glyph `✎` are both ACCENT + bold.
    if focused && editing {
        EDIT_MARK.fg(accent()).bold()
    } else if focused {
        FOCUS_MARK.fg(accent()).bold()
    } else {
        Span::raw(FOCUS_PAD)
    }
}

/// Pads a lowercase form-row label to the group's shared column width plus a
/// 2-space gap, so every value in the group stacks at the same column. No colon
/// (form rows take no colon). `label_width` is the widest label in the
/// group; pass `0` to fall back to the label's own width + 2 spaces.
pub fn label_cell(label: &str, label_width: usize) -> String {
    let width = label_width.max(label.chars().count());
    format!("{label:<width$}  ")
}

/// A text-input row. `editing` applies only when `focused` and drives the `✎`
/// glyph; the native caret is the caller's job (it requests `cursor_col` only
/// while editing — see each view's `render`).
///
/// `label_width` column-aligns the value with the group's other rows — see
/// [`label_cell`]; the caller passes the group's widest label width.
pub fn input_item(
    field: &InputField,
    focused: bool,
    editing: bool,
    label_width: usize,
) -> ListItem<'static> {
    let value = if field.value.is_empty() {
        field.placeholder.clone().fg(text_faint())
    } else {
        field.value.clone().fg(accent())
    };

    let spans = vec![
        input_focus_span(focused, editing),
        Span::styled(
            label_cell(&field.label.to_lowercase(), label_width),
            focused_label(focused),
        ),
        value,
    ];
    ListItem::new(Line::from(spans))
}

/// A text-input row whose value renders masked (`•`), for password fields.
///
/// Mirrors [`input_item`] exactly except the value glyphs are hidden. The
/// placeholder still shows while empty, and the caret column is unaffected:
/// the caret is a char index and the mask is one `•` per source char, so
/// [`input_cursor_col`] lands on the right cell.
pub fn password_input_item(
    field: &InputField,
    focused: bool,
    editing: bool,
    label_width: usize,
) -> ListItem<'static> {
    let value = if field.value.is_empty() {
        field.placeholder.clone().fg(text_faint())
    } else {
        "•".repeat(field.value.chars().count()).fg(accent())
    };

    let spans = vec![
        input_focus_span(focused, editing),
        Span::styled(
            label_cell(&field.label.to_lowercase(), label_width),
            focused_label(focused),
        ),
        value,
    ];
    ListItem::new(Line::from(spans))
}

/// Left inset of the boxed search input, matching [`FOCUS_PAD`] on every other
/// form row.
const SEARCH_BOX_INSET: usize = 2;
/// Cells the box's own chrome eats on its text row: the two border cells plus
/// the 2-cell focus glyph.
const SEARCH_BOX_CHROME: usize = 2 + FOCUS_PAD.len();
/// Column the value starts at, shared by [`search_box_item`] and
/// [`search_box_cursor_col`] so the caret can't drift off the text.
const SEARCH_BOX_VALUE_COL: usize = SEARCH_BOX_INSET + 1 + FOCUS_PAD.len();

/// A free-text input drawn as a 3-row bordered search box — top border, text,
/// bottom border — spanning `width` (the panel's [`panel_content_width`]) less
/// the left inset.
///
/// The box carries no label column: it IS the label, so the value starts at a
/// fixed [`SEARCH_BOX_VALUE_COL`] rather than at a group-aligned one. The focus
/// glyph sits inside the frame at the text row's left edge, so focus never rests
/// on the border colour alone.
///
/// The frame is hand-drawn rather than a [`Block`]: a `List` item is `Text`, so
/// there is no seam to hang a widget on inside a row. [`ItemHeights`] is what
/// keeps the surrounding index math honest about the extra rows.
pub fn search_box_item(
    field: &InputField,
    focused: bool,
    editing: bool,
    width: u16,
) -> ListItem<'static> {
    let box_width = (width as usize).saturating_sub(SEARCH_BOX_INSET);
    let rail = box_width.saturating_sub(2);
    let text_width = box_width.saturating_sub(SEARCH_BOX_CHROME);
    // Focus is the box's own state, so the frame takes ACCENT while it holds the
    // cursor and the recessive LINE otherwise.
    let border = Style::default().fg(if focused { accent() } else { line() });

    let (value, value_style) = if field.value.is_empty() {
        (field.placeholder.clone(), Style::default().fg(text_faint()))
    } else {
        (field.value.clone(), Style::default().fg(accent()))
    };
    // Clipped, not wrapped: the frame owns the right edge, and a value long
    // enough to reach it parks the caret on the last cell like any other row.
    let (value, used) = truncate_to_width(&value, text_width as u16);
    let pad = text_width.saturating_sub(used as usize);

    ListItem::new(Text::from(vec![
        Line::from(vec![
            Span::raw(FOCUS_PAD),
            Span::styled(format!("╭{}╮", "─".repeat(rail)), border),
        ]),
        Line::from(vec![
            Span::raw(FOCUS_PAD),
            Span::styled("│", border),
            input_focus_span(focused, editing),
            Span::styled(value, value_style),
            Span::raw(" ".repeat(pad)),
            Span::styled("│", border),
        ]),
        Line::from(vec![
            Span::raw(FOCUS_PAD),
            Span::styled(format!("╰{}╯", "─".repeat(rail)), border),
        ]),
    ]))
}

/// Caret column (within a panel's inner area) for a focused [`search_box_item`]:
/// the box's fixed value column plus the caret's char offset into the value.
pub fn search_box_cursor_col(field: &InputField) -> u16 {
    (SEARCH_BOX_VALUE_COL + field.caret()) as u16
}

/// A stepper row showing a numeric value with an optional "recommended N" chip.
///
/// `recommended` is shown as a dim chip when the current value differs; omitted
/// when `value == recommended` (the field is already at the suggested setting).
pub fn stepper_item(
    label: &str,
    value: u8,
    recommended: u8,
    focused: bool,
    label_width: usize,
) -> ListItem<'static> {
    let mut s = String::with_capacity(3);
    s.push_str(&value.to_string());
    let value_span = s.fg(accent());

    let mut spans = vec![
        focus_span(focused),
        Span::styled(label_cell(label, label_width), focused_label(focused)),
        value_span,
    ];

    if value != recommended {
        let mut chip = String::with_capacity(16);
        chip.push_str("  recommended ");
        chip.push_str(&recommended.to_string());
        spans.push(chip.fg(text_faint()));
    }

    ListItem::new(Line::from(spans))
}

/// Gap between two chips on a chip row. Load-bearing for the wrap math in
/// [`cycle_item`], not just decoration.
const CHIP_GAP: &str = "  ";

/// A chip row: the label cell, then every option with the selected one accented
/// (and `[bracketed]` while the row is focused).
///
/// `width` is the cells the row can draw into — the panel's
/// [`panel_content_width`]. A row whose chips overrun it continues on further
/// lines indented to the value column, so the chips stay aligned under each
/// other and the break always falls BETWEEN chips: a chip label can carry a
/// space (`has leaderboard`, `all ranked`), which a character-level clip cuts
/// mid-word. A row that fits stays a single line, byte for byte.
///
/// `width == 0` means the caller has no width to offer; the row then renders on
/// one line whatever its length, which is what every chip row did before.
pub fn cycle_item(
    label: &str,
    options: &[&str],
    selected: &str,
    focused: bool,
    label_width: usize,
    width: u16,
) -> ListItem<'static> {
    // A single-select row's cursor IS its selection, so both cues land on the
    // same chip — which is what makes it the degenerate case of [`chip_row`].
    chip_row(
        label,
        options,
        |idx| options[idx] == selected,
        |idx| options[idx] == selected,
        false,
        focused,
        label_width,
        width,
    )
}

/// Pick marks on a MULTI-select chip row. Which chips are on has to survive a
/// colourblind palette, a low-contrast theme, and a copy-pasted screen, so the
/// `ACCENT` fill cannot be the only channel carrying it. Same glyph pair the
/// Config toggle rows use rather than a new vocabulary; the prefix also tells a
/// multi row apart from a cycle row, which carries none.
const CHIP_PICKED: &str = "●";
const CHIP_UNPICKED: &str = "○";

/// A multi-select chip row: any number of `options` can be picked at once, so
/// the row carries its own `cursor` for `space`/`enter` to act on.
///
/// Same grammar as [`cycle_item`], which is the point — the two sit adjacent in
/// the find form and must read as one control family. Each chip states its own
/// pick state ([`CHIP_PICKED`] / [`CHIP_UNPICKED`]) with `ACCENT` reinforcing
/// it, and the `[brackets]` are the focus cue, tracking the cursor — which on
/// this row no longer coincides with the selection:
///
/// ```text
///   rank    ●XH  [●X]  ○SH  ○S  ○A  ○B  ○C  ○D
/// ```
pub fn multi_chip_item(
    label: &str,
    options: &[&str],
    picked: impl Fn(usize) -> bool,
    cursor: usize,
    focused: bool,
    label_width: usize,
    width: u16,
) -> ListItem<'static> {
    chip_row(
        label,
        options,
        picked,
        |idx| idx == cursor,
        true,
        focused,
        label_width,
        width,
    )
}

/// The shared chip-row body behind [`cycle_item`] and [`multi_chip_item`]:
/// label cell, then every option, wrapping between chips at `width`.
/// `picked` paints a chip `ACCENT`; `at_cursor` wraps it in `[brackets]` while
/// the row is focused. `marked` prefixes every chip with its own pick glyph —
/// the multi-select row, where `picked` is a per-chip answer nobody can infer
/// from the row's shape.
#[allow(clippy::too_many_arguments)]
fn chip_row(
    label: &str,
    options: &[&str],
    picked: impl Fn(usize) -> bool,
    at_cursor: impl Fn(usize) -> bool,
    marked: bool,
    focused: bool,
    label_width: usize,
    width: u16,
) -> ListItem<'static> {
    let lead = focus_span(focused);
    let cell = Span::styled(label_cell(label, label_width), focused_label(focused));
    // Continuation lines indent to the value column so the chips column-align.
    let indent = lead.width() + cell.width();
    let mut used = indent;
    let mut spans = vec![lead, cell];
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut first_on_line = true;

    for (idx, &option) in options.iter().enumerate() {
        let body = match (marked, picked(idx)) {
            (false, _) => option.to_string(),
            (true, true) => format!("{CHIP_PICKED}{option}"),
            (true, false) => format!("{CHIP_UNPICKED}{option}"),
        };
        // [brackets] only while the row is focused; ACCENT, no bold. They wrap
        // the mark too, so the cursor reads as sitting on the whole chip.
        let text = if focused && at_cursor(idx) {
            format!("[{body}]")
        } else {
            body
        };
        let chip = if picked(idx) {
            text.fg(accent())
        } else {
            text.fg(text_faint())
        };
        // A chip that overruns the width on its own still goes out whole, on a
        // line of its own: breaking inside one is never an option.
        if width > 0 && !first_on_line && used + CHIP_GAP.len() + chip.width() > width as usize {
            lines.push(Line::from(std::mem::take(&mut spans)));
            spans.push(Span::raw(" ".repeat(indent)));
            used = indent;
            first_on_line = true;
        }
        if !first_on_line {
            spans.push(Span::raw(CHIP_GAP));
            used += CHIP_GAP.len();
        }
        used += chip.width();
        spans.push(chip);
        first_on_line = false;
    }
    lines.push(Line::from(spans));

    ListItem::new(Text::from(lines))
}

/// Eyebrow section header — `TEXT_DIM + bold`, UPPERCASE (the sanctioned eyebrow
/// bold variant, always on).  Adds an underline while `active` (focus rests on a
/// row within this section) as the current-section cue.
pub fn section_header(label: &str, active: bool) -> ListItem<'static> {
    let mut style = Style::default().fg(text_dim()).bold();
    if active {
        style = style.underlined();
    }
    ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(label.to_uppercase(), style),
    ]))
}

pub fn help_item(text: impl Into<String>) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        "  └ ".fg(line()),
        text.into().fg(text_faint()),
    ]))
}

/// Splits copy carrying `[key]` markers into styled spans: each bracketed
/// token renders in `key_style` (brackets stripped), everything else in
/// `rest_style`. An unclosed `[` falls through as literal text.
pub fn keyed_spans(text: &str, key_style: Style, rest_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']') else {
            break;
        };
        if open > 0 {
            spans.push(Span::styled(rest[..open].to_string(), rest_style));
        }
        spans.push(Span::styled(
            rest[open + 1..open + close].to_string(),
            key_style,
        ));
        rest = &rest[open + close + 1..];
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), rest_style));
    }
    spans
}

/// [`help_item`] for copy that highlights bracketed tokens (key names or
/// example values): each `[token]` renders in the footer-hint key style
/// (`ACCENT + bold`) inside the faint tooltip text.
pub fn help_item_keyed(text: &str) -> ListItem<'static> {
    let mut spans = vec!["  └ ".fg(line())];
    spans.extend(keyed_spans(
        text,
        Style::default().fg(accent()).bold(),
        Style::default().fg(text_faint()),
    ));
    ListItem::new(Line::from(spans))
}

/// Builds a `[focus_span] [icon] [ label] [  detail] [suffix]` row.
///
/// Shared by [`row_item`] and [`disclosure_row`]; each caller supplies its own
/// focus span, icon, label style, and optional detail. The detail (when present)
/// is always rendered in `text_faint`. An optional pre-styled `suffix` span is
/// appended verbatim after the detail (the caller owns its leading spacing).
///
/// The selected-row `bg_hover` tint is applied by the list's `highlight_style`
/// (see [`render_list`]), not per-span here.
fn icon_label_row(
    focus: Span<'static>,
    icon: Span<'static>,
    label: &str,
    label_style: Style,
    detail: Option<String>,
    suffix: Option<Span<'static>>,
    label_width: usize,
) -> ListItem<'static> {
    // ` ` + padded label so the trailing detail stacks at the group's column.
    let label_text = format!(" {}", label_cell(label, label_width));
    let mut spans = vec![focus, icon, Span::styled(label_text, label_style)];
    if let Some(detail) = detail {
        spans.push(detail.fg(text_faint()));
    }
    if let Some(suffix) = suffix {
        spans.push(suffix);
    }
    ListItem::new(Line::from(spans))
}

pub fn disclosure_row(
    label: &str,
    detail: impl Into<String>,
    expanded: bool,
    focused: bool,
    expandable: bool,
    label_width: usize,
) -> ListItem<'static> {
    // An empty section can't be opened: drop the arrow and dim the label so the
    // row reads as inert rather than collapsed-but-openable.
    let marker = if !expandable {
        " "
    } else if expanded {
        EXPANDED
    } else {
        COLLAPSED
    };
    // Glyph: TEXT_DIM collapsed, ACCENT expanded.
    let glyph_color = if expanded { accent() } else { text_dim() };
    let label_style = if !expandable {
        Style::default().fg(text_faint())
    } else if expanded {
        Style::default().fg(accent()).bold()
    } else {
        focused_label(focused)
    };
    icon_label_row(
        focus_span(focused && !expanded),
        marker.fg(glyph_color),
        label,
        label_style,
        Some(detail.into()),
        None,
        label_width,
    )
}

pub fn row_item(
    label: &str,
    detail: Option<&str>,
    state: bool,
    focused: bool,
    label_width: usize,
) -> ListItem<'static> {
    row_item_with_suffix(label, detail, state, focused, None, label_width)
}

/// Like [`row_item`] but appends a pre-styled trailing `suffix` span after the
/// detail (e.g. the home tab's per-mirror latency readout). The base row —
/// focus marker, toggle glyph, label, and detail — is identical to [`row_item`].
pub fn row_item_with_suffix(
    label: &str,
    detail: Option<&str>,
    state: bool,
    focused: bool,
    suffix: Option<Span<'static>>,
    label_width: usize,
) -> ListItem<'static> {
    let toggle = toggle_spans(state);
    // A toggle has multiple spans for its glyph; flatten into a single item via
    // icon_label_row by using the first span as the icon and inserting the rest
    // before the label through a manual build.
    let caret = focus_span(focused);
    let label_style = focused_label(focused);
    let mut spans = vec![caret];
    spans.extend(toggle);
    spans.push(Span::styled(
        format!(" {}", label_cell(label, label_width)),
        label_style,
    ));
    if let Some(d) = detail {
        spans.push(d.to_string().fg(text_faint()));
    }
    if let Some(s) = suffix {
        spans.push(s);
    }
    ListItem::new(Line::from(spans))
}

/// A toggle row rendered **disabled** (inert): the toggle glyph, label, and
/// detail are all `text_faint`, signalling the row can't be activated. The focus
/// caret still shows (the row is focusable so the user sees where they are), but
/// the caller must skip activation and surface a reason — pair it with a
/// `help_item` tooltip on focus (cloudy-tui disabled-row pattern).
pub fn disabled_toggle_row(
    label: &str,
    detail: Option<&str>,
    state: bool,
    focused: bool,
    label_width: usize,
) -> ListItem<'static> {
    let faint = Style::default().fg(text_faint());
    // The toggle glyph is rendered faint too (not the live accent/line colors),
    // so the whole control reads as disabled.
    let toggle: Vec<Span<'static>> = match theme().tier() {
        Tier::Full if state => vec![Span::styled("─●", faint)],
        Tier::Full => vec![Span::styled("○─", faint)],
        Tier::Compatible if state => vec![Span::styled("[on]", faint)],
        Tier::Compatible => vec![Span::styled("[off]", faint)],
    };

    let mut spans = vec![focus_span(focused)];
    spans.extend(toggle);
    spans.push(Span::styled(
        format!(" {}", label_cell(label, label_width)),
        faint,
    ));
    if let Some(d) = detail {
        spans.push(Span::styled(d.to_string(), faint));
    }
    ListItem::new(Line::from(spans))
}

/// An action button (` label `), activated with `enter`.
///
/// Renders the action-only chip: `ACCENT + bold` on a `BG_RAISED` fill at rest
/// when `prominence` is [`ButtonProminence::Primary`], `TEXT_DIM` when it's
/// [`ButtonProminence::Secondary`]; an inverse `ACCENT` block (`fg = BG`) when
/// focused, 1-space inset. `enabled == false` is the disabled chip — the whole
/// pill goes `TEXT_FAINT`, focusable-but-inert (the caret still lands so the row
/// reads as selected); the caller skips activation and surfaces a reason.
///
/// `prominence` is the caller's call: the single primary CTA is the form's last
/// *enabled* action button in field order (`find`/`scan` → `view N maps` →
/// `download`), so every other action button drops to `Secondary`. A disabled
/// pill renders faint regardless of prominence, so the primary label can stay
/// pinned on the terminal button without shouting when it's inert.
pub fn button_item(
    label: &str,
    focused: bool,
    enabled: bool,
    prominence: ButtonProminence,
) -> ListItem<'static> {
    ListItem::new(Line::from(button_spans(
        label, focused, enabled, prominence,
    )))
}

/// [`button_item`] with extra pre-styled spans appended after the pill on the
/// SAME row (the find CTA carries its resolved-backend indicator this way). The
/// caller owns the trailing spans' leading gap and picks the pill's prominence.
pub fn button_item_with_trailing(
    label: &str,
    focused: bool,
    enabled: bool,
    trailing: Vec<Span<'static>>,
    prominence: ButtonProminence,
) -> ListItem<'static> {
    let mut spans = button_spans(label, focused, enabled, prominence);
    spans.extend(trailing);
    ListItem::new(Line::from(spans))
}

/// The `view N mapsets` button, with a trailing `⠋ loading titles` cue while
/// `enriching` — it stays pressable mid-fetch, so its loading state trails the
/// pill rather than swapping the label. Wrapped by [`view_browse_button`], the
/// single path every source's view button routes through.
pub fn button_item_with_loading_cue(
    label: &str,
    focused: bool,
    enabled: bool,
    enriching: bool,
    tick: u64,
    prominence: ButtonProminence,
) -> ListItem<'static> {
    let mut spans = button_spans(label, focused, enabled, prominence);
    if enriching {
        spans.push(Span::raw("  "));
        spans.push(loading_titles_span(tick));
    }
    ListItem::new(Line::from(spans))
}

/// Prominence tier of an action button at rest. Both tiers focus to the same
/// inverse-`ACCENT` block; they differ only in the blurred fill — a `Primary`
/// CTA keeps `ACCENT + bold`, a `Secondary` action drops to `TEXT_DIM` so it
/// doesn't shout as loud as the primary beside it (cloudy-tui action-only chip:
/// "reserve the prominent form for a genuine primary CTA"). The primary is the
/// form's last enabled action button — see [`ButtonProminence::primary_if`].
#[derive(Clone, Copy)]
pub enum ButtonProminence {
    Primary,
    Secondary,
}

impl ButtonProminence {
    /// `Primary` when `is_primary`, else `Secondary`. The call-site test is
    /// `field == primary`, where `primary` is the form's current CTA — the last
    /// enabled action button, falling back to `Download`.
    pub fn primary_if(is_primary: bool) -> Self {
        if is_primary {
            Self::Primary
        } else {
            Self::Secondary
        }
    }
}

fn button_spans(
    label: &str,
    focused: bool,
    enabled: bool,
    prominence: ButtonProminence,
) -> Vec<Span<'static>> {
    let pill = format!(" {label} ");

    let pill_style = if !enabled {
        Style::default().fg(text_faint()).bg(bg_raised())
    } else if focused {
        Style::default().fg(bg()).bg(accent()).bold()
    } else {
        match prominence {
            ButtonProminence::Primary => Style::default().fg(accent()).bold().bg(bg_raised()),
            ButtonProminence::Secondary => Style::default().fg(text_dim()).bg(bg_raised()),
        }
    };

    vec![focus_span(focused), Span::styled(pill, pill_style)]
}

/// Label + enabled state for a "download the checked sets" button (the search and
/// update source forms): `download (N)` when `selected` sets are checked, a
/// disabled bare `download` when none are. The count lives in the parens so the
/// label stays terse; the running total is also on the browse status line.
pub fn download_button_label(selected: usize) -> (String, bool) {
    if selected > 0 {
        (format!("download ({selected})"), true)
    } else {
        ("download".to_string(), false)
    }
}

/// [`download_button_label`] with a `· ~<size>` suffix when `known_bytes > 0` —
/// the summed nekoha sizes of the checked osu-routed find results. The `~` marks
/// it approximate: coverage is partial until every checked set's probe lands, so
/// the figure only grows. Reuses [`format_bytes`] (IEC units, `MiB` below a
/// `GiB`) to match the nzbasic size line on the same form. A zero sum (nothing
/// probed yet, or a route with no size cache) falls back to the plain label, so
/// the update and collection sources — which pass no size — keep the bare label.
pub fn download_button_label_with_size(selected: usize, known_bytes: u64) -> (String, bool) {
    let (label, enabled) = download_button_label(selected);
    if known_bytes > 0 {
        (
            format!("{label} · ~{}", format_bytes(known_bytes, "B")),
            enabled,
        )
    } else {
        (label, enabled)
    }
}

/// Label for a source's "open the results browse" button: `view N mapset(s)`.
/// The count is what the browse will show, so each source passes its own
/// (loaded rows / new count / resolved set count). Singular at 1.
pub fn view_maps_label(n: usize) -> String {
    format!("view {n} {}", if n == 1 { "mapset" } else { "mapsets" })
}

/// A source's "open the results browse" button, unified across every Get Maps
/// source (find / update / collection). One path so all three read and behave
/// identically: the browse opens id-only immediately and titles fill in behind
/// the trailing `⠋ loading titles` cue — no source defers its descend. `enabled`
/// gates the count label too, so a disabled button shows a generic `view maps`
/// (a stale-find or unresolved-collection button never advertises a count it
/// can't open).
pub fn view_browse_button(
    count: usize,
    focused: bool,
    enabled: bool,
    enriching: bool,
    tick: u64,
    prominence: ButtonProminence,
) -> ListItem<'static> {
    let label = if enabled {
        view_maps_label(count)
    } else {
        "view maps".to_string()
    };
    button_item_with_loading_cue(&label, focused, enabled, enriching, tick, prominence)
}

/// The main-content spans of a beatmapset browse row, shared by every "view
/// beatmaps" surface (find results, collection browse&pick, update missing-sets).
/// `artist - title` once metadata is folded in, else a bare `#id` — the
/// enrichment-in-flight state is no longer a per-row concern (it reads from the
/// owning panel's title-right meta instead, see [`meta_with_loading_cue`]). A
/// tier-coloured `★X.XX` suffix follows the title when the set carries a
/// difficulty spread, so the list is scannable without descending into the
/// preview.
///
/// `style` colors the id/title (the caller owns the cursor/dim treatment); the
/// star suffix always takes its tier colour regardless of `style`.
///
/// `max_width`: when `Some`, the star suffix is right-aligned within this many
/// cells — the title is truncated with a trailing `…` to fit, and the gap is
/// padded so the star always stays visible at the right edge. When `None`
/// (id-only rows, or callers that don't have a pane width), the star simply
/// trails the title.
pub fn browse_row_label(
    id: u32,
    meta: Option<&BeatmapSetMeta>,
    style: Style,
    max_width: Option<u16>,
) -> Vec<Span<'static>> {
    match meta {
        Some(meta) => {
            let title = format!("{} - {}", meta.artist, meta.title);
            if let Some(stars) = representative_stars(meta) {
                if let Some(max_w) = max_width {
                    return right_aligned_star_spans(&title, style, stars, max_w);
                }
                return vec![
                    Span::styled(title, style),
                    format!(" ★{stars:.2}").fg(stars_color(stars)),
                ];
            }
            vec![Span::styled(title, style)]
        }
        None => vec![Span::styled(format!("#{id}"), style)],
    }
}

/// `head` padded (or clipped with a trailing `…`) to exactly `head_width`
/// columns, then `trailing`. Whatever the individual heads cost, the figure
/// starts in one column across every row built to the same `head_width`.
///
/// Shared by every "label then figure" row. Where that column comes from is the
/// caller's call: a content column off the widest head (the preview spread's
/// names), or the row's own right edge ([`right_aligned_spans`]).
pub fn columned_spans(
    head: &str,
    head_style: Style,
    trailing: Vec<Span<'static>>,
    head_width: u16,
) -> Vec<Span<'static>> {
    let (truncated, used) = truncate_to_width(head, head_width);
    let pad = head_width.saturating_sub(used);
    let mut spans = Vec::with_capacity(trailing.len() + 2);
    spans.push(Span::styled(truncated, head_style));
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad as usize)));
    }
    spans.extend(trailing);
    spans
}

/// [`columned_spans`] with the head column taken from the row's right edge, so
/// `trailing` lands flush against `max_width`. The trailing block's width is
/// measured at build time, since a `★10.24` rating runs one cell wider than a
/// `★9.99` one.
///
/// Used by the browse list's star suffix ([`right_aligned_star_spans`]), where
/// every row shares one fixed-width column and the edge IS the column.
pub fn right_aligned_spans(
    head: &str,
    head_style: Style,
    trailing: Vec<Span<'static>>,
    max_width: u16,
) -> Vec<Span<'static>> {
    let trailing_width = trailing.iter().map(Span::width).sum::<usize>() as u16;
    let head_width = max_width.saturating_sub(trailing_width);
    columned_spans(head, head_style, trailing, head_width)
}

/// Title truncated to fit before a right-aligned tier-coloured star suffix.
/// Star always visible — the title gets clipped with trailing `…` when too
/// long, and the gap padded so the star lands at the right edge.
fn right_aligned_star_spans(
    title: &str,
    title_style: Style,
    stars: f64,
    max_width: u16,
) -> Vec<Span<'static>> {
    right_aligned_spans(
        title,
        title_style,
        vec![format!(" ★{stars:.2}").fg(stars_color(stars))],
        max_width,
    )
}

/// The hardest difficulty's star rating from a set's `beatmaps[]` spread — the
/// representative tier a browse-row star suffix uses. `None` when the carrier
/// response omitted the array (id-only rows still enrich behind it).
fn representative_stars(meta: &BeatmapSetMeta) -> Option<f64> {
    meta.beatmaps
        .iter()
        .map(|b| b.difficulty_rating)
        .reduce(f64::max)
}

/// The enrichment-in-flight cue text (`⠋ loading titles`), dim so it reads as
/// chrome. Shared by [`button_item_with_loading_cue`] and
/// [`meta_with_loading_cue`] so every surface waiting on the same osu-batch
/// backfill reads identically.
fn loading_titles_span(tick: u64) -> Span<'static> {
    format!("{} loading titles", spinner_str(tick).trim()).fg(text_dim())
}

/// Appends the [`loading_titles_span`] cue to a panel's title-right meta line
/// while `enriching`, e.g. the set-browse list pane's selected/total ratio or
/// the update preview's `N new · M removed` — the existing line is kept, never
/// dropped, so the cue reads as an addition rather than a replacement. `base`
/// passes through unchanged while idle.
pub fn meta_with_loading_cue(base: Line<'static>, enriching: bool, tick: u64) -> Line<'static> {
    if !enriching {
        return base;
    }
    let mut spans = base.spans;
    spans.push(SEPARATOR.fg(line()));
    spans.push(loading_titles_span(tick));
    Line::from(spans)
}

/// A `label value` metric line separated by [`SEPARATOR`].
///
/// Metric styling: each label is lowercase `TEXT_FAINT` (a recessive
/// tag), with its value beside it in its own brighter color — never the
/// UPPERCASE bold eyebrow, which reads as a section header.
pub fn summary_line(metrics: &[Metric<'_>]) -> Line<'static> {
    let mut spans = vec![Span::raw("  ")];
    for (index, metric) in metrics.iter().enumerate() {
        if index > 0 {
            spans.push(SEPARATOR.fg(line()));
        }
        spans.push(metric.label.to_owned().fg(text_faint()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(metric.value.clone(), metric.style));
    }
    Line::from(spans)
}

/// A `selected/total` ratio line for header-border meta. Selected is accent
/// when > 0, neutral otherwise; total is always neutral.
pub fn ratio_line(selected: usize, total: usize) -> Line<'static> {
    let selected_color = if selected > 0 { accent() } else { text_dim() };
    Line::from(vec![
        selected.to_string().fg(selected_color),
        "/".fg(text_faint()),
        total.to_string().fg(text_dim()),
    ])
}

/// [`summary_line`] as a list item, for form / list render paths.
pub fn summary_item(metrics: &[Metric<'_>]) -> ListItem<'static> {
    ListItem::new(summary_line(metrics))
}

/// Builds a `[ label ]` status pill as a `Line`.
///
/// Brackets are always `TEXT_DIM`.  Label color: semantic (`SUCCESS` / `WARNING`
/// / `DANGER`) for charged states, `TEXT_DIM` for neutral steady states.
/// Label is always bold.
pub fn status_pill(label: impl Into<String>, color: Color) -> Line<'static> {
    Line::from(vec![
        "[ ".fg(text_dim()),
        label.into().fg(color).bold(),
        " ]".fg(text_dim()),
    ])
}

pub fn spacer() -> ListItem<'static> {
    ListItem::new(Line::from(""))
}

pub fn status_style(stage: DownloadStage) -> Style {
    match stage {
        DownloadStage::Pending | DownloadStage::Resolving | DownloadStage::Rechecking => {
            Style::default().fg(warning())
        }
        DownloadStage::Downloading => Style::default().fg(info()),
        DownloadStage::Completed => Style::default().fg(success()),
        DownloadStage::Failed => Style::default().fg(danger()),
    }
}

pub fn active_download_item(dl: &ActiveDownloadLine, width: u16) -> ListItem<'static> {
    active_download_item_msg(dl, &dl.displayed_message(), width)
}

/// Like [`active_download_item`] but accepts an explicit message string.
///
/// Used by the rate-limited renderer to splice a countdown suffix into the
/// message before truncation without duplicating the progress-bar layout logic.
pub fn active_download_item_msg(
    dl: &ActiveDownloadLine,
    message_text: &str,
    width: u16,
) -> ListItem<'static> {
    const BAR_WIDTH: u16 = 12;
    const LABEL_WIDTH: u16 = 5;
    const GAP: u16 = 1;
    const RESERVED_RIGHT: u16 = BAR_WIDTH + GAP + LABEL_WIDTH;

    let prefix = {
        let id_s = dl.beatmapset_id.to_string();
        let pad = 7usize.saturating_sub(id_s.len());
        let mut s = String::with_capacity(3 + 7 + 1);
        s.push_str("  #");
        s.push_str(&id_s);
        for _ in 0..pad {
            s.push(' ');
        }
        s.push(' ');
        s
    };
    let prefix_w = prefix.len() as u16;
    let rate_limited = dl.displayed_rate_limited();
    let bar_color = dl.bar_color();

    let message_budget = width
        .saturating_sub(prefix_w)
        .saturating_sub(RESERVED_RIGHT)
        .saturating_sub(GAP);
    let (message, message_w) = truncate_to_width(message_text, message_budget);

    let mut spans = vec![
        prefix.fg(text_faint()),
        Span::styled(message, message_style(dl.stage, rate_limited)),
    ];

    let used = prefix_w.saturating_add(message_w);
    let pad = width.saturating_sub(used).saturating_sub(RESERVED_RIGHT) as usize;
    spans.push(Span::raw(
        glyph_fill(&FILL_SPACE, GLYPH_SPACE, pad).into_owned(),
    ));

    match dl.progress_ratio() {
        Some(ratio) => {
            let filled = ((ratio * BAR_WIDTH as f32).round() as u16).min(BAR_WIDTH);
            let empty = BAR_WIDTH - filled;
            spans.push(
                glyph_fill(&FILL_BLOCK, GLYPH_BLOCK, filled as usize)
                    .into_owned()
                    .fg(bar_color),
            );
            spans.push(
                glyph_fill(&FILL_SHADE, GLYPH_SHADE, empty as usize)
                    .into_owned()
                    .fg(line()),
            );
            let pct = (ratio * 100.0).round() as u16;
            spans.push(pct_label(pct).fg(text_faint()));
        }
        None if matches!(dl.stage, crate::download::BeatmapStage::Downloading) => {
            spans.extend(indeterminate_bar_spans(BAR_WIDTH, bar_color));
            spans.push("  …".fg(text_faint()));
        }
        None => {
            spans.push(
                glyph_fill(&FILL_SHADE, GLYPH_SHADE, BAR_WIDTH as usize)
                    .into_owned()
                    .fg(line()),
            );
            spans.push("     ".fg(text_faint()));
        }
    }

    ListItem::new(Line::from(spans))
}

/// The bracketed bouncing-block indeterminate bar: a `[ … ]` frame in `line()`
/// color with a short filled chunk that bounces inside the track. Shared by the
/// per-row mini-bar and the resolve panel's no-known-total bar so both pulse
/// identically. Time-driven (one global clock) — no per-page tick state.
pub(super) fn indeterminate_bar_spans(width: u16, bar_color: Color) -> Vec<Span<'static>> {
    // The `[ … ]` frame (in `line()`) is the determinate/indeterminate tell; the
    // bouncing block travels inside it, so the inner track is `width - 2` cells
    // and total width stays `width`.
    let width = width as usize;
    let inner = width.saturating_sub(2);
    let segment = 4usize.min(inner);
    let travel = inner.saturating_sub(segment);
    let tick = animation_start().elapsed().as_millis() as usize / 90;
    let cycle = travel.saturating_mul(2).max(1);
    let phase = tick % cycle;
    let offset = if phase <= travel {
        phase
    } else {
        cycle.saturating_sub(phase)
    };

    let frame_style = Style::default().fg(line());
    let mut spans = vec![Span::styled("[", frame_style)];
    if offset > 0 {
        spans.push(Span::styled(
            glyph_fill(&FILL_SHADE, GLYPH_SHADE, offset).into_owned(),
            frame_style,
        ));
    }
    spans.push(
        glyph_fill(&FILL_BLOCK, GLYPH_BLOCK, segment)
            .into_owned()
            .fg(bar_color),
    );
    let right = inner.saturating_sub(offset).saturating_sub(segment);
    if right > 0 {
        spans.push(Span::styled(
            glyph_fill(&FILL_SHADE, GLYPH_SHADE, right).into_owned(),
            frame_style,
        ));
    }
    spans.push(Span::styled("]", frame_style));
    spans
}

fn animation_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

pub fn truncate_to_width(message: &str, budget: u16) -> (String, u16) {
    use unicode_width::UnicodeWidthChar as _;
    use unicode_width::UnicodeWidthStr as _;

    let budget = budget as usize;
    if budget == 0 {
        return (String::new(), 0);
    }
    let display_width = message.width();
    if display_width <= budget {
        return (message.to_string(), display_width as u16);
    }
    if budget == 1 {
        return ("…".to_string(), 1);
    }
    // Reserve 1 column for the ellipsis; accumulate chars until we'd overflow.
    let target = budget.saturating_sub(1);
    let mut out = String::with_capacity(message.len());
    let mut used = 0usize;
    for ch in message.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > target {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    // `used + 1`, never `budget`: a cut landing mid double-width glyph stops a
    // column short of the target, and a caller padding by the difference (the
    // right-aligned star block) would otherwise sit one column off.
    (out, (used + 1) as u16)
}

// 101-entry table of " {:>3}%" strings for pct in 0..=100.
// Returned by `pct_label` to avoid per-frame allocation in `active_download_item`.
const PCT_LABELS: [&str; 101] = [
    "   0%", "   1%", "   2%", "   3%", "   4%", "   5%", "   6%", "   7%", "   8%", "   9%",
    "  10%", "  11%", "  12%", "  13%", "  14%", "  15%", "  16%", "  17%", "  18%", "  19%",
    "  20%", "  21%", "  22%", "  23%", "  24%", "  25%", "  26%", "  27%", "  28%", "  29%",
    "  30%", "  31%", "  32%", "  33%", "  34%", "  35%", "  36%", "  37%", "  38%", "  39%",
    "  40%", "  41%", "  42%", "  43%", "  44%", "  45%", "  46%", "  47%", "  48%", "  49%",
    "  50%", "  51%", "  52%", "  53%", "  54%", "  55%", "  56%", "  57%", "  58%", "  59%",
    "  60%", "  61%", "  62%", "  63%", "  64%", "  65%", "  66%", "  67%", "  68%", "  69%",
    "  70%", "  71%", "  72%", "  73%", "  74%", "  75%", "  76%", "  77%", "  78%", "  79%",
    "  80%", "  81%", "  82%", "  83%", "  84%", "  85%", "  86%", "  87%", "  88%", "  89%",
    "  90%", "  91%", "  92%", "  93%", "  94%", "  95%", "  96%", "  97%", "  98%", "  99%",
    " 100%",
];

fn pct_label(pct: u16) -> &'static str {
    PCT_LABELS[pct.min(100) as usize]
}

fn message_style(stage: crate::download::BeatmapStage, rate_limited: bool) -> Style {
    use crate::download::BeatmapStage;
    if rate_limited {
        return Style::default().fg(warning());
    }
    match stage {
        BeatmapStage::Failed | BeatmapStage::Aborted => Style::default().fg(danger()),
        BeatmapStage::Success => Style::default().fg(success()),
        BeatmapStage::Skipped => Style::default().fg(text_faint()),
        BeatmapStage::Pending | BeatmapStage::Downloading | BeatmapStage::Verifying => {
            Style::default().fg(text_dim())
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tui_widgets.rs"]
mod tests;
