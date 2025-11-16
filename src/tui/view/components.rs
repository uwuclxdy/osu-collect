use crate::{
    app::{InputField, ThreadStatusLine},
    download::DownloadStage,
};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{ListItem, Tabs},
};

use super::TabsView;

pub fn tab_bar(tabs: &TabsView) -> Tabs<'static> {
    let titles: Vec<Line> = tabs
        .titles()
        .iter()
        .map(|title| Line::from(Span::raw(title.clone())))
        .collect();

    Tabs::new(titles)
        .select(tabs.active())
        .divider(" | ")
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
}

pub fn input_item(field: &InputField, focused: bool) -> ListItem<'static> {
    let value = if field.value.is_empty() {
        Span::styled(
            field.placeholder.clone(),
            Style::default().fg(Color::DarkGray),
        )
    } else {
        Span::raw(field.value.clone())
    };

    let spans = vec![
        Span::styled(
            if focused { "> " } else { "  " },
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(
            format!("{}: ", field.label),
            Style::default().fg(Color::Gray),
        ),
        value,
    ];

    let style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    ListItem::new(Line::from(spans)).style(style)
}

pub fn toggle_item(label: &str, state: bool, focused: bool) -> ListItem<'static> {
    let marker = if state { "[x]" } else { "[ ]" };
    let style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let spans = vec![
        Span::styled(
            if focused { "> " } else { "  " },
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(marker, style),
        Span::raw(format!(" {label}")),
    ];
    ListItem::new(Line::from(spans)).style(style)
}

pub fn status_style(stage: DownloadStage) -> Style {
    match stage {
        DownloadStage::Pending | DownloadStage::Resolving | DownloadStage::Rechecking => {
            Style::default().fg(Color::Yellow)
        }
        DownloadStage::Downloading => Style::default().fg(Color::Cyan),
        DownloadStage::Completed => Style::default().fg(Color::Green),
        DownloadStage::Failed => Style::default().fg(Color::Red),
    }
}

pub fn thread_item(index: usize, status: &ThreadStatusLine) -> ListItem<'static> {
    let prefix = Span::styled(
        format!("Thread {}: ", index + 1),
        Style::default().fg(Color::Gray),
    );
    let line = Line::from(vec![
        prefix,
        Span::styled(status.message.clone(), thread_style(status)),
    ]);
    ListItem::new(line)
}

fn thread_style(status: &ThreadStatusLine) -> Style {
    if status.rate_limited {
        return Style::default().fg(Color::Yellow);
    }

    if status.message.to_lowercase().contains("error") || status.message.starts_with("Failed") {
        return Style::default().fg(Color::Red);
    }

    if status.message.starts_with("Done") {
        return Style::default().fg(Color::Green);
    }

    if status.message.starts_with("Skipped") {
        return Style::default().fg(Color::Magenta);
    }

    Style::default()
}
