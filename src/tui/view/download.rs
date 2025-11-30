use crate::{
    app::CollectionPage,
    download::{DownloadStage, DownloadSummary},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph, Wrap},
};

use super::{DownloadView, components};

pub fn render(frame: &mut Frame, area: Rect, view: DownloadView) {
    let page = view.page;
    let layout = Layout::vertical([
        Constraint::Length(6),
        Constraint::Length(3),
        Constraint::Min(0),
    ]);
    let [info_area, gauge_area, threads_area] = layout.areas(area);

    render_info(frame, info_area, page);
    render_gauge(frame, gauge_area, page);
    render_threads(frame, threads_area, page);
}

fn render_info(frame: &mut Frame, area: Rect, page: &CollectionPage) {
    let stage_label = match page.stage {
        DownloadStage::Pending => "Pending",
        DownloadStage::Resolving => "Resolving",
        DownloadStage::Rechecking => "Rechecking existing maps",
        DownloadStage::Downloading => "Downloading",
        DownloadStage::Completed => "Completed",
        DownloadStage::Failed => "Failed",
    };
    let status =
        if matches!(page.stage, DownloadStage::Downloading) && page.all_threads_rate_limited() {
            "Rate Limited"
        } else {
            stage_label
        };

    let speed_display = if matches!(page.stage, DownloadStage::Downloading) {
        let speed = page.cumulative_speed();
        if speed >= 1.0 {
            Some(format_speed(speed))
        } else {
            None
        }
    } else {
        None
    };

    let counts_line = |label: &str, downloaded: u16, skipped: u16, failed: u16, unverified: u16| {
        let displayed_skipped = skipped.saturating_add(unverified);
        let mut parts = vec![
            format!("{downloaded} downloaded"),
            format!("{displayed_skipped} skipped"),
            format!("{failed} failed"),
        ];
        if unverified > 0 {
            parts.push(format!("{unverified} unverified"));
        }
        format!(" {label}: {}", parts.join(" • "))
    };

    let summary_line = if let Some(summary) = &page.summary {
        counts_line(
            "Done",
            summary.downloaded,
            summary.skipped,
            summary.failed,
            summary.unverified,
        )
    } else {
        counts_line(
            "Progress",
            page.stats.downloaded,
            page.stats.skipped,
            page.stats.failed,
            page.stats.unverified,
        )
    };

    let bytes_display = if matches!(
        page.stage,
        DownloadStage::Downloading | DownloadStage::Completed
    ) && page.stats.bytes_total > 0
    {
        Some(format_bytes_progress(
            page.stats.bytes_downloaded,
            page.stats.bytes_total,
        ))
    } else {
        None
    };

    let mut status_spans = vec![
        Span::styled("Status: ", Style::default().fg(Color::Gray)),
        Span::styled(status, components::status_style(page.stage)),
    ];
    if let Some(ref speed) = speed_display {
        status_spans.push(Span::styled(" @ ", Style::default().fg(Color::Gray)));
        status_spans.push(Span::styled(
            speed.clone(),
            Style::default().fg(Color::Green),
        ));
    }
    if let Some(ref bytes) = bytes_display {
        status_spans.push(Span::styled(" (", Style::default().fg(Color::Gray)));
        status_spans.push(Span::styled(
            bytes.clone(),
            Style::default().fg(Color::Yellow),
        ));
        status_spans.push(Span::styled(")", Style::default().fg(Color::Gray)));
    }

    let lines = vec![
        Line::from(vec![
            Span::styled("Collection: ", Style::default().fg(Color::Gray)),
            Span::styled(page.title.clone(), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("Uploader: ", Style::default().fg(Color::Gray)),
            Span::raw(
                page.uploader
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string()),
            ),
        ]),
        Line::from(vec![
            Span::styled("Output: ", Style::default().fg(Color::Gray)),
            Span::raw(
                page.output_dir
                    .clone()
                    .unwrap_or_else(|| "Preparing...".to_string()),
            ),
        ]),
        Line::from(status_spans),
        Line::from(summary_line),
    ];

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Overview ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, area);
}

fn format_speed(bytes_per_sec: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;

    if bytes_per_sec >= MB {
        format!("{:.2} MB/s", bytes_per_sec / MB)
    } else if bytes_per_sec >= KB {
        format!("{:.1} KB/s", bytes_per_sec / KB)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

fn format_bytes_progress(downloaded: u64, total: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let downloaded_f = downloaded as f64;
    let total_f = total as f64;

    if total_f >= GB {
        format!("{:.2}/{:.2} GB", downloaded_f / GB, total_f / GB)
    } else if total_f >= MB {
        format!("{:.1}/{:.1} MB", downloaded_f / MB, total_f / MB)
    } else if total_f >= KB {
        format!("{:.0}/{:.0} KB", downloaded_f / KB, total_f / KB)
    } else {
        format!("{downloaded}/{total} B")
    }
}

fn render_gauge(frame: &mut Frame, area: Rect, page: &CollectionPage) {
    let queue_remaining = page.download_target;
    let total_collection = page.total_maps.max(1);
    let downloaded = page.stats.downloaded as usize;
    let verified = downloaded + page.stats.skipped as usize;
    let verified_display = verified.min(total_collection);
    let ratio = (verified_display as f64 / total_collection as f64).clamp(0.0, 1.0);

    let mut title_style = Style::default().fg(Color::LightGreen);
    if page.progress_label_style_locked {
        if page.progress_label_bold_when_locked {
            title_style = title_style.add_modifier(Modifier::BOLD);
        }
    } else {
        title_style = title_style.add_modifier(Modifier::BOLD);
    }

    let downloaded_title = format!(" Downloaded: {downloaded} In Queue: {queue_remaining} ");
    let verified_title = format!(" {verified_display}/{total_collection} maps verified ");

    let block = Block::default()
        .title(Line::from(vec![Span::styled(downloaded_title, title_style)]).centered())
        .title_bottom(
            Line::from(vec![Span::styled(
                verified_title,
                Style::default().fg(Color::Green),
            )])
            .centered(),
        )
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);

    let gauge = Gauge::default()
        .block(block)
        .ratio(ratio)
        .label(Span::raw(""))
        .gauge_style(Style::default().fg(Color::Cyan));

    frame.render_widget(gauge, area);
}

fn render_threads(frame: &mut Frame, area: Rect, page: &CollectionPage) {
    if matches!(page.stage, DownloadStage::Completed)
        && let Some(summary) = &page.summary
    {
        render_results_block(frame, area, summary);
        return;
    }

    let mut items: Vec<ListItem> = page
        .thread_statuses
        .iter()
        .enumerate()
        .map(|(idx, status)| components::thread_item(idx, status))
        .collect();

    if matches!(page.stage, DownloadStage::Completed | DownloadStage::Failed)
        && !page.failed_maps.is_empty()
    {
        let header = format!("Failed maps ({}):", page.failed_maps.len());
        let header_line = Line::from(vec![Span::styled(
            header,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]);
        items.push(ListItem::new(header_line));

        for chunk in page.failed_maps.chunks(5) {
            let joined = chunk
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let chunk_line = Line::from(vec![Span::styled(
                format!("  {joined}"),
                Style::default().fg(Color::Red),
            )]);
            items.push(ListItem::new(chunk_line));
        }
    }

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Threads ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .highlight_symbol("");
    frame.render_widget(list, area);
}

fn render_results_block(frame: &mut Frame, area: Rect, summary: &DownloadSummary) {
    let mut items: Vec<ListItem> = Vec::new();
    items.push(ListItem::new(Line::from(vec![Span::styled(
        "Collection Results",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )])));

    let displayed_skipped = summary.skipped.saturating_add(summary.unverified);
    let stats = [
        ("Downloaded", summary.downloaded, Style::default()),
        ("Skipped", displayed_skipped, Style::default()),
        ("Failed", summary.failed, Style::default()),
        (
            "Unverified",
            summary.unverified,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    for (label, value, style) in stats {
        let line = if label == "Unverified" && value == 0 {
            Line::from(format!("{label}: {value}"))
        } else if label == "Unverified" {
            Line::from(vec![Span::styled(format!("{label}: {value}"), style)])
        } else {
            Line::from(format!("{label}: {value}"))
        };
        items.push(ListItem::new(line));
    }

    let list = List::new(items).block(
        Block::default()
            .title(" Results ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded),
    );
    frame.render_widget(list, area);
}
