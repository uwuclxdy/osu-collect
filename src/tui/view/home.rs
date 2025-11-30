use crate::app::{HomeField, HomeTab, MessageKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap},
};

use super::{HomeView, components};

pub fn render(frame: &mut Frame, area: Rect, view: HomeView) {
    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(area);
    render_form(frame, chunks[0], view.form);
    render_message(frame, chunks[1], view.form);
}

fn render_form(frame: &mut Frame, area: Rect, form: &HomeTab) {
    let items = vec![
        components::input_item(&form.collection, form.focus == HomeField::Collection),
        components::input_item(&form.directory, form.focus == HomeField::Directory),
        components::input_item(&form.custom_mirror, form.focus == HomeField::CustomMirror),
        mirror_toggle(
            "Use Nerinyan",
            "api.nerinyan.moe",
            form.nerinyan,
            form.focus == HomeField::MirrorNerinyan,
        ),
        mirror_toggle(
            "Use Catboy Central",
            "catboy.best",
            form.catboy_central,
            form.focus == HomeField::MirrorCatboyCentral,
        ),
        mirror_toggle(
            "Use Catboy US",
            "us.catboy.best",
            form.catboy_us,
            form.focus == HomeField::MirrorCatboyUs,
        ),
        mirror_toggle(
            "Use Catboy Asia",
            "sg.catboy.best",
            form.catboy_asia,
            form.focus == HomeField::MirrorCatboyAsia,
        ),
        mirror_toggle(
            "Use osu.direct",
            "osu.direct",
            form.osu_direct,
            form.focus == HomeField::MirrorOsuDirect,
        ),
        mirror_toggle(
            "Use Sayobot",
            "dl.sayobot.cn",
            form.sayobot,
            form.focus == HomeField::MirrorSayobot,
        ),
        mirror_toggle(
            "Use Nekoha",
            "mirror.nekoha.moe",
            form.nekoha,
            form.focus == HomeField::MirrorNekoha,
        ),
        components::input_item(&form.threads, form.focus == HomeField::Threads),
        toggle(
            "Skip existing files",
            form.skip_existing,
            form.focus == HomeField::SkipExisting,
        ),
        toggle(
            "Auto-overwrite",
            form.auto_overwrite,
            form.focus == HomeField::AutoOverwrite,
        ),
        toggle(
            "Download without video",
            form.no_video,
            form.focus == HomeField::NoVideo,
        ),
    ];

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Configuration ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .highlight_symbol("");
    frame.render_widget(list, area);
}

fn toggle(label: &str, value: bool, focused: bool) -> ratatui::widgets::ListItem<'static> {
    components::toggle_item(label, value, focused)
}

fn mirror_toggle(label: &str, url: &str, value: bool, focused: bool) -> ListItem<'static> {
    let marker = if value { "[x]" } else { "[ ]" };
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
        Span::raw(format!(" {} ", label)),
        Span::styled(url.to_string(), Style::default().fg(Color::DarkGray)),
    ];

    ListItem::new(Line::from(spans)).style(style)
}

fn render_message(frame: &mut Frame, area: Rect, form: &HomeTab) {
    let (text, style) = if form.quit_prompt {
        (
            " Press q again to quit; all downloads will be cancelled.".to_string(),
            Style::default().fg(Color::Yellow),
        )
    } else {
        match &form.message {
            Some(msg) => match msg.kind {
                MessageKind::Info => (msg.text.clone(), Style::default().fg(Color::Green)),
                MessageKind::Error => (msg.text.clone(), Style::default().fg(Color::Red)),
            },
            None => (
                " Press Enter to start downloading the collection shown above.".to_string(),
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
