use super::{
    collection::CollectionPage,
    home::{HomeField, HomeTab},
};
use crate::{
    config::Config,
    download::{DownloadEvent, DownloadId, DownloadRequest, DownloadStage},
    utils,
};
use crossterm::event::{KeyCode, KeyEvent};
use tracing::debug;

pub struct App {
    pub home: HomeTab,
    pub downloads: Vec<CollectionPage>,
    pub active_tab: usize,
    next_download_id: DownloadId,
}

#[derive(Debug)]
pub enum AppCommand {
    StartDownload {
        id: DownloadId,
        request: DownloadRequest,
    },
    CancelDownload {
        id: DownloadId,
    },
    Quit,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            home: HomeTab::new(&config),
            downloads: Vec::new(),
            active_tab: 0,
            next_download_id: 1,
        }
    }

    pub fn active_tab(&self) -> usize {
        self.active_tab
    }

    pub fn next_tab(&mut self) {
        if self.downloads.is_empty() {
            self.active_tab = 0;
            return;
        }
        let total = self.downloads.len() + 1;
        self.active_tab = (self.active_tab + 1) % total;
    }

    pub fn prev_tab(&mut self) {
        if self.downloads.is_empty() {
            self.active_tab = 0;
            return;
        }
        let total = self.downloads.len() + 1;
        if self.active_tab == 0 {
            self.active_tab = total - 1;
        } else {
            self.active_tab -= 1;
        }
    }

    pub fn request_download(&mut self) -> Option<(DownloadId, DownloadRequest)> {
        match self.home.build_request() {
            Ok(request) => {
                if self.downloads.len() >= usize::MAX - 1 {
                    self.home.set_error("Too many downloads queued");
                    return None;
                }

                let id = self.next_download_id;
                self.next_download_id += 1;

                let placeholder_title =
                    Self::placeholder_collection_title(&request.collection_input, id);
                let concurrent = usize::from(request.concurrent.max(1));
                let mut page = CollectionPage::new(id, placeholder_title, concurrent);
                page.stage = DownloadStage::Resolving;
                self.downloads.push(page);
                self.active_tab = self.downloads.len();

                self.home.set_info(&format!("Queued download #{id}"));

                Some((id, request))
            }
            Err(err) => {
                self.home.set_error(&err);
                None
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<AppCommand> {
        let is_quit_key = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc);
        if self.home.quit_prompt && !is_quit_key {
            self.home.quit_prompt = false;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                return self.handle_quit_key();
            }
            KeyCode::Left => self.prev_tab(),
            KeyCode::Right => self.next_tab(),
            KeyCode::Tab => {
                if self.active_tab() == 0 {
                    self.home.next_field();
                }
            }
            KeyCode::BackTab => {
                if self.active_tab() == 0 {
                    self.home.prev_field();
                }
            }
            KeyCode::Up => {
                if self.active_tab() == 0 {
                    self.home.prev_field();
                }
            }
            KeyCode::Down => {
                if self.active_tab() == 0 {
                    self.home.next_field();
                }
            }
            KeyCode::Enter => {
                if let Some((id, request)) = self.request_download() {
                    return Some(AppCommand::StartDownload { id, request });
                }
            }
            KeyCode::Char(' ') => {
                if self.active_tab() == 0 {
                    if matches!(
                        self.home.focus,
                        HomeField::SkipExisting
                            | HomeField::AutoOverwrite
                            | HomeField::MirrorNerinyan
                            | HomeField::MirrorCatboyCentral
                            | HomeField::MirrorCatboyUs
                            | HomeField::MirrorCatboyAsia
                            | HomeField::MirrorOsuDirect
                            | HomeField::MirrorSayobot
                            | HomeField::NoVideo
                    ) {
                        self.home.toggle_current();
                    } else {
                        self.home.handle_char(' ');
                    }
                }
            }
            KeyCode::Char(ch) => {
                if self.active_tab() == 0 {
                    self.home.handle_char(ch);
                }
            }
            KeyCode::Backspace => {
                if self.active_tab() == 0 {
                    self.home.backspace();
                }
            }
            _ => {}
        }

        None
    }

    pub fn handle_download_event(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::CollectionReady {
                id,
                collection_name,
                uploader,
                total_maps,
                output_dir,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.title = collection_name.clone();
                    page.uploader = Some(uploader);
                    page.total_maps = total_maps;
                    page.download_target = total_maps;
                    page.output_dir = Some(output_dir);
                    page.stage = DownloadStage::Downloading;
                    page.push_log("Collection fetched");
                }
            }
            DownloadEvent::BeatmapsRegistered { id, beatmap_ids } => {
                if let Some(page) = self.page_mut(id) {
                    page.register_beatmaps(&beatmap_ids);
                }
            }
            DownloadEvent::BeatmapProgress {
                id,
                beatmapset_id,
                downloaded,
                total,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.update_progress(beatmapset_id, downloaded, total);
                }
            }
            DownloadEvent::BeatmapStatus {
                id,
                beatmapset_id,
                stage,
                message,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.update_status(beatmapset_id, stage, &message);
                }
            }
            DownloadEvent::DownloadTarget { id, remaining } => {
                if let Some(page) = self.page_mut(id) {
                    page.download_target = remaining;
                }
            }
            DownloadEvent::OverallProgress {
                id,
                downloaded,
                skipped,
                failed,
                unverified,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.stats.downloaded = downloaded;
                    page.stats.skipped = skipped;
                    page.stats.failed = failed;
                    page.stats.unverified = unverified;
                    let downloaded = page.stats.downloaded as usize;
                    let ratio = if page.download_target == 0 {
                        1.0
                    } else {
                        let goal = page.download_target as f64;
                        (downloaded as f64 / goal).clamp(0.0, 1.0)
                    };
                    if ratio > 0.5 && !page.progress_label_style_locked {
                        page.progress_label_style_locked = true;
                        page.progress_label_bold_when_locked = true;
                    }
                }
            }
            DownloadEvent::Log { id, message } => {
                if let Some(page) = self.page_mut(id) {
                    page.push_log(&message);
                }
            }
            DownloadEvent::ThreadStatus {
                id,
                thread_index,
                message,
                rate_limited,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.update_thread_status(thread_index, &message, rate_limited);
                }
            }
            DownloadEvent::StageChanged { id, stage } => {
                if let Some(page) = self.page_mut(id) {
                    page.stage = stage;
                }
            }
            DownloadEvent::FailedMaps { id, beatmapset_ids } => {
                if let Some(page) = self.page_mut(id) {
                    page.set_failed_maps(beatmapset_ids);
                }
            }
            DownloadEvent::Finished { id, summary } => {
                if let Some(page) = self.page_mut(id) {
                    page.stage = DownloadStage::Completed;
                    page.summary = Some(summary);
                    page.push_log("Download finished");
                }
            }
            DownloadEvent::Failed { id, message } => {
                if let Some(page) = self.page_mut(id) {
                    page.stage = DownloadStage::Failed;
                    page.push_log(&format!("Error: {message}"));
                    page.summary = None;
                }
            }
        }
    }

    pub fn tab_titles(&self) -> Vec<String> {
        let mut titles = Vec::with_capacity(self.downloads.len() + 1);
        titles.push("Home".to_string());
        for page in &self.downloads {
            titles.push(page.title.clone());
        }
        titles
    }

    pub fn download_for_tab(&self, tab_index: usize) -> Option<&CollectionPage> {
        if tab_index == 0 {
            None
        } else {
            self.downloads.get(tab_index - 1)
        }
    }

    pub fn handle_cancel_result(&mut self, download_id: DownloadId, was_running: bool) {
        let title = self.remove_download_page(download_id);
        self.active_tab = 0;
        self.home.quit_prompt = false;

        let display = title.unwrap_or_else(|| format!("download #{download_id}"));
        if was_running {
            self.home
                .set_info(&format!("Cancelled download \"{}\"", display));
        } else {
            self.home
                .set_info(&format!("No active download to cancel for \"{}\"", display));
        }
    }

    fn page_mut(&mut self, id: DownloadId) -> Option<&mut CollectionPage> {
        self.downloads.iter_mut().find(|page| page.id == id)
    }

    fn remove_download_page(&mut self, download_id: DownloadId) -> Option<String> {
        if let Some(position) = self
            .downloads
            .iter()
            .position(|page| page.id == download_id)
        {
            let title = self.downloads[position].title.clone();
            self.downloads.remove(position);
            Some(title)
        } else {
            None
        }
    }

    fn handle_quit_key(&mut self) -> Option<AppCommand> {
        if self.active_tab() == 0 {
            if self.downloads.is_empty() {
                self.home.quit_prompt = false;
                return Some(AppCommand::Quit);
            }

            if self.home.quit_prompt {
                self.home.quit_prompt = false;
                return Some(AppCommand::Quit);
            }

            self.home.quit_prompt = true;
            debug!("Quit requested; showing confirmation prompt");
            return None;
        }

        self.home.quit_prompt = false;
        self.cancel_command_for_active_tab()
    }

    fn cancel_command_for_active_tab(&mut self) -> Option<AppCommand> {
        if self.active_tab == 0 {
            return None;
        }

        let idx = self.active_tab - 1;
        if let Some(page) = self.downloads.get(idx) {
            return Some(AppCommand::CancelDownload { id: page.id });
        }

        self.active_tab = 0;
        None
    }

    fn placeholder_collection_title(input: &str, download_id: DownloadId) -> String {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return format!("collection {download_id}");
        }

        match utils::parse_collection_id(trimmed) {
            Ok(collection_id) => format!("collection {collection_id}"),
            Err(_) => format!("collection {trimmed}"),
        }
    }
}
