//! Reusable modal overlay primitives.
//!
//! # Usage
//!
//! 1. Render [`ratatui::widgets::Clear`] over the popup area to erase the
//!    content behind it.
//! 2. Call the specific overlay renderer (e.g. [`render_help_overlay`]).
//!
//! Future modals follow the same pattern: add a render function here that
//! accepts `frame` and `area`, compute the popup rect with inline
//! `Layout::vertical` + `Layout::horizontal`, and call
//! `frame.render_widget(Clear, popup_area)` before drawing.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
};

use unicode_width::UnicodeWidthStr;

use super::widgets;
use super::{accent, accent_alt, bg, text, text_dim};
use crate::app::state::{CONFIRM_RETRY_BUTTONS, RETRY_ON_START_BUTTONS};
use crate::config::constants::{CONFIG_TAB_INDEX, HOME_TAB_INDEX, UPDATES_TAB_INDEX};

/// Modal sizing cap: a modal never exceeds 60% of the terminal width.
/// Below that it shrinks to its content, so a big terminal doesn't inflate it.
const MODAL_MAX_WIDTH_PCT: usize = 60;
/// Horizontal chrome subtracted from the popup width to reach the content area:
/// border (2) + 2-cell padding each side (4). Wider than the vertical padding so
/// the side gaps read as deep as the top/bottom rows (cells are ~2× taller).
const MODAL_CHROME_W: usize = 6;
/// Vertical chrome subtracted from the popup height: border (2) + 1-row padding
/// top and bottom (2).
const MODAL_CHROME_H: usize = 4;

/// Centered popup width that fits `content_w` (the widest inner line) plus chrome,
/// never below what the titled top border needs, and never past 60% of the
/// terminal — shrinks to content, grows only up to the cap.
fn modal_width(area_width: u16, content_w: usize, title: &str) -> u16 {
    let cap = ((area_width as usize * MODAL_MAX_WIDTH_PCT) / 100).max(1);
    (content_w + MODAL_CHROME_W)
        .max(title.width() + 2) // title sits in the top border; keep it whole
        .min(cap)
        .min(area_width as usize)
        .max(1) as u16
}

/// Display width of a rendered line (sum of its spans' widths).
fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

/// Standard inner padding for every modal popup: 2 cells left/right, 1 row
/// top/bottom — content never touches the border and the side gaps read as deep
/// as the top/bottom rows (terminal cells are ~2× taller than wide).
fn modal_padding() -> Padding {
    Padding::new(2, 2, 1, 1)
}

/// Builds the standard bordered modal block: rounded `accent_alt` (orange)
/// border — the sole modal anchor — base `BG` fill (no raised tone, no
/// backdrop), and a `text_dim` italic-only title (no bold).
///
/// Callers add a scroll indicator via `title_top` afterwards when needed.
fn modal_block(title: &'static str) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent_alt()))
        .style(Style::default().bg(bg()))
        .title(Span::styled(
            title,
            Style::default().fg(text_dim()).italic(),
        ))
        .padding(modal_padding())
}

/// Key/action pair for the help overlay.
struct HelpRow {
    key: &'static str,
    action: &'static str,
}

impl HelpRow {
    const fn new(key: &'static str, action: &'static str) -> Self {
        Self { key, action }
    }
}

const GLOBAL: &[HelpRow] = &[
    HelpRow::new("← → / tab", "switch tabs"),
    HelpRow::new("↑ ↓", "move / scroll"),
    HelpRow::new("↵", "activate / toggle / edit field"),
    HelpRow::new("space", "toggle selection"),
    HelpRow::new("s", "jump to download button"),
    HelpRow::new("esc", "exit edit / back"),
    HelpRow::new("?", "toggle help"),
    HelpRow::new("q", "back / quit"),
];

/// Shown above the per-tab section only while the vim keymap is enabled.
const VIM: &[HelpRow] = &[
    HelpRow::new("h l", "prev / next tab"),
    HelpRow::new("j k", "move / scroll"),
    HelpRow::new("g g / G", "top / bottom"),
    HelpRow::new("ctrl+d / u", "page down / up"),
    HelpRow::new("i / a", "edit focused field"),
];

const HOME_TAB: &[HelpRow] = &[HelpRow::new("↵", "edit field / activate row")];

const UPDATES_TAB: &[HelpRow] = &[
    HelpRow::new("↵", "expand list / download"),
    HelpRow::new("a", "select all"),
    HelpRow::new("d", "select none"),
    HelpRow::new("r", "recheck failed"),
];

const CONFIG_TAB: &[HelpRow] = &[HelpRow::new("↵ (auth chip)", "open login tab")];

const LOGIN_TAB: &[HelpRow] = &[
    HelpRow::new("↵", "edit field / submit"),
    HelpRow::new("esc / q", "close login tab"),
];

const DOWNLOAD_TAB: &[HelpRow] = &[
    HelpRow::new("↵", "expand / collapse failed"),
    HelpRow::new("↑ ↓", "navigate failed rows"),
    HelpRow::new("r", "retry failed maps"),
    HelpRow::new("s", "defer rate-limited"),
    HelpRow::new("S", "drop rate-limited"),
    HelpRow::new("esc / q", "close completed tab"),
];

/// Renders a centred keybindings overlay.
///
/// Call this after all other tab content and the footer have been drawn —
/// it clears the area it occupies and draws on top.
///
/// `scroll` is the requested top-row offset; it is clamped to the real viewport
/// (the list never scrolls past its last screen) and the clamped value is
/// returned so the caller can store it back — this keeps ↑/↓ anchored to what is
/// actually on screen, with no dead presses when the list already fits.
pub(crate) fn render_help_overlay(
    frame: &mut Frame,
    area: Rect,
    scroll: usize,
    active_tab: usize,
    is_login_tab: bool,
    vim_keys: bool,
) -> usize {
    let lines = build_help_lines(active_tab, is_login_tab, vim_keys);
    let content_w = lines.iter().map(line_width).max().unwrap_or(0);
    let items: Vec<ListItem<'static>> = lines.into_iter().map(ListItem::new).collect();

    // Size the popup to fit all items exactly (+MODAL_CHROME_H for the border and
    // 1-row top/bottom padding), capped at the terminal height so it never
    // overflows on very small terminals. Width shrinks to the longest row, capped
    // at 60% of the terminal.
    let needed_height = (items.len() as u16)
        .saturating_add(MODAL_CHROME_H as u16)
        .min(area.height);
    let popup_w = modal_width(area.width, content_w, " KEYBINDINGS ");
    let [popup_area] = Layout::vertical([Constraint::Length(needed_height)])
        .flex(Flex::Center)
        .areas(area);
    let [popup_area] = Layout::horizontal([Constraint::Length(popup_w)])
        .flex(Flex::Center)
        .areas(popup_area);
    frame.render_widget(Clear, popup_area);

    let outer_block = modal_block(" KEYBINDINGS ");

    let inner = outer_block.inner(popup_area);
    let total = items.len();
    let visible_height = inner.height as usize;
    // Top-anchored scroll: clamp the offset so the last row never scrolls above
    // the bottom edge.
    let max_start = total.saturating_sub(visible_height);
    let start = scroll.min(max_start);
    frame.render_widget(outer_block, popup_area);

    // Reference sheet — no row cursor, so the list scrolls by offset with no
    // selection highlight.
    let mut state = ListState::default().with_offset(start);
    frame.render_stateful_widget(List::new(items), inner, &mut state);
    widgets::render_scrollbar(frame, inner, start, total);
    start
}

/// Renders the pre-download "retry failed?" modal.
///
/// Buttons (`cancel`, `skip`, `retry`) carry the choices; `focus` marks the
/// selected one. ←/→ move it, `enter` activates it, `esc` cancels the download
/// (all handled by `handle_key`).
pub(crate) fn render_retry_on_start_modal(
    frame: &mut Frame,
    area: Rect,
    count: usize,
    focus: usize,
) {
    let body = vec![Line::from(vec![
        Span::styled(count.to_string(), Style::default().fg(accent()).bold()),
        Span::styled(
            " failed maps from a previous run. retry them?",
            Style::default().fg(text_dim()),
        ),
    ])];
    render_button_modal(
        frame,
        area,
        " RETRY FAILED? ",
        body,
        &RETRY_ON_START_BUTTONS,
        focus,
    );
}

/// Renders the "retry N failed maps?" confirmation modal.
///
/// Buttons (`cancel`, `retry`); `enter` activates the focused one, `esc`/`q`
/// cancel. The modal intercepts all other keys while open.
pub(crate) fn render_confirm_retry_modal(
    frame: &mut Frame,
    area: Rect,
    count: usize,
    focus: usize,
) {
    let body = vec![Line::from(vec![
        Span::styled("retry ", Style::default().fg(text_dim())),
        Span::styled(count.to_string(), Style::default().fg(accent()).bold()),
        Span::styled(" failed maps?", Style::default().fg(text_dim())),
    ])];
    render_button_modal(
        frame,
        area,
        " CONFIRM RETRY ",
        body,
        &CONFIRM_RETRY_BUTTONS,
        focus,
    );
}

/// Renders a centred confirm/prompt modal: a wrapped body block on top and a
/// right-aligned button group on the last inner row.
///
/// The modal sizes to its content — width fits the longest body / button line
/// (capped at 60% of the terminal), height fits the body (wrapped at that width)
/// plus a blank row and the button row. It never inflates to fill a big terminal.
/// Buttons render via [`button_spans`].
fn render_button_modal(
    frame: &mut Frame,
    area: Rect,
    title: &'static str,
    body: Vec<Line<'static>>,
    buttons: &[&'static str],
    focus: usize,
) {
    let content_w = body
        .iter()
        .map(line_width)
        .max()
        .unwrap_or(0)
        .max(button_row_width(buttons));
    let popup_w = modal_width(area.width, content_w, title);

    // Wrap the body at the inner content width to learn its real height, then add
    // a blank separator row + the button row.
    let inner_w = (popup_w as usize).saturating_sub(MODAL_CHROME_W).max(1);
    let body_rows: u16 = body
        .iter()
        .map(|line| wrapped_rows(line_width(line), inner_w))
        .sum::<u16>()
        .max(1);
    // inner rows = body + blank separator + button row; +MODAL_CHROME_H for the
    // border and the 1-row top/bottom padding.
    let popup_h = (body_rows + 2 + MODAL_CHROME_H as u16)
        .min(area.height)
        .max(1);

    let [popup_area] = Layout::vertical([Constraint::Length(popup_h)])
        .flex(Flex::Center)
        .areas(area);
    let [popup_area] = Layout::horizontal([Constraint::Length(popup_w)])
        .flex(Flex::Center)
        .areas(popup_area);
    frame.render_widget(Clear, popup_area);

    let outer_block = modal_block(title);
    let inner = outer_block.inner(popup_area);
    frame.render_widget(outer_block, popup_area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let body_area = Rect {
        height: body_rows.min(inner.height),
        ..inner
    };
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: true }), body_area);

    // Right-aligned button group on the last inner row: 3-space gap,
    // no separator glyph; the focused button is the only inverse block.
    let button_area = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new(Line::from(button_spans(buttons, focus))).alignment(Alignment::Right),
        button_area,
    );
}

/// Display width of the rendered button row. Every button always carries its
/// 1-cell insets (`+2`) and a 1-cell gap sits between buttons, so the row width —
/// and every button's position — is identical no matter which one is focused.
fn button_row_width(buttons: &[&str]) -> usize {
    if buttons.is_empty() {
        return 0;
    }
    let labels: usize = buttons.iter().map(|b| b.width() + 2).sum();
    labels + (buttons.len() - 1)
}

/// Rows a line of display width `w` occupies when wrapped at `content_w`.
fn wrapped_rows(w: usize, content_w: usize) -> u16 {
    if content_w == 0 {
        return 1;
    }
    w.div_ceil(content_w).max(1) as u16
}

/// Build the right-aligned modal button row. EVERY button is rendered as
/// ` label ` (1-cell insets always present) with a 1-cell gap between buttons, so
/// no button moves when focus changes — only the highlight toggles. The focused
/// button fills its ` label ` as a neutral inverse block (`fg = BG`, `bg = TEXT`,
/// no bold — the insets are its caps, never literal `▐`/`▌`); the rest are plain
/// `TEXT_DIM`. Because the trailing inset is always reserved, the rightmost
/// button reads as 3 plain cells of right-spacing when unselected (1 inset + the
/// modal's 2-cell padding) and 1 highlighted + 2 plain when selected.
fn button_spans(buttons: &[&'static str], focus: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::with_capacity(buttons.len() * 2);
    for (i, label) in buttons.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let style = if i == focus {
            Style::default().fg(bg()).bg(text())
        } else {
            Style::default().fg(text_dim())
        };
        spans.push(Span::styled(format!(" {label} "), style));
    }
    spans
}

/// Builds the help rows as measurable [`Line`]s: the always-shown `global`
/// section plus only the section for the currently active tab. Download tabs
/// (any index past the static three) map to the `download` section.
fn build_help_lines(active_tab: usize, is_login_tab: bool, vim_keys: bool) -> Vec<Line<'static>> {
    let (heading, rows) = if is_login_tab {
        ("login", LOGIN_TAB)
    } else {
        match active_tab {
            HOME_TAB_INDEX => ("home", HOME_TAB),
            UPDATES_TAB_INDEX => ("updates", UPDATES_TAB),
            CONFIG_TAB_INDEX => ("config", CONFIG_TAB),
            _ => ("download", DOWNLOAD_TAB),
        }
    };

    let mut lines = Vec::new();
    push_section(&mut lines, "global", GLOBAL);
    if vim_keys {
        lines.push(Line::from(""));
        push_section(&mut lines, "vim", VIM);
    }
    lines.push(Line::from(""));
    push_section(&mut lines, heading, rows);
    lines
}

fn push_section(lines: &mut Vec<Line<'static>>, heading: &'static str, rows: &[HelpRow]) {
    lines.push(section_heading(heading));
    for row in rows {
        lines.push(help_row(row.key, row.action));
    }
}

fn section_heading(label: &'static str) -> Line<'static> {
    // Help-modal section header: eyebrow — TEXT_DIM, UPPERCASE TRACKED, no bold.
    Line::from(vec![Span::styled(
        label.to_uppercase(),
        Style::default().fg(text_dim()),
    )])
}

fn help_row(key: &'static str, action: &'static str) -> Line<'static> {
    const KEY_WIDTH: usize = 16;
    let pad = KEY_WIDTH.saturating_sub(key.width());
    let mut key_cell = String::with_capacity(KEY_WIDTH + 2);
    key_cell.push_str("  ");
    key_cell.push_str(key);
    for _ in 0..pad {
        key_cell.push(' ');
    }
    Line::from(vec![
        Span::styled(key_cell, Style::default().fg(accent()).bold()),
        // Help modal: hotkey in ACCENT, action in TEXT (the action column is
        // primary text, not the dim secondary tier).
        Span::styled(action, Style::default().fg(text())),
    ])
}
