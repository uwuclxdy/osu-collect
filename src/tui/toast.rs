//! Toast rendering: a floating stack anchored to the top-right.
//!
//! Each toast is borderless: a 1-cell semantic `┃` bar, then content on a
//! semi-transparent surface (`BG_SUNKEN` blended at 75 % over the cells
//! beneath). Toasts render last in the frame so the buffer below is final.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
};

use super::theme::blend;
use super::widgets::truncate_to_width;
use super::{bg, bg_sunken, danger, info, success, text, text_dim, warning};
use crate::app::{ToastLevel, Toasts};

/// Margin from the top and right screen edges.
const TOP_INSET: u16 = 2;
const RIGHT_INSET: u16 = 2;
/// Max box width — wide enough for the app's longest toast copy, still bounded
/// so a toast never spans a wide terminal. Collapses on narrow terminals.
const MAX_WIDTH: u16 = 60;
/// bar (1) + a 1-cell pad on each side of the content.
const CHROME_WIDTH: u16 = 3;
/// Column where content starts inside the box: bar (1) + left pad (1).
const CONTENT_OFFSET: u16 = 2;
/// Max wrapped detail lines. The toast stays ≤ 3 rows (title + 2 detail), so a
/// long context line wraps instead of hard-truncating to one line.
const MAX_DETAIL_LINES: usize = 2;
/// Heavy vertical left-bar.
const BAR: &str = "┃";
/// Opacity of the sunken surface over whatever sits beneath.
const BLEND_RATIO: f32 = 0.75;

/// Render the toast stack into the full-screen `area`. Newest sits on top.
pub fn render_toasts(frame: &mut Frame, area: Rect, toasts: &Toasts) {
    if toasts.is_empty() || area.width < RIGHT_INSET + CHROME_WIDTH || area.height <= TOP_INSET {
        return;
    }

    let max_width = MAX_WIDTH.min(area.width.saturating_sub(RIGHT_INSET * 2));
    let content_budget = max_width.saturating_sub(CHROME_WIDTH);
    if content_budget == 0 {
        return;
    }

    let bottom = area.y + area.height;
    let mut y = area.y + TOP_INSET;
    for toast in toasts.iter().rev() {
        let (title, title_w) = truncate_to_width(toast.title(), content_budget);
        let detail = toast
            .detail()
            .map(|line| wrap_to_width(line, content_budget, MAX_DETAIL_LINES))
            .unwrap_or_default();
        let detail_w = detail.iter().map(|(_, w)| *w).max().unwrap_or(0);
        let content_w = title_w.max(detail_w);
        let box_w = content_w + CHROME_WIDTH;
        let box_h = 1 + detail.len() as u16;
        if y + box_h > bottom {
            break; // out of vertical room — drop the rest
        }

        let rect = Rect {
            x: area.x + area.width - RIGHT_INSET - box_w,
            y,
            width: box_w,
            height: box_h,
        };
        draw_toast(frame, rect, toast.level(), &title, &detail);
        y += box_h;
    }
}

fn draw_toast(
    frame: &mut Frame,
    rect: Rect,
    level: ToastLevel,
    title: &str,
    detail: &[(String, u16)],
) {
    blend_surface(frame, rect);

    let bar_style = Style::default().fg(bar_color(level));
    let title_style = Style::default().fg(text()).bold();
    let detail_style = Style::default().fg(text_dim());
    let buf = frame.buffer_mut();

    buf.set_string(rect.x, rect.y, BAR, bar_style);
    buf.set_string(rect.x + CONTENT_OFFSET, rect.y, title, title_style);
    for (i, (line, _)) in detail.iter().enumerate() {
        let ly = rect.y + 1 + i as u16;
        buf.set_string(rect.x, ly, BAR, bar_style);
        buf.set_string(rect.x + CONTENT_OFFSET, ly, line, detail_style);
    }
}

/// Word-wrap `text` to `budget` columns across at most `max_lines` lines. When
/// content remains past the last allowed line, that line ends with `…`, so a
/// long detail keeps as much context as fits rather than collapsing to a single
/// hard-truncated line. Each entry is `(line, display_width)`.
fn wrap_to_width(text: &str, budget: u16, max_lines: usize) -> Vec<(String, u16)> {
    use unicode_width::UnicodeWidthStr as _;

    if budget == 0 || max_lines == 0 {
        return Vec::new();
    }
    let cap = budget as usize;
    let mut lines: Vec<(String, u16)> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;

    let mut words = text.split_whitespace().peekable();
    while let Some(&word) = words.peek() {
        if lines.len() == max_lines {
            break;
        }
        let ww = word.width();
        let sep = usize::from(!cur.is_empty());
        if cur_w + sep + ww <= cap {
            if sep == 1 {
                cur.push(' ');
            }
            cur.push_str(word);
            cur_w += sep + ww;
            words.next();
        } else if cur.is_empty() {
            // A single word wider than the whole line: hard-truncate it onto its
            // own line and move on.
            lines.push(truncate_to_width(word, budget));
            words.next();
        } else {
            // Flush the packed line; the word retries on a fresh one.
            lines.push((std::mem::take(&mut cur), cur_w as u16));
            cur_w = 0;
        }
    }
    if lines.len() < max_lines && !cur.is_empty() {
        lines.push((std::mem::take(&mut cur), cur_w as u16));
    }
    // Any unconsumed words (or a buffer left at the line cap) mean we ran out of
    // rows: mark the last visible line so the cut is honest.
    if (words.peek().is_some() || !cur.is_empty())
        && let Some((last, w)) = lines.last_mut()
    {
        ellipsize(last, w, budget);
    }
    lines
}

/// Ensure `line` ends with `…` within `budget`, updating `*w` to the new width.
fn ellipsize(line: &mut String, w: &mut u16, budget: u16) {
    use unicode_width::UnicodeWidthStr as _;

    if line.ends_with('…') {
        return;
    }
    if *w >= budget {
        // No spare column — cut one so the ellipsis fits (truncate appends it).
        *line = truncate_to_width(line, budget.saturating_sub(1).max(1)).0;
    } else {
        line.push('…');
    }
    *w = line.width() as u16;
}

/// Tint every cell of `rect` toward `BG_SUNKEN` at [`BLEND_RATIO`] over whatever
/// sits beneath, and clear the glyph so the surface reads as glass, not ghost
/// text. Content written afterward sets `fg` only, so the blend shows through.
fn blend_surface(frame: &mut Frame, rect: Rect) {
    let base = bg();
    let sunken = bg_sunken();
    let buf = frame.buffer_mut();
    for cy in rect.y..rect.y + rect.height {
        for cx in rect.x..rect.x + rect.width {
            if let Some(cell) = buf.cell_mut((cx, cy)) {
                let under = match cell.bg {
                    Color::Reset => base,
                    other => other,
                };
                cell.set_symbol(" ");
                cell.bg = blend(sunken, under, BLEND_RATIO);
                cell.fg = Color::Reset;
            }
        }
    }
}

fn bar_color(level: ToastLevel) -> Color {
    match level {
        ToastLevel::Success => success(),
        ToastLevel::Info => info(),
        ToastLevel::Warning => warning(),
        ToastLevel::Danger => danger(),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tui_toast.rs"]
mod tests;
