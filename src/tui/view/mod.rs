mod components;
mod config;
mod download;
mod footer;
mod home;

use crate::app::{App, CollectionPage, ConfigTab, HomeTab};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Paragraph},
};

pub fn draw(frame: &mut Frame, app: &App) {
    let view = AppView::from(app);
    let area = frame.area();
    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]);
    let [main_area, footer_area] = layout.areas(area);

    let version = env!("CARGO_PKG_VERSION");
    let title_left = " osu-collect • osu!collector downloader ";
    let title_right = format!(" v{} ", version);
    let title_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let version_style = Style::default().fg(Color::DarkGray);

    let shell = app_shell(title_left, title_style);
    let content_area = shell.inner(main_area);
    frame.render_widget(shell, main_area);

    let right_len = title_right.len() as u16;
    let version_x = main_area.x + main_area.width.saturating_sub(right_len + 1);
    let version_area = ratatui::layout::Rect {
        x: version_x,
        y: main_area.y,
        width: right_len,
        height: 1,
    };
    let version_paragraph = Paragraph::new(title_right)
        .style(version_style)
        .alignment(Alignment::Right);
    frame.render_widget(version_paragraph, version_area);

    match view.active_tab {
        0 => home::render(frame, content_area, view.home),
        1 => config::render(frame, content_area, view.config),
        _ => {
            if let Some(download_view) = view.download {
                download::render(frame, content_area, download_view);
            } else {
                home::render(frame, content_area, view.home);
            }
        }
    }

    footer::render(frame, footer_area, footer::FooterView::new(&view.tabs));
}

fn app_shell(title_left: &str, style: Style) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title_left.to_string())
        .title_alignment(Alignment::Left)
        .title_style(style)
}

#[derive(Clone, Copy)]
pub struct HomeView<'a> {
    pub form: &'a HomeTab,
}

impl<'a> From<&'a HomeTab> for HomeView<'a> {
    fn from(form: &'a HomeTab) -> Self {
        Self { form }
    }
}

#[derive(Clone, Copy)]
pub struct DownloadView<'a> {
    pub page: &'a CollectionPage,
}

impl<'a> From<&'a CollectionPage> for DownloadView<'a> {
    fn from(page: &'a CollectionPage) -> Self {
        Self { page }
    }
}

#[derive(Clone, Copy)]
pub struct ConfigView<'a> {
    pub form: &'a ConfigTab,
    pub quit_prompt: bool,
}

impl<'a> ConfigView<'a> {
    fn new(form: &'a ConfigTab, quit_prompt: bool) -> Self {
        Self { form, quit_prompt }
    }
}

pub struct TabsView {
    titles: Vec<String>,
    active_tab: usize,
}

impl TabsView {
    fn new(titles: Vec<String>, active_tab: usize) -> Self {
        Self { titles, active_tab }
    }

    pub fn titles(&self) -> &[String] {
        &self.titles
    }

    pub fn active(&self) -> usize {
        self.active_tab
    }
}

struct AppView<'a> {
    home: HomeView<'a>,
    config: ConfigView<'a>,
    download: Option<DownloadView<'a>>,
    tabs: TabsView,
    active_tab: usize,
}

impl<'a> From<&'a App> for AppView<'a> {
    fn from(app: &'a App) -> Self {
        let active_tab = app.active_tab();
        let download = app.download_for_tab(active_tab).map(DownloadView::from);

        Self {
            home: HomeView::from(&app.home),
            config: ConfigView::new(&app.config, app.home.quit_prompt),
            download,
            tabs: TabsView::new(app.tab_titles(), active_tab),
            active_tab,
        }
    }
}
