use super::{
    collection::{CollectionPage, ThreadStatusLine},
    config::ConfigTab,
    home::{HomeField, HomeTab},
    updates::{UpdatesAction, UpdatesTab},
};
use crate::{
    config::{
        Config, ConfigService,
        constants::{CONFIG_TAB_INDEX, HOME_TAB_INDEX, STATIC_TABS, UPDATES_TAB_INDEX},
    },
    download::{
        DownloadConfig, DownloadEvent, DownloadId, DownloadRequest, DownloadStage,
        SelectiveDownloadRequest,
    },
    utils,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tracing::debug;

pub struct App {
    pub home: HomeTab,
    pub updates: UpdatesTab,
    pub config: ConfigTab,
    pub downloads: Vec<CollectionPage>,
    pub active_tab: usize,
    next_download_id: DownloadId,
    config_service: ConfigService,
}

#[derive(Debug)]
pub enum AppCommand {
    StartDownload {
        id: DownloadId,
        request: DownloadRequest,
    },
    StartSelectiveDownload {
        id: DownloadId,
        request: SelectiveDownloadRequest,
    },
    CancelDownload {
        id: DownloadId,
    },
    ScanLocalDatabase,
    Quit,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            home: HomeTab::new(&config),
            updates: UpdatesTab::new(),
            config: ConfigTab::new(&config),
            downloads: Vec::new(),
            active_tab: HOME_TAB_INDEX,
            next_download_id: 1,
            config_service: ConfigService::new(),
        }
    }

    pub fn active_tab(&self) -> usize {
        self.active_tab
    }

    pub fn next_tab(&mut self) -> Option<AppCommand> {
        let total = self.total_tabs();
        self.active_tab = (self.active_tab + 1) % total;
        self.check_auto_scan()
    }

    pub fn prev_tab(&mut self) -> Option<AppCommand> {
        let total = self.total_tabs();
        if self.active_tab == 0 {
            self.active_tab = total - 1;
        } else {
            self.active_tab -= 1;
        }
        self.check_auto_scan()
    }

    fn check_auto_scan(&mut self) -> Option<AppCommand> {
        if self.active_tab == UPDATES_TAB_INDEX {
            self.updates.scan.scan_generation = self.updates.scan.scan_generation.wrapping_add(1);
            Some(AppCommand::ScanLocalDatabase)
        } else {
            None
        }
    }

    fn focus_next_field(&mut self) {
        match self.active_tab() {
            HOME_TAB_INDEX => self.home.next_field(),
            UPDATES_TAB_INDEX => self.updates.next_field(),
            CONFIG_TAB_INDEX => self.config.next_field(),
            _ => {}
        }
    }

    fn focus_prev_field(&mut self) {
        match self.active_tab() {
            HOME_TAB_INDEX => self.home.prev_field(),
            UPDATES_TAB_INDEX => self.updates.prev_field(),
            CONFIG_TAB_INDEX => self.config.prev_field(),
            _ => {}
        }
    }

    fn try_save_config(&mut self) {
        match self.config.build_config() {
            Ok(new_config) => {
                if let Err(err) = new_config.validate() {
                    self.config.set_error(err.to_string());
                    return;
                }

                match self.config_service.save(&new_config) {
                    Ok(path) => {
                        let message = format!(
                            "Config saved to {} (applies on next launch)",
                            path.display()
                        );
                        self.config.set_info(message);
                    }
                    Err(err) => self.config.set_error(err.to_string()),
                }
            }
            Err(err) => self.config.set_error(err),
        }
    }

    fn total_tabs(&self) -> usize {
        STATIC_TABS + self.downloads.len()
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
                let concurrent = usize::from(request.config.concurrent.max(1));
                let mut page = CollectionPage::new(id, placeholder_title, concurrent);
                page.stage = DownloadStage::Resolving;
                self.downloads.push(page);
                self.active_tab = STATIC_TABS + self.downloads.len() - 1;

                self.home.set_info(&format!("Queued download #{id}"));

                Some((id, request))
            }
            Err(err) => {
                self.home.set_error(&err);
                None
            }
        }
    }

    pub fn request_selective_download(&mut self) -> Option<(DownloadId, SelectiveDownloadRequest)> {
        let beatmapset_ids = self.updates.selected_beatmapset_ids();
        if beatmapset_ids.is_empty() {
            self.updates.set_error("No beatmaps selected for download");
            return None;
        }

        let collection_ids: Vec<u32> = self
            .updates
            .selected_collection_ids()
            .into_iter()
            .filter_map(|id| u32::try_from(id).ok())
            .collect();

        if collection_ids.is_empty() {
            self.updates.set_error("No collections available");
            return None;
        }

        let mirrors = self.home.build_mirrors();
        if mirrors.is_empty() {
            self.updates
                .set_error("No mirrors selected (configure in Home tab)");
            return None;
        }

        let concurrent = self.home.resolved_threads();

        let directory = if self.home.directory.value.trim().is_empty() {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".to_string())
        } else {
            self.home.directory.value.trim().to_string()
        };

        if self.downloads.len() >= usize::MAX - 1 {
            self.updates.set_error("Too many downloads queued");
            return None;
        }

        let id = self.next_download_id;
        self.next_download_id += 1;

        let placeholder_title = if collection_ids.len() == 1 {
            format!("Update #{}", collection_ids[0])
        } else {
            format!("Update ({} collections)", collection_ids.len())
        };

        let concurrent_usize = usize::from(concurrent.max(1));
        let mut page = CollectionPage::new(id, placeholder_title, concurrent_usize);
        page.stage = DownloadStage::Resolving;
        self.downloads.push(page);
        self.active_tab = STATIC_TABS + self.downloads.len() - 1;

        self.updates.set_info(format!(
            "Queued update download #{id} ({} beatmaps)",
            beatmapset_ids.len()
        ));

        let config = DownloadConfig {
            directory,
            mirrors,
            concurrent,
            verify_zip_eocd: self.home.verify_zip_eocd,
            max_retries: self.home.resolved_retries(),
        };

        let request = SelectiveDownloadRequest {
            collection_ids,
            beatmapset_ids,
            config,
        };

        Some((id, request))
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<AppCommand> {
        let is_quit_key = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc);
        if self.home.quit_prompt && !is_quit_key {
            self.home.quit_prompt = false;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.active_tab() == UPDATES_TAB_INDEX && self.updates.handle_escape().is_some()
                {
                    return None;
                }
                return self.handle_quit_key();
            }
            KeyCode::Left => {
                if !self.updates.selection.in_collection_list
                    && !self.updates.selection.in_beatmap_list
                    && let Some(cmd) = self.prev_tab()
                {
                    return Some(cmd);
                }
            }
            KeyCode::Right => {
                if !self.updates.selection.in_collection_list
                    && !self.updates.selection.in_beatmap_list
                    && let Some(cmd) = self.next_tab()
                {
                    return Some(cmd);
                }
            }
            KeyCode::Tab => self.focus_next_field(),
            KeyCode::BackTab => self.focus_prev_field(),
            KeyCode::Up => {
                if self.active_tab() == UPDATES_TAB_INDEX
                    && (self.updates.selection.in_collection_list
                        || self.updates.selection.in_beatmap_list)
                {
                    self.updates.scroll_up();
                } else {
                    self.focus_prev_field();
                }
            }
            KeyCode::Down => {
                if self.active_tab() == UPDATES_TAB_INDEX
                    && (self.updates.selection.in_collection_list
                        || self.updates.selection.in_beatmap_list)
                {
                    self.updates.scroll_down();
                } else {
                    self.focus_next_field();
                }
            }
            KeyCode::Enter => {
                if self.active_tab() == HOME_TAB_INDEX
                    && let Some((id, request)) = self.request_download()
                {
                    return Some(AppCommand::StartDownload { id, request });
                }
                if self.active_tab() == UPDATES_TAB_INDEX {
                    // Don't allow download while scan is in progress
                    if !self.updates.is_scan_ready() {
                        return None;
                    }
                    if let UpdatesAction::Download = self.updates.handle_enter()
                        && let Some((id, request)) = self.request_selective_download() {
                            return Some(AppCommand::StartSelectiveDownload { id, request });
                        }
                }
            }
            KeyCode::Char(' ') => match self.active_tab() {
                HOME_TAB_INDEX => {
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
                            | HomeField::MirrorNekoha
                            | HomeField::NoVideo
                    ) {
                        self.home.toggle_current();
                    } else {
                        self.home.handle_char(' ');
                    }
                }
                UPDATES_TAB_INDEX => {
                    if self.updates.toggle_current() == UpdatesAction::RefreshAll {
                        return Some(AppCommand::ScanLocalDatabase);
                    }
                }
                CONFIG_TAB_INDEX => {
                    if self.config.focus.is_text_input() {
                        self.config.handle_char(' ');
                    } else {
                        self.config.toggle_current();
                    }
                }
                _ => {}
            },
            KeyCode::Char(ch) => match self.active_tab() {
                HOME_TAB_INDEX => self.home.handle_char(ch),
                UPDATES_TAB_INDEX => self.updates.handle_char(ch),
                CONFIG_TAB_INDEX => {
                    let focus = self.config.focus;
                    let is_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    if ch == 's' && !is_ctrl && !focus.is_text_input() {
                        self.try_save_config();
                    } else {
                        self.config.handle_char(ch);
                    }
                }
                _ => {}
            },
            KeyCode::Backspace => match self.active_tab() {
                HOME_TAB_INDEX => self.home.backspace(),
                UPDATES_TAB_INDEX => self.updates.backspace(),
                CONFIG_TAB_INDEX => self.config.backspace(),
                _ => {}
            },
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
            DownloadEvent::CollectionSizeResolved { id, total_bytes } => {
                if let Some(page) = self.page_mut(id) {
                    page.stats.total_collection_bytes = Some(total_bytes);
                }
            }
            DownloadEvent::LowDiskSpace {
                id,
                available_bytes,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.low_disk_space = Some(available_bytes);
                }
            }
            DownloadEvent::VerifiedMapSizes { id, total_bytes } => {
                if let Some(page) = self.page_mut(id) {
                    page.stats.verified_bytes += total_bytes;
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
                thread_index,
                downloaded,
                total,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.update_progress(beatmapset_id, downloaded, total);
                    page.update_thread_progress(thread_index, downloaded);
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
                beatmapset_id,
            } => {
                if let Some(page) = self.page_mut(id) {
                    let completed = ThreadStatusLine::is_completion_message(&message);
                    if completed {
                        page.reset_thread_speed(thread_index);
                    }
                    page.update_thread_status(thread_index, &message, rate_limited, beatmapset_id);
                }
            }
            DownloadEvent::StageChanged { id, stage } => {
                if let Some(page) = self.page_mut(id) {
                    page.stage = stage;
                }
            }
            DownloadEvent::FailedMaps { id, failures } => {
                if let Some(page) = self.page_mut(id) {
                    page.set_failed_maps(failures);
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
        let mut titles = Vec::with_capacity(self.downloads.len() + STATIC_TABS);
        titles.push("Home".to_string());
        titles.push("Updates".to_string());
        titles.push("Config".to_string());
        for page in &self.downloads {
            titles.push(page.title.clone());
        }
        titles
    }

    pub fn download_for_tab(&self, tab_index: usize) -> Option<&CollectionPage> {
        if tab_index < STATIC_TABS {
            None
        } else {
            self.downloads.get(tab_index - STATIC_TABS)
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
        if self.active_tab() < STATIC_TABS {
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
        if self.active_tab < STATIC_TABS {
            return None;
        }

        let idx = self.active_tab - STATIC_TABS;
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
