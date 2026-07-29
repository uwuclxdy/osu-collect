//! Downloads tab: a two-pane master-detail over every run — the run list on
//! the left (active runs, then past ones newest-first), a preview of the
//! highlighted run on the right. A live page previews through the full
//! [`super::download::render`] view (per-map status, failed section); a
//! history record gets a read-only summary panel. Pure view — selection and
//! pane focus live on `App.downloads_tab`.

use crate::app::{App, DownloadsRow, DownloadsTab, HistoryRecord, HistoryStage};
use crate::download::DownloadStage;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{ListItem, Paragraph, Wrap},
};
use std::time::{SystemTime, UNIX_EPOCH};

use super::widgets;
use super::{accent, danger, success, text, text_dim, text_faint, warning};

const PANEL_DOWNLOADS: &str = " DOWNLOADS ";

/// Geometry mirrors `master_detail`: below these the split collapses to the
/// focused pane.
const MIN_SPLIT_WIDTH: u16 = 60;
const COMPACT_HEIGHT: u16 = 14;
const LIST_WIDTH_MIN: u16 = 28;
const LIST_WIDTH_MAX: u16 = 52;

const GLYPH_ACTIVE: &str = "● ";
const GLYPH_PAST: &str = "○ ";
const CURSOR_CARET: &str = "❯ ";
const CURSOR_NONE: &str = "  ";

const EMPTY_LINE_1: &str = "no downloads yet";
const EMPTY_LINE_2: &str = "start one from the get maps tab";

pub fn render(frame: &mut Frame, area: Rect, app: &App, tick: u64) {
    let rows = app.downloads_rows();
    let view = &app.downloads_tab;

    if rows.is_empty() {
        render_empty(frame, area);
        return;
    }
    let selected = view.selected.min(rows.len() - 1);

    let wide = area.width >= MIN_SPLIT_WIDTH && area.height >= COMPACT_HEIGHT;
    if wide {
        // Divide before multiply (matches `master_detail`) so the intermediate
        // never overflows u16.
        let list_width = (area.width / 5 * 2).clamp(LIST_WIDTH_MIN, LIST_WIDTH_MAX);
        let [list_area, preview_area] =
            Layout::horizontal([Constraint::Length(list_width), Constraint::Min(0)]).areas(area);
        render_run_list(frame, list_area, &rows, selected, view);
        render_preview(frame, preview_area, &rows[selected], tick);
    } else if view.preview_focused {
        render_preview(frame, area, &rows[selected], tick);
    } else {
        render_run_list(frame, area, &rows, selected, view);
    }
}

fn render_empty(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(vec![Span::raw("  "), EMPTY_LINE_1.fg(text_dim())]),
        Line::from(vec![Span::raw("  "), EMPTY_LINE_2.fg(text_faint())]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(widgets::panel_block(PANEL_DOWNLOADS, None, true, true))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_run_list(
    frame: &mut Frame,
    area: Rect,
    rows: &[DownloadsRow<'_>],
    selected: usize,
    view: &DownloadsTab,
) {
    let focused = !view.preview_focused;
    let items: Vec<ListItem<'static>> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| row_item(row, i == selected, focused))
        .collect();
    widgets::render_scrollable_panel(
        frame,
        area,
        PANEL_DOWNLOADS,
        Some(counts_meta(rows)),
        items,
        selected,
        focused,
        None,
        focused,
        true,
        &view.list_offset,
    );
}

/// Title-right meta: `N active` (accent while non-zero).
fn counts_meta(rows: &[DownloadsRow<'_>]) -> Line<'static> {
    let active = rows
        .iter()
        .filter(|row| matches!(row, DownloadsRow::Page(page) if !page.is_settled()))
        .count();
    let active_color = if active > 0 { accent() } else { text_dim() };
    Line::from(format!("{active} active").fg(active_color))
}

fn row_item(row: &DownloadsRow<'_>, is_cursor: bool, list_focused: bool) -> ListItem<'static> {
    let caret = if is_cursor && list_focused {
        CURSOR_CARET.fg(accent())
    } else {
        Span::raw(CURSOR_NONE)
    };
    let label_style = if is_cursor {
        Style::default().fg(text()).bold()
    } else {
        Style::default().fg(text_dim())
    };

    let (glyph, title, suffix) = match row {
        DownloadsRow::Page(page) => {
            let glyph = Span::styled(
                if page.is_settled() {
                    GLYPH_PAST
                } else {
                    GLYPH_ACTIVE
                },
                widgets::status_style(page.stage),
            );
            (glyph, page.title.clone(), page_suffix(page))
        }
        // A settled record's state rides its `○` glyph color alone — the
        // done/failed/cancelled word is dropped from the row.
        DownloadsRow::Record(record) => (
            GLYPH_PAST.fg(record_color(record.stage)),
            record.title.clone(),
            None,
        ),
    };

    let mut spans = vec![caret, glyph, Span::styled(title, label_style)];
    // Only an in-progress run keeps a trailing count; a terminal state (settled
    // page or history record) reads its outcome from the glyph color.
    if let Some(suffix) = suffix {
        spans.push(Span::raw("  "));
        spans.push(suffix);
    }
    ListItem::new(Line::from(spans))
}

/// Short trailing status for a live page row: progress counts while running,
/// `None` once terminal (the `●`/`○` glyph color carries the outcome).
fn page_suffix(page: &crate::app::CollectionPage) -> Option<Span<'static>> {
    let done = page.stats.downloaded as usize + page.stats.skipped as usize;
    Some(match page.stage {
        DownloadStage::Pending | DownloadStage::Resolving => "resolving…".fg(text_faint()),
        DownloadStage::Rechecking => "rechecking".fg(warning()),
        DownloadStage::Downloading => format!("{done}/{}", page.total_maps).fg(text_dim()),
        DownloadStage::Completed | DownloadStage::Failed => return None,
    })
}

fn record_color(stage: HistoryStage) -> ratatui::style::Color {
    match stage {
        HistoryStage::Finished => success(),
        HistoryStage::Failed => danger(),
        HistoryStage::Cancelled => text_faint(),
    }
}

fn render_preview(frame: &mut Frame, area: Rect, row: &DownloadsRow<'_>, tick: u64) {
    match row {
        DownloadsRow::Page(page) => super::download::render(frame, area, page, tick),
        DownloadsRow::Record(record) => render_record_preview(frame, area, record),
    }
}

/// Read-only summary panel for a persisted past run — a proper-noun panel
/// title (the run's name, case preserved) over kv rows, mirroring the update
/// browse's preview idiom.
fn render_record_preview(frame: &mut Frame, area: Rect, record: &HistoryRecord) {
    // Static kv key column: TEXT_DIM + bold (cloudy-tui static key:value rows).
    let key_style = Style::default().fg(text_dim()).bold();
    let label_width = ["status", "downloaded", "skipped", "failed", "output"]
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);
    let kv = |key: &str, value: Span<'static>| {
        Line::from(vec![
            Span::styled(widgets::label_cell(key, label_width), key_style),
            value,
        ])
    };

    let mut status_line = vec![Span::styled(
        widgets::label_cell("status", label_width),
        key_style,
    )];
    status_line
        .extend(widgets::status_pill(record.stage.label(), record_color(record.stage)).spans);

    let failed_color = if record.failed > 0 {
        danger()
    } else {
        text_dim()
    };
    let mut lines = vec![
        Line::from(status_line),
        kv(
            "downloaded",
            format!("{}/{}", record.downloaded, record.total_maps).fg(text_dim()),
        ),
        kv("skipped", record.skipped.to_string().fg(text_dim())),
        kv("failed", record.failed.to_string().fg(failed_color)),
    ];
    if let Some(dir) = record.output_dir.as_deref() {
        lines.push(kv(
            "output",
            crate::utils::pretty_path(dir).into_owned().fg(text_dim()),
        ));
    }
    let title = format!(" {} ", record.title.to_uppercase());
    let age = Line::from(age_label(record.finished_at).fg(text_dim()));
    frame.render_widget(
        Paragraph::new(lines)
            .block(widgets::panel_block(title, Some(age), false, false))
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// Coarse relative age for a unix-seconds timestamp: `just now`, `5m ago`,
/// `3h ago`, `2d ago`, `3w ago`; an absolute ISO date (`2026-04-12`) past
/// 30 days.
fn age_label(finished_at: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ago = now.saturating_sub(finished_at);
    if ago < 60 {
        "just now".to_string()
    } else if ago < 3600 {
        format!("{}m ago", ago / 60)
    } else if ago < 86_400 {
        format!("{}h ago", ago / 3600)
    } else if ago < 7 * 86_400 {
        format!("{}d ago", ago / 86_400)
    } else if ago < 30 * 86_400 {
        format!("{}w ago", ago / (7 * 86_400))
    } else {
        time::OffsetDateTime::from_unix_timestamp(finished_at as i64).map_or_else(
            |_| format!("{}w ago", ago / (7 * 86_400)),
            |dt| {
                format!(
                    "{:04}-{:02}-{:02}",
                    dt.year(),
                    u8::from(dt.month()),
                    dt.day()
                )
            },
        )
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tui_downloads.rs"]
mod tests;
