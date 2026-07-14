use crate::app::Banner;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::theme::blend;
use super::{accent, bg, danger, format_free_space, text_dim, warning, widgets};

const ACTION_DISK: &str = "[d] change output dir";
const TOO_SMALL_LABEL: &str = "terminal too small · enlarge for full layout";

/// Render a single banner row into `area`.
///
/// Each banner row carries a full-width semantic background wash — the
/// semantic color blended 25% over BG — applied to the `Paragraph` style and
/// every span so that the tint fills the entire row width.  The ` ! ` glyph
/// is fg(full semantic color) on that wash.  Label, separator, and action
/// hint are fg(text_dim()) on that same wash.  No bold anywhere.
pub fn render_banner(frame: &mut Frame, area: Rect, banner: &Banner) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let (color, label) = banner_style_and_text(banner);
    let wash = blend(color, bg(), 0.25);
    let glyph_style = Style::default().fg(color).bg(wash);
    let msg_style = Style::default().fg(text_dim()).bg(wash);

    let mut spans = vec![
        Span::styled(" ! ", glyph_style),
        Span::styled(label, msg_style),
    ];
    // Disk banners carry an action hint; the size banner has none.
    if banner_has_action(banner) {
        spans.push(Span::styled(" · ", msg_style));
        spans.extend(widgets::keyed_spans(
            ACTION_DISK,
            Style::default().fg(accent()).bold().bg(wash),
            msg_style,
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).bg(wash), area);
}

/// Whether a banner carries the `d change output dir` action hint.
fn banner_has_action(banner: &Banner) -> bool {
    matches!(banner, Banner::DiskFull { .. } | Banner::DiskLow { .. })
}

/// Split `area` into a banner strip (top) and the body area (rest).
///
/// When `banners` is empty the banner strip has height 0 and the body is the
/// full `area`. Only inserts rows for the actual number of banners so the body
/// is never unnecessarily shrunk.
pub(crate) fn split_banner_area(area: Rect, banners: &[Banner]) -> (Rect, Rect) {
    let n = banner_height(banners);
    if n == 0 {
        return (Rect { height: 0, ..area }, area);
    }
    let h = n.min(area.height);
    let banner_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: h,
    };
    let body_area = Rect {
        x: area.x,
        y: area.y + h,
        width: area.width,
        height: area.height.saturating_sub(h),
    };
    (banner_area, body_area)
}

/// Render one row per banner. Returns the number of rows consumed (= banners.len()).
pub fn render_banners(frame: &mut Frame, area: Rect, banners: &[Banner]) {
    if banners.is_empty() || area.height == 0 {
        return;
    }

    for (i, banner) in banners.iter().enumerate() {
        let row_y = area.y + i as u16;
        if row_y >= area.y + area.height {
            break;
        }
        let row = Rect {
            x: area.x,
            y: row_y,
            width: area.width,
            height: 1,
        };
        render_banner(frame, row, banner);
    }
}

/// How many rows `banners` will consume.
pub fn banner_height(banners: &[Banner]) -> u16 {
    banners.len() as u16
}

/// Returns `(semantic_color, label_text)`.
///
/// `semantic_color` is the full-saturation semantic color — `danger()` or
/// `warning()` — used both as the ` ! ` glyph fg and as the `a` argument to
/// [`blend`] when computing the wash background.
fn banner_style_and_text(banner: &Banner) -> (ratatui::style::Color, String) {
    match banner {
        Banner::DiskFull { free_bytes } => {
            let label = disk_label("disk critical", *free_bytes);
            (danger(), label)
        }
        Banner::DiskLow { free_bytes } => {
            let label = disk_label("disk low", *free_bytes);
            (warning(), label)
        }
        Banner::TooSmall => (warning(), TOO_SMALL_LABEL.to_string()),
    }
}

fn disk_label(kind: &str, free_bytes: u64) -> String {
    let mut s = String::with_capacity(32);
    s.push_str(kind);
    // Separate clauses with ` · ` (mid-dot), not a colon.
    s.push_str(" · ");
    s.push_str(&format_free_space(free_bytes));
    s
}
