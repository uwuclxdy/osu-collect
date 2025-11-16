use crate::{
    app::{ConfigField, ConfigTab, MessageKind},
    config::{LogFormat, LogLevel},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, List, Paragraph, Wrap},
};

use super::{ConfigView, components};

pub fn render(frame: &mut Frame, area: Rect, view: ConfigView) {
    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(area);
    render_form(frame, chunks[0], view.form);
    render_message(frame, chunks[1], view);
}

fn render_form(frame: &mut Frame, area: Rect, form: &ConfigTab) {
    let items = vec![
        components::toggle_item(
            "Default: use Nerinyan",
            form.nerinyan,
            form.focus == ConfigField::MirrorNerinyan,
        ),
        components::toggle_item(
            "Default: use Catboy Central",
            form.catboy_central,
            form.focus == ConfigField::MirrorCatboyCentral,
        ),
        components::toggle_item(
            "Default: use Catboy US",
            form.catboy_us,
            form.focus == ConfigField::MirrorCatboyUs,
        ),
        components::toggle_item(
            "Default: use Catboy Asia",
            form.catboy_asia,
            form.focus == ConfigField::MirrorCatboyAsia,
        ),
        components::toggle_item(
            "Default: use osu.direct",
            form.osu_direct,
            form.focus == ConfigField::MirrorOsuDirect,
        ),
        components::toggle_item(
            "Default: use Sayobot",
            form.sayobot,
            form.focus == ConfigField::MirrorSayobot,
        ),
        components::input_item(
            &form.custom_mirror,
            form.focus == ConfigField::MirrorCustomUrl,
        ),
        components::toggle_item(
            "Default: skip existing files",
            form.skip_existing,
            form.focus == ConfigField::DownloadSkipExisting,
        ),
        components::input_item(&form.threads, form.focus == ConfigField::DownloadThreads),
        components::toggle_item(
            "Default: download without video",
            form.no_video,
            form.focus == ConfigField::DownloadNoVideo,
        ),
        components::toggle_item(
            "Enable logging",
            form.logging_enabled,
            form.focus == ConfigField::LoggingEnabled,
        ),
        components::select_item(
            "Logging level",
            log_level_label(form.logging_level),
            form.focus == ConfigField::LoggingLevel,
        ),
        components::select_item(
            "Logging format",
            log_format_label(form.logging_format),
            form.focus == ConfigField::LoggingFormat,
        ),
        components::input_item(
            &form.logging_dir,
            form.focus == ConfigField::LoggingDirectory,
        ),
    ];

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Config Defaults ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .highlight_symbol("");
    frame.render_widget(list, area);
}

fn render_message(frame: &mut Frame, area: Rect, view: ConfigView) {
    let (text, style) = if view.quit_prompt {
        (
            " Press q again to quit; all downloads will be cancelled.".to_string(),
            Style::default().fg(Color::Yellow),
        )
    } else {
        match &view.form.message {
            Some(msg) => match msg.kind {
                MessageKind::Info => (msg.text.clone(), Style::default().fg(Color::Green)),
                MessageKind::Error => (msg.text.clone(), Style::default().fg(Color::Red)),
            },
            None => (
                " Press S to save the configuration file.".to_string(),
                Style::default().fg(Color::Gray),
            ),
        }
    };

    let paragraph = Paragraph::new(text)
        .style(style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Console "),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

fn log_level_label(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Trace => "trace",
    }
}

fn log_format_label(format: LogFormat) -> &'static str {
    match format {
        LogFormat::Compact => "compact",
        LogFormat::Pretty => "pretty",
    }
}
