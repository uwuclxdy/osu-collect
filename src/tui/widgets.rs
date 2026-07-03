use crate::{
    app::{ActiveDownloadLine, InputField},
    download::DownloadStage,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, List, ListItem, ListState, Padding, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
};
use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Instant;

use super::theme::{Tier, theme};
use super::{
    FILL_BLOCK, FILL_SHADE, FILL_SPACE, GLYPH_BLOCK, GLYPH_SHADE, GLYPH_SPACE, accent, accent_alt,
    bg, bg_hover, danger, focused_label, glyph_fill, info, line, line_strong, success, text_dim,
    text_faint, warning,
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

/// Renders an item list into `inner` with `ListState`-driven scrolling and,
/// when `highlight` is set, the [`highlight_style`] on the focused
/// row, then draws the overflow [`render_scrollbar`] in the panel's right
/// padding column.
///
/// `focused` is the row to scroll into view (`None` for panels with no cursor —
/// e.g. the blurred collection list). The scroll target is decoupled from the
/// highlight: when `highlight` is `false` (the focused row styles itself — the
/// CTA button, the auth chip) the row is still scrolled into view but the
/// `bg_hover` bar is suppressed so the row's own styling shows through.
/// `List::scroll_padding(1)` keeps one row of context above/below while it
/// scrolls.
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
    // Persist the scroll offset across frames: seed the list with the previous
    // offset so ratatui only scrolls when the focused row falls outside the
    // viewport. A fresh `ListState::default()` each frame re-pins the selection
    // to the panel's bottom edge on every redraw (offset resets to 0, then
    // `select` scrolls the minimum to reveal it — always the bottom once past
    // the first page).
    let mut state = ListState::default().with_offset(offset.get());
    // Always scroll the focused row into view; only the highlight bar is gated.
    state.select(focused);
    // A self-styling focused row (CTA / auth chip) keeps its own styling by
    // rendering a neutral highlight that leaves the row's spans untouched.
    let row_style = if highlight {
        highlight_style()
    } else {
        Style::default()
    };
    let list = List::new(items)
        .scroll_padding(1)
        .highlight_symbol("")
        .highlight_style(row_style);
    frame.render_stateful_widget(list, inner, &mut state);
    let resolved = state.offset();
    offset.set(resolved);
    render_scrollbar(frame, inner, resolved, total);
    resolved
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
    title: &'static str,
    items: Vec<ListItem<'static>>,
    focused_index: usize,
    highlight: bool,
    cursor_col: Option<u16>,
    focused: bool,
    first_panel: bool,
    offset: &Cell<usize>,
) {
    let block = panel_block(title, focused, first_panel);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let total = items.len();
    let start = render_list(frame, inner, items, Some(focused_index), highlight, offset);
    let end = (start + inner.height as usize).min(total);

    set_panel_cursor(frame, inner, focused_index, start, end, cursor_col);
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
pub fn set_panel_cursor(
    frame: &mut Frame,
    inner: Rect,
    focused_index: usize,
    start: usize,
    end: usize,
    cursor_col: Option<u16>,
) {
    let Some(col) = cursor_col else { return };
    if inner.width == 0 || inner.height == 0 || focused_index < start || focused_index >= end {
        return;
    }
    let y = inner.y + (focused_index - start) as u16;
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
/// Callers must pass an already-uppercased, space-padded title constant
/// (e.g. `" OVERVIEW "`). This avoids per-call allocation; use the module-level
/// `PANEL_*` constants defined in each view module.
pub fn panel_block(title: &'static str, focused: bool, first_panel: bool) -> Block<'static> {
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
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        // Tab title sits right after the rounded corner: `╭ TITLE ─`.
        .title(Line::from(vec![
            Span::styled(title, title_style),
            Span::styled("─", Style::default().fg(border_color)),
        ]))
        .padding(Padding::new(1, 1, 0, 0))
}

pub fn focus_span(focused: bool) -> Span<'static> {
    if focused {
        Span::styled(FOCUS_MARK, Style::default().fg(accent()))
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
        Span::styled("x", Style::default().fg(accent()))
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
                vec![
                    Span::styled("─", Style::default().fg(line())),
                    Span::styled("●", Style::default().fg(accent())),
                ]
            } else {
                vec![
                    Span::styled("○", Style::default().fg(text_dim())),
                    Span::styled("─", Style::default().fg(line())),
                ]
            }
        }
        Tier::Compatible => {
            if on {
                vec![
                    Span::styled("[", Style::default().fg(text_dim())),
                    Span::styled("on", Style::default().fg(accent())),
                    Span::styled("]", Style::default().fg(text_dim())),
                ]
            } else {
                vec![
                    Span::styled("[", Style::default().fg(text_dim())),
                    Span::styled("off", Style::default().fg(text_dim())),
                    Span::styled("]", Style::default().fg(text_dim())),
                ]
            }
        }
    }
}

/// Leading glyph for a text-input row: `✎` when the row is being edited, `❯`
/// when selected-not-editing, two-space pad when blurred.
pub fn input_focus_span(focused: bool, editing: bool) -> Span<'static> {
    if focused && editing {
        Span::styled(EDIT_MARK, Style::default().fg(accent()))
    } else if focused {
        Span::styled(FOCUS_MARK, Style::default().fg(accent()))
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
        Span::styled(field.placeholder.clone(), Style::default().fg(text_faint()))
    } else {
        Span::styled(field.value.clone(), Style::default().fg(accent()))
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
        Span::styled(field.placeholder.clone(), Style::default().fg(text_faint()))
    } else {
        Span::styled(
            "•".repeat(field.value.chars().count()),
            Style::default().fg(accent()),
        )
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

/// A stepper row showing a numeric value with an optional "recommended: N" chip.
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
    let value_span = Span::styled(s, Style::default().fg(accent()));

    let mut spans = vec![
        focus_span(focused),
        Span::styled(label_cell(label, label_width), focused_label(focused)),
        value_span,
    ];

    if value != recommended {
        let mut chip = String::with_capacity(16);
        chip.push_str("  recommended: ");
        chip.push_str(&recommended.to_string());
        spans.push(Span::styled(chip, Style::default().fg(text_faint())));
    }

    ListItem::new(Line::from(spans))
}

pub fn cycle_item(
    label: &str,
    options: &[&str],
    selected: &str,
    focused: bool,
    label_width: usize,
) -> ListItem<'static> {
    let mut spans = vec![
        focus_span(focused),
        Span::styled(label_cell(label, label_width), focused_label(focused)),
    ];
    for (index, &option) in options.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        if option == selected {
            // [brackets] only while the row is focused; ACCENT, no bold.
            if focused {
                spans.push(Span::styled(
                    format!("[{option}]"),
                    Style::default().fg(accent()),
                ));
            } else {
                spans.push(Span::styled(
                    option.to_string(),
                    Style::default().fg(accent()),
                ));
            }
        } else {
            spans.push(Span::styled(
                option.to_string(),
                Style::default().fg(text_faint()),
            ));
        }
    }
    ListItem::new(Line::from(spans))
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
        Span::styled("  └ ", Style::default().fg(line())),
        Span::styled(text.into(), Style::default().fg(text_faint())),
    ]))
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
        spans.push(Span::styled(detail, Style::default().fg(text_faint())));
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
        Span::styled(marker, Style::default().fg(glyph_color)),
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
        spans.push(Span::styled(
            d.to_string(),
            Style::default().fg(text_faint()),
        ));
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

/// A button row rendered as a filled pill, activated with `enter`.
///
/// `enabled` greys the label when the action is currently unavailable (e.g. no
/// maps selected). The button is still rendered so its position stays stable.
pub fn button_item(label: &str, focused: bool, enabled: bool) -> ListItem<'static> {
    let mut pill = String::with_capacity(label.len() + 4);
    pill.push_str("  ");
    pill.push_str(label);
    pill.push_str("  ");

    let pill_style = if !enabled && !focused {
        Style::default().fg(text_faint())
    } else if !enabled {
        // focused but disabled: show dim accent so the row is visibly selected
        Style::default().fg(accent()).dim()
    } else if focused {
        Style::default().fg(bg()).bg(accent()).bold()
    } else {
        Style::default().fg(accent()).bold()
    };

    let spans = vec![focus_span(focused), Span::styled(pill, pill_style)];
    ListItem::new(Line::from(spans))
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
            spans.push(Span::styled(SEPARATOR, Style::default().fg(line())));
        }
        spans.push(Span::styled(
            metric.label.to_owned(),
            Style::default().fg(text_faint()),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(metric.value.clone(), metric.style));
    }
    Line::from(spans)
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
        Span::styled("[ ", Style::default().fg(text_dim())),
        Span::styled(label.into(), Style::default().fg(color).bold()),
        Span::styled(" ]", Style::default().fg(text_dim())),
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
        Span::styled(prefix, Style::default().fg(text_faint())),
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
            spans.push(Span::styled(
                glyph_fill(&FILL_BLOCK, GLYPH_BLOCK, filled as usize).into_owned(),
                Style::default().fg(bar_color),
            ));
            spans.push(Span::styled(
                glyph_fill(&FILL_SHADE, GLYPH_SHADE, empty as usize).into_owned(),
                Style::default().fg(line()),
            ));
            let pct = (ratio * 100.0).round() as u16;
            spans.push(Span::styled(
                pct_label(pct),
                Style::default().fg(text_faint()),
            ));
        }
        None if matches!(dl.stage, crate::download::BeatmapStage::Downloading) => {
            spans.extend(indeterminate_bar_spans(BAR_WIDTH, bar_color));
            spans.push(Span::styled("  ...", Style::default().fg(text_faint())));
        }
        None => {
            spans.push(Span::styled(
                glyph_fill(&FILL_SHADE, GLYPH_SHADE, BAR_WIDTH as usize).into_owned(),
                Style::default().fg(line()),
            ));
            spans.push(Span::styled("     ", Style::default().fg(text_faint())));
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
    spans.push(Span::styled(
        glyph_fill(&FILL_BLOCK, GLYPH_BLOCK, segment).into_owned(),
        Style::default().fg(bar_color),
    ));
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
    (out, budget as u16)
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
