use crate::{
    app::{CollectionPage, collection::FailureReason},
    download::{DownloadStage, DownloadSummary},
    utils::{format_bytes, pretty_path},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Gauge, List, ListItem, ListState, Paragraph, Wrap},
};

use super::widgets::{self, Metric, SEPARATOR};
use super::{
    FILL_BLOCK, FILL_SHADE, GLYPH_BLOCK, GLYPH_SHADE, accent, bg_raised, danger, glyph_fill, info,
    line, spinner_str, success, text_dim, text_faint, warning,
};

/// Tallest the gauge section ever gets: a one-row bar plus the two title rows the
/// block reserves during the recheck pass (top `rechecking` + bottom `verified`).
/// Every other stage uses one fewer row (see [`gauge_section_height`]).
const GAUGE_HEIGHT: u16 = 3;
/// Horizontal margin (columns) applied to each side of the progress bar.
const GAUGE_H_MARGIN: u16 = 1;

/// Vertical size of the gauge section for a stage. The colored bar fill is always
/// one row; the extra rows are title rows the block reserves — the bottom
/// `verified` title on every stage, plus a top `rechecking` title during recheck.
fn gauge_section_height(stage: DownloadStage) -> u16 {
    if matches!(stage, DownloadStage::Rechecking) {
        GAUGE_HEIGHT // one bar row + top + bottom title rows
    } else {
        GAUGE_HEIGHT - 1 // one bar row + bottom title row
    }
}

const PANEL_OVERVIEW: &str = " OVERVIEW ";
const PANEL_ACTIVE: &str = " ACTIVE ";
const PANEL_RESULTS: &str = " RESULTS ";
const PANEL_FAILED: &str = " FAILED ";

/// Fixed height of the results-summary panel when it shares the completed view
/// with the failed-maps disclosure. Fits the counts line, both outro lines, and
/// the panel borders; the failed list takes the remaining space below.
const RESULTS_SUMMARY_HEIGHT: u16 = 8;

const KEY_COLLECTION: &str = "collection";
const KEY_UPLOADER: &str = "uploader";
const KEY_OUTPUT: &str = "output";
const KEY_STATUS: &str = "status";
const KEY_SPEED: &str = "  speed ";
const KEY_ETA: &str = "  eta ";
const KEY_SIZE: &str = "  size ";

const VALUE_UNKNOWN: &str = "unknown";
const VALUE_PREPARING: &str = "preparing";

const STATUS_PENDING: &str = "pending";
const STATUS_RESOLVING: &str = "resolving";
const STATUS_RECHECKING: &str = "rechecking";
const STATUS_DOWNLOADING: &str = "downloading";
const STATUS_COMPLETED: &str = "completed";
const STATUS_FAILED: &str = "failed";

const ACTIVE_VERIFYING: &str = "verifying existing archives…";
const ACTIVE_FETCHING: &str = "fetching collection metadata…";
const ACTIVE_NONE: &str = "no active threads";
const PLACEHOLDER_PREPARING: &str = "preparing";
const PLACEHOLDER_RESOLVING: &str = "resolving collection";

const FAILED_SECTION_LABEL: &str = "failed";

const RESULTS_DOWNLOADED: &str = "downloaded";
const RESULTS_SKIPPED: &str = "skipped";
const RESULTS_FAILED: &str = "failed";
const RESULTS_UNVERIFIED: &str = "unverified";
const RESULTS_OUTRO_1: &str =
    "how to import into osu!: https://github.com/uwuclxdy/osu-collect#importing-into-osu";
const RESULTS_OUTRO_2: &str = "and leave a star while you're at it :3";

const COMPACT_ACTIVE: &str = "active: ";
const COMPACT_FAILED: &str = " failed: ";

pub fn render(frame: &mut Frame, area: Rect, page: &CollectionPage, tick: u64) {
    if area.height < super::COMPACT_HEIGHT {
        render_compact(frame, area, page, tick);
        return;
    }
    // Low/full-disk surfaces as the system-wide Banner (top of body, every tab),
    // not an inline line here — see `tui::draw` / `app::system_banners`.
    //
    // The OVERVIEW panel sizes to its content so a live download (no settings /
    // tally rows) sits 2 lines shorter than a settled stage. The gauge collapses
    // to nothing once complete — the RESULTS panel already carries the tally.
    let info_lines = overview_lines(page);
    let info_height = (info_lines.len() as u16).saturating_add(2); // rounded top + bottom border
    let gauge_height = if matches!(page.stage, DownloadStage::Completed) {
        0
    } else {
        gauge_section_height(page.stage)
    };
    let [overview_area, gauge_area, threads_area] = Layout::vertical([
        Constraint::Length(info_height),
        Constraint::Length(gauge_height),
        Constraint::Min(0),
    ])
    .areas(area);
    render_overview(frame, overview_area, info_lines);
    if gauge_height > 0 {
        render_gauge(frame, gauge_area, page, tick);
    }
    render_threads(frame, threads_area, page);
}

/// Compact render: overall gauge + active-download count + failed count.
///
/// Per-row breakdown, failed-maps collapsible, and session ETA are hidden.
/// The gauge alone tells the user "is it making progress."
fn render_compact(frame: &mut Frame, area: Rect, page: &CollectionPage, tick: u64) {
    let [gauge_area, summary_area] = Layout::vertical([
        Constraint::Length(gauge_section_height(page.stage)),
        Constraint::Min(0),
    ])
    .areas(area);
    render_gauge(frame, gauge_area, page, tick);

    let active_count = page
        .active_downloads
        .iter()
        .flatten()
        .filter(|l| !l.stage.is_terminal())
        .count();
    let failed = page.stats.failed;

    let key_style = Style::default().fg(text_faint());
    let line = Line::from(vec![
        Span::styled(COMPACT_ACTIVE, key_style),
        Span::styled(active_count.to_string(), Style::default().fg(text_dim())),
        Span::styled(COMPACT_FAILED, key_style),
        Span::styled(
            failed.to_string(),
            if failed > 0 {
                Style::default().fg(danger())
            } else {
                Style::default().fg(text_dim())
            },
        ),
    ]);

    frame.render_widget(Paragraph::new(line), summary_area);
}

/// Build the OVERVIEW panel's content lines. The panel height is derived from
/// the line count, so each stage shows only what it needs:
/// - rows: `collection`, `uploader`, `output`, `status` (+ inline speed/eta/size).
///
/// The `downloaded · queued · skipped · failed` tally rides the gauge's bottom
/// title row (see [`render_gauge`]), not here.
fn overview_lines(page: &CollectionPage) -> Vec<Line<'_>> {
    let rate_limited =
        matches!(page.stage, DownloadStage::Downloading) && page.fully_rate_limited();
    let status = if rate_limited {
        crate::config::constants::status::RATE_LIMITED
    } else {
        stage_label(page.stage)
    };

    let speed = current_speed(page);
    let bytes = bytes_display(page);

    // First-column kv keys are static labels: TEXT_DIM + bold (cloudy-tui). The
    // inline speed/eta/size sub-labels sit mid-line, so they stay recessive faint.
    let key_style = Style::default().fg(text_dim()).bold();
    let sub_key_style = Style::default().fg(text_faint());
    let value_style = Style::default().fg(text_dim());

    // Column-align the four key-value rows: every value starts at the same
    // column (widest label + ≥2 spaces, no colon).
    let label_width = [KEY_COLLECTION, KEY_UPLOADER, KEY_OUTPUT, KEY_STATUS]
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    let mut status_spans = vec![Span::styled(
        widgets::label_cell(KEY_STATUS, label_width),
        key_style,
    )];
    status_spans.extend(widgets::status_pill(status, status_color(page.stage, rate_limited)).spans);
    if let Some(speed) = speed {
        status_spans.push(Span::styled(KEY_SPEED, sub_key_style));
        status_spans.push(Span::styled(speed, Style::default().fg(success())));
        // ETA sits next to speed (both derived from cumulative_speed) so the
        // gauge can drop the duplicate — every figure lives in exactly one place.
        if let Some(eta) = session_eta(page) {
            status_spans.push(Span::styled(KEY_ETA, sub_key_style));
            status_spans.push(Span::styled(eta, Style::default().fg(accent())));
        }
    }
    if let Some(bytes) = bytes {
        status_spans.push(Span::styled(KEY_SIZE, sub_key_style));
        status_spans.push(Span::styled(bytes, Style::default().fg(warning())));
    }

    let mut lines = vec![
        Line::from(vec![
            Span::styled(widgets::label_cell(KEY_COLLECTION, label_width), key_style),
            Span::styled(page.title.as_str(), Style::default().fg(accent())),
        ]),
        Line::from(vec![
            Span::styled(widgets::label_cell(KEY_UPLOADER, label_width), key_style),
            Span::styled(
                page.uploader.as_deref().unwrap_or(VALUE_UNKNOWN),
                value_style,
            ),
        ]),
        Line::from(vec![
            Span::styled(widgets::label_cell(KEY_OUTPUT, label_width), key_style),
            Span::styled(
                page.output_dir
                    .as_deref()
                    .map(|p| pretty_path(p).into_owned())
                    .unwrap_or_else(|| VALUE_PREPARING.to_string()),
                value_style,
            ),
        ]),
    ];
    lines.push(Line::from(status_spans));
    lines
}

/// The `downloaded │ queued │ skipped │ failed` tally rendered left-aligned on
/// the gauge's bottom title row, opposite the right-aligned `verified` count (see
/// [`render_gauge`]). Separators recede in `line()`; each count sits in its
/// semantic color tier.
fn tally_line(page: &CollectionPage) -> Line<'static> {
    let downloaded = page.stats.downloaded as usize;
    let queued = page.download_target;
    let skipped = page.stats.skipped as usize;
    let failed = page.stats.failed as usize;
    let sep = || Span::styled(SEPARATOR, Style::default().fg(line()));
    let failed_color = if failed > 0 { danger() } else { text_dim() };
    Line::from(vec![
        Span::styled(
            format!("{downloaded} downloaded"),
            Style::default().fg(success()),
        ),
        sep(),
        Span::styled(format!("{queued} queued"), Style::default().fg(text_dim())),
        sep(),
        Span::styled(
            format!("{skipped} skipped"),
            Style::default().fg(text_dim()),
        ),
        sep(),
        Span::styled(
            format!("{failed} failed"),
            Style::default().fg(failed_color),
        ),
    ])
}

fn render_overview(frame: &mut Frame, area: Rect, lines: Vec<Line<'_>>) {
    frame.render_widget(
        Paragraph::new(lines)
            .block(widgets::panel_block(PANEL_OVERVIEW, None, false, true))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn stage_label(stage: DownloadStage) -> &'static str {
    match stage {
        DownloadStage::Pending => STATUS_PENDING,
        DownloadStage::Resolving => STATUS_RESOLVING,
        DownloadStage::Rechecking => STATUS_RECHECKING,
        DownloadStage::Downloading => STATUS_DOWNLOADING,
        DownloadStage::Completed => STATUS_COMPLETED,
        DownloadStage::Failed => STATUS_FAILED,
    }
}

fn current_speed(page: &CollectionPage) -> Option<String> {
    if !matches!(page.stage, DownloadStage::Downloading) {
        return None;
    }
    let speed = page.cumulative_speed();
    (speed >= 1.0).then(|| format_bytes(speed as u64, "B/s"))
}

fn bytes_display(page: &CollectionPage) -> Option<String> {
    if !matches!(
        page.stage,
        DownloadStage::Downloading | DownloadStage::Completed
    ) {
        return None;
    }
    page.stats.total_collection_bytes.map(|total| {
        format!(
            "{}/{}",
            format_bytes(page.total_downloaded_bytes(), "B"),
            format_bytes(total, "B"),
        )
    })
}

fn status_color(stage: DownloadStage, rate_limited: bool) -> Color {
    if rate_limited {
        warning()
    } else {
        widgets::status_style(stage).fg.unwrap_or(text_dim())
    }
}

/// Returns the session ETA label, or `None` if there is not yet enough data
/// (speed < 1 B/s or no total size known). Speed comes from `cumulative_speed()`
/// — the same rolling average shown in the OVERVIEW panel — so the ETA derived
/// here always agrees with the displayed speed.
fn session_eta(page: &CollectionPage) -> Option<String> {
    let speed = page.cumulative_speed();
    if speed < 1.0 {
        return None;
    }
    let total = page.stats.total_collection_bytes?;
    let bytes_done = page.stats.bytes_downloaded;
    let remaining = total.saturating_sub(bytes_done);
    let eta_secs = (remaining as f64 / speed) as u64;
    Some(format_eta(eta_secs))
}

/// Format a duration in seconds as a compact human label: `45s`, `2m 30s`, `1h 12m`.
pub(crate) fn format_eta(secs: u64) -> String {
    if secs < 60 {
        let mut s = secs.to_string();
        s.push('s');
        s
    } else if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}m {s:02}s")
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}h {m:02}m")
    }
}

fn render_gauge(frame: &mut Frame, area: Rect, page: &CollectionPage, tick: u64) {
    // Inset the bar by GAUGE_H_MARGIN on each side; titles stay at the outer edge.
    let bar_area = area.inner(Margin::new(GAUGE_H_MARGIN, 0));

    if matches!(
        page.stage,
        DownloadStage::Pending | DownloadStage::Resolving
    ) {
        if let Some((current, total)) = page.resolve_progress
            && total > 0
        {
            render_resolve_progress_gauge(frame, bar_area, current, total, tick);
        } else {
            render_indeterminate_gauge(frame, bar_area, page.stage, tick);
        }
        return;
    }

    let is_rechecking = matches!(page.stage, DownloadStage::Rechecking);
    let total_collection = page.total_maps.max(1);
    let downloaded = page.stats.downloaded as usize;
    let failed = page.stats.failed as usize;
    let verified = downloaded + page.stats.skipped as usize;
    let verified_display = verified.min(total_collection);
    let failed_display = failed.min(total_collection.saturating_sub(verified_display));
    let verified_ratio = (verified_display as f64 / total_collection as f64).clamp(0.0, 1.0);
    let failed_ratio = (failed_display as f64 / total_collection as f64).clamp(0.0, 1.0);

    // The bottom title row carries the downloaded · queued · skipped · failed
    // tally on the left and the verified count on the right. The tally wins the
    // row: the verified count is dropped when the two would collide on a narrow
    // terminal. While downloading the gauge carries no top title; only the
    // rechecking stage keeps its own progress title.
    let tally = tally_line(page);
    let verified_title = format!(" {verified_display}/{total_collection} verified ");
    let fits_both = tally.width() + verified_title.chars().count() + 2 <= bar_area.width as usize;

    let mut block = Block::default().title_bottom(tally.left_aligned());
    if fits_both {
        block = block.title_bottom(
            Line::from(Span::styled(
                verified_title,
                Style::default().fg(text_faint()),
            ))
            .right_aligned(),
        );
    }
    if is_rechecking {
        let style = Style::default().fg(warning()).bold();
        block = block.title(
            Line::from(Span::styled(
                format!(" rechecking {verified_display}/{total_collection} "),
                style,
            ))
            .left_aligned(),
        );
    }

    let fill_color = if is_rechecking { warning() } else { accent() };

    if failed_display == 0 {
        frame.render_widget(
            Gauge::default()
                .block(block)
                .ratio(verified_ratio)
                .label(Span::raw(""))
                .gauge_style(Style::default().fg(fill_color).bg(bg_raised())),
            bar_area,
        );
        return;
    }

    let inner = block.inner(bar_area);
    frame.render_widget(block, bar_area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let bar_width = inner.width as usize;
    let verified_cells = ((verified_ratio * bar_width as f64).round() as usize).min(bar_width);
    let failed_cells =
        ((failed_ratio * bar_width as f64).round() as usize).min(bar_width - verified_cells);
    let empty_cells = bar_width.saturating_sub(verified_cells + failed_cells);

    let bar = Line::from(vec![
        Span::styled(
            glyph_fill(&FILL_BLOCK, GLYPH_BLOCK, verified_cells).into_owned(),
            Style::default().fg(fill_color),
        ),
        Span::styled(
            glyph_fill(&FILL_BLOCK, GLYPH_BLOCK, failed_cells).into_owned(),
            Style::default().fg(danger()),
        ),
        Span::styled(
            glyph_fill(&FILL_SHADE, GLYPH_SHADE, empty_cells).into_owned(),
            Style::default().fg(bg_raised()),
        ),
    ]);
    frame.render_widget(Paragraph::new(bar), inner);
}

fn render_indeterminate_gauge(frame: &mut Frame, area: Rect, stage: DownloadStage, tick: u64) {
    let spinner = spinner_str(tick).trim();
    let label = match stage {
        DownloadStage::Pending => PLACEHOLDER_PREPARING,
        _ => PLACEHOLDER_RESOLVING,
    };
    let title = format!(" {spinner} {label} ");
    render_indeterminate_block(frame, area, &title);
}

fn render_resolve_progress_gauge(
    frame: &mut Frame,
    area: Rect,
    current: u32,
    total: u32,
    tick: u64,
) {
    let spinner = spinner_str(tick).trim();
    let title = format!(" {spinner} resolving {current}/{total} collections ");
    let title_style = Style::default().fg(info()).bold();
    let block = Block::default().title(Line::from(Span::styled(title, title_style)).left_aligned());

    let inner = block.inner(area);
    if inner.width == 0 || inner.height == 0 {
        frame.render_widget(block, area);
        return;
    }

    let ratio = if total == 0 {
        0.0
    } else {
        (current as f64 / total as f64).clamp(0.0, 1.0)
    };
    // Determinate progress: sub-cell unicode fill, no centered percent label.
    frame.render_widget(
        Gauge::default()
            .block(block)
            .use_unicode(true)
            .ratio(ratio)
            .label(Span::raw(""))
            .gauge_style(Style::default().fg(info()).bg(bg_raised())),
        area,
    );
}

fn render_indeterminate_block(frame: &mut Frame, area: Rect, title: &str) {
    let title_style = Style::default().fg(info()).bold();
    let block = Block::default().title(Line::from(Span::styled(title, title_style)).left_aligned());

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // No known total: the canonical bracketed bouncing-block bar ([ ████░░░░ ]),
    // the same widget the per-row mini-bar uses, sized to the full panel width.
    // It signals live work without claiming determinate progress.
    let bar = Line::from(widgets::indeterminate_bar_spans(inner.width, info()));
    frame.render_widget(Paragraph::new(bar), inner);
}

fn render_threads(frame: &mut Frame, area: Rect, page: &CollectionPage) {
    if matches!(page.stage, DownloadStage::Completed)
        && let Some(summary) = &page.summary
    {
        // No failures: the results block is the sole content panel, so it owns
        // focus (LINE_STRONG). With failures, the FAILED list is the navigable
        // section (↑↓ move, r/R retry, enter expand) — it takes focus while the
        // results summary recedes to a blurred read-only panel.
        if page.failed_maps.is_empty() {
            render_results_block(frame, area, summary, true);
        } else {
            let [summary_area, failed_area] = Layout::vertical([
                Constraint::Length(RESULTS_SUMMARY_HEIGHT),
                Constraint::Min(0),
            ])
            .areas(area);
            render_results_block(frame, summary_area, summary, false);
            render_failed_section(frame, failed_area, page);
        }
        return;
    }

    let block = widgets::panel_block(PANEL_ACTIVE, None, true, false);
    let inner = block.inner(area);

    let row_width = inner.width;
    let mut items: Vec<ListItem> = Vec::new();

    if matches!(page.stage, DownloadStage::Downloading) {
        // Single pass: collect non-rate-limited and rate-limited rows separately,
        // then emit non-rate-limited first and rate-limited last with no separator.
        // Each rate-limited row already carries warning() bar color + a per-row
        // countdown, so no divider is needed as a third signal.
        let mut normal: Vec<ListItem> = Vec::new();
        let mut throttled: Vec<ListItem> = Vec::new();

        // Only rows that are actively downloading render — empty (not-started)
        // slots are skipped and finished rows were already freed, so nothing that
        // isn't downloading appears in the list.
        for line in page.active_downloads.iter().flatten() {
            if line.displayed_rate_limited() {
                throttled.push(rate_limited_item(line, row_width));
            } else {
                normal.push(widgets::active_download_item(line, row_width));
            }
        }

        items.extend(normal);
        items.extend(throttled);
    } else if items.is_empty() && page.failed_maps.is_empty() {
        let (text, color) = match page.stage {
            DownloadStage::Rechecking => (ACTIVE_VERIFYING, warning()),
            DownloadStage::Pending | DownloadStage::Resolving => (ACTIVE_FETCHING, info()),
            _ => (ACTIVE_NONE, text_faint()),
        };
        items.push(ListItem::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(text, Style::default().fg(color)),
        ])));
    }

    if !page.failed_maps.is_empty() {
        push_failed_rows(&mut items, page);
    }

    render_scrollable_panel(frame, area, block, inner, items, page);
}

/// Render the `FAILED (n)` disclosure as a standalone scrollable panel.
///
/// Used in the completed view (alongside the results summary) so the failed
/// list stays expandable and navigable post-completion. Shares the same
/// `thread_scroll` / `thread_total_items` / `thread_visible_items` bookkeeping
/// as the ACTIVE panel, so ↑↓ navigation and `r`/`R` retry keep working.
fn render_failed_section(frame: &mut Frame, area: Rect, page: &CollectionPage) {
    // Focused (LINE_STRONG): on the completed view this is the section the
    // cursor lives in (↑↓ navigate the failed rows, r/R retries).
    let block = widgets::panel_block(PANEL_FAILED, None, true, false);
    let inner = block.inner(area);
    let mut items: Vec<ListItem> = Vec::new();
    push_failed_rows(&mut items, page);
    render_scrollable_panel(frame, area, block, inner, items, page);
}

/// Clamp `thread_scroll` to the item count, attach a scroll indicator, and draw
/// the visible window. Records `thread_total_items` / `thread_visible_items` so
/// the app-side scroll/nav keybinds operate on the same bounds.
fn render_scrollable_panel(
    frame: &mut Frame,
    area: Rect,
    block: Block<'static>,
    inner: Rect,
    items: Vec<ListItem<'static>>,
    page: &CollectionPage,
) {
    let visible_height = inner.height as usize;
    let total = items.len();
    page.thread_total_items.set(total);
    page.thread_visible_items.set(visible_height);

    let max_scroll = total.saturating_sub(visible_height);
    let start = page.thread_scroll.min(max_scroll);

    frame.render_widget(block, area);

    // Offset-driven scroll with no selection: the failed list scrolls by
    // `thread_scroll`, it has no per-row cursor highlight.
    let mut state = ListState::default().with_offset(start);
    frame.render_stateful_widget(List::new(items).highlight_symbol(""), inner, &mut state);
    widgets::render_scrollbar(frame, inner, start, total);
}

/// Append the `FAILED (n)` disclosure header and, when expanded, one row per
/// failed beatmapset. Shared by the ACTIVE panel and the completed-view panel
/// so both render identical rows.
fn push_failed_rows(items: &mut Vec<ListItem<'static>>, page: &CollectionPage) {
    let count = page.failed_maps.len();
    let detail = format!("({count})");
    items.push(widgets::disclosure_row(
        FAILED_SECTION_LABEL,
        detail,
        page.failed_section_expanded,
        false,
        count > 0,
        // Standalone header — no form group to column-align with.
        0,
    ));
    if page.failed_section_expanded {
        for failure in &page.failed_maps {
            let (reason_label, reason_color) = failure_display(failure.reason);
            let id_str = format!("#{}", failure.beatmapset_id);
            let title_part = failure
                .title
                .as_deref()
                .map(|t| format!(" · {t}"))
                .unwrap_or_default();
            items.push(ListItem::new(Line::from(vec![
                Span::raw("    "),
                Span::styled(id_str, Style::default().fg(text_faint())),
                Span::styled(title_part, Style::default().fg(text_faint())),
                Span::raw("  "),
                Span::styled(reason_label, Style::default().fg(reason_color)),
            ])));
        }
    }
}

fn render_results_block(frame: &mut Frame, area: Rect, summary: &DownloadSummary, focused: bool) {
    let failed_color = if summary.failed > 0 {
        danger()
    } else {
        text_dim()
    };
    let mut metrics = vec![
        Metric::colored(RESULTS_DOWNLOADED, summary.downloaded.to_string(), accent()),
        Metric::colored(RESULTS_SKIPPED, summary.skipped.to_string(), text_dim()),
        Metric::colored(RESULTS_FAILED, summary.failed.to_string(), failed_color),
    ];
    if summary.unverified > 0 {
        metrics.push(Metric::colored(
            RESULTS_UNVERIFIED,
            summary.unverified.to_string(),
            warning(),
        ));
    }

    let lines = vec![
        widgets::summary_line(&metrics),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(RESULTS_OUTRO_1, Style::default().fg(text_dim())),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(RESULTS_OUTRO_2, Style::default().fg(text_faint())),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(widgets::panel_block(PANEL_RESULTS, None, focused, false))
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// Builds a list row for a rate-limited download slot.
///
/// The base message (from `events::emit_status`) ends at `...waiting` with no
/// number; this is the ONLY place the live `Ns` countdown is appended, so the
/// seconds always tick down (a number baked into the debounced base message
/// would freeze). When no deadline is recorded the count falls back to `0s` so
/// the row never shows a dangling `waiting` with no value.
fn rate_limited_item(
    line: &crate::app::collection::ActiveDownloadLine,
    width: u16,
) -> ListItem<'static> {
    let msg = rate_limited_message(
        &line.displayed_message(),
        line.cooldown_secs_remaining().unwrap_or(0),
    );
    widgets::active_download_item_msg(line, &msg, width)
}

/// Compose the rate-limited row text: the base message (ending at `...waiting`)
/// plus the single live ` {secs}s` countdown. The only place the seconds are
/// appended, so there is exactly one live-updating number.
fn rate_limited_message(base: &str, secs: u64) -> String {
    let s = secs.to_string();
    let mut msg = String::with_capacity(base.len() + 1 + s.len() + 1);
    msg.push_str(base);
    msg.push(' ');
    msg.push_str(&s);
    msg.push('s');
    msg
}

/// Returns `(display_label, color)` for a failure reason.
///
/// Transient errors (`NetworkError`, `RateLimited`) use warning color; definitive
/// failures use danger color.
fn failure_display(reason: FailureReason) -> (&'static str, ratatui::style::Color) {
    match reason {
        FailureReason::NetworkError | FailureReason::RateLimited => (reason.label(), warning()),
        FailureReason::NotFound | FailureReason::ValidationFailed | FailureReason::Unknown => {
            (reason.label(), danger())
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tui_download.rs"]
mod tests;
