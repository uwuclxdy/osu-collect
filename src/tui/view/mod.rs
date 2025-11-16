mod components;
mod download;
mod footer;
mod home;

use crate::app::{App, CollectionPage, HomeTab};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders},
};

pub fn draw(frame: &mut Frame, app: &App) {
    let view = AppView::from(app);
    let area = frame.area();
    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]);
    let [main_area, footer_area] = layout.areas(area);

    let shell = app_shell();
    let content_area = shell.inner(main_area);
    frame.render_widget(shell, main_area);

    match view.active_tab {
        0 => home::render(frame, content_area, view.home),
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

fn app_shell() -> Block<'static> {
    let header_line = Line::from(vec![
        Span::styled(
            " osu-collect ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  •  osu!collector downloader "),
    ]);

    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(header_line)
        .title_alignment(Alignment::Left)
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
            download,
            tabs: TabsView::new(app.tab_titles(), active_tab),
            active_tab,
        }
    }
}
