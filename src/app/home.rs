use super::messages::{AppMessage, MessageKind};
use crate::{
    config::Config,
    download::DownloadRequest,
    mirrors::{CatboyRegion, MirrorEndpoint, MirrorKind},
};
use std::{env, str::FromStr};

pub struct InputField {
    pub label: &'static str,
    pub value: String,
    pub placeholder: String,
}

impl InputField {
    fn push(&mut self, ch: char) {
        self.value.push(ch);
    }

    fn pop(&mut self) {
        self.value.pop();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeField {
    Collection,
    Directory,
    CustomMirror,
    MirrorNerinyan,
    MirrorOsuDirect,
    MirrorSayobot,
    MirrorNekoha,
    MirrorCatboyCentral,
    MirrorCatboyUs,
    MirrorCatboyAsia,
    Threads,
    SkipExisting,
    AutoOverwrite,
    NoVideo,
}

pub struct HomeTab {
    pub collection: InputField,
    pub directory: InputField,
    pub custom_mirror: InputField,
    pub threads: InputField,
    pub skip_existing: bool,
    pub auto_overwrite: bool,
    pub nerinyan: bool,
    pub catboy_central: bool,
    pub catboy_us: bool,
    pub catboy_asia: bool,
    pub osu_direct: bool,
    pub sayobot: bool,
    pub nekoha: bool,
    pub no_video: bool,
    pub focus: HomeField,
    pub message: Option<AppMessage>,
    pub quit_prompt: bool,
    default_threads: u8,
    default_directory: String,
}

impl HomeTab {
    pub fn new(config: &Config) -> Self {
        let mut nerinyan = config.mirror.nerinyan;
        let catboy_central = config.mirror.catboy_central;
        let catboy_us = config.mirror.catboy_us;
        let catboy_asia = config.mirror.catboy_asia;
        let osu_direct = config.mirror.osu_direct;
        let sayobot = config.mirror.sayobot;
        let nekoha = config.mirror.nekoha;
        let custom_template = config.mirror.custom_template().unwrap_or("");

        if !nerinyan
            && !catboy_central
            && !catboy_us
            && !catboy_asia
            && !osu_direct
            && !sayobot
            && !nekoha
            && custom_template.is_empty()
        {
            nerinyan = true;
        }

        let default_directory = env::current_dir()
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string());

        let placeholder_directory = default_directory.clone();

        let default_threads = config.download.resolved_concurrent();
        let threads_value = config
            .download
            .concurrent
            .map(|value| value.to_string())
            .unwrap_or_default();

        Self {
            collection: InputField {
                label: "Collection URL or ID",
                value: String::new(),
                placeholder: "https://osucollector.com/collections/...".to_string(),
            },
            directory: InputField {
                label: "Download directory",
                value: String::new(),
                placeholder: placeholder_directory,
            },
            custom_mirror: InputField {
                label: "Custom mirror URL (optional)",
                value: custom_template.to_string(),
                placeholder: "https://example.com/d/{id}".to_string(),
            },
            threads: InputField {
                label: "Threads",
                value: threads_value,
                placeholder: "3".to_string(),
            },
            skip_existing: config.download.skip_existing,
            auto_overwrite: false,
            nerinyan,
            catboy_central,
            catboy_us,
            catboy_asia,
            osu_direct,
            sayobot,
            nekoha,
            no_video: config.download.no_video,
            focus: HomeField::Collection,
            message: None,
            quit_prompt: false,
            default_threads,
            default_directory,
        }
    }

    pub fn next_field(&mut self) {
        self.focus = match self.focus {
            HomeField::Collection => HomeField::Directory,
            HomeField::Directory => HomeField::CustomMirror,
            HomeField::CustomMirror => HomeField::MirrorNerinyan,
            HomeField::MirrorNerinyan => HomeField::MirrorOsuDirect,
            HomeField::MirrorOsuDirect => HomeField::MirrorSayobot,
            HomeField::MirrorSayobot => HomeField::MirrorNekoha,
            HomeField::MirrorNekoha => HomeField::MirrorCatboyCentral,
            HomeField::MirrorCatboyCentral => HomeField::MirrorCatboyUs,
            HomeField::MirrorCatboyUs => HomeField::MirrorCatboyAsia,
            HomeField::MirrorCatboyAsia => HomeField::Threads,
            HomeField::Threads => HomeField::SkipExisting,
            HomeField::SkipExisting => HomeField::AutoOverwrite,
            HomeField::AutoOverwrite => HomeField::NoVideo,
            HomeField::NoVideo => HomeField::Collection,
        };
    }

    pub fn prev_field(&mut self) {
        self.focus = match self.focus {
            HomeField::Collection => HomeField::NoVideo,
            HomeField::Directory => HomeField::Collection,
            HomeField::CustomMirror => HomeField::Directory,
            HomeField::MirrorNerinyan => HomeField::CustomMirror,
            HomeField::MirrorOsuDirect => HomeField::MirrorNerinyan,
            HomeField::MirrorSayobot => HomeField::MirrorOsuDirect,
            HomeField::MirrorNekoha => HomeField::MirrorSayobot,
            HomeField::MirrorCatboyCentral => HomeField::MirrorNekoha,
            HomeField::MirrorCatboyUs => HomeField::MirrorCatboyCentral,
            HomeField::MirrorCatboyAsia => HomeField::MirrorCatboyUs,
            HomeField::Threads => HomeField::MirrorCatboyAsia,
            HomeField::SkipExisting => HomeField::Threads,
            HomeField::AutoOverwrite => HomeField::SkipExisting,
            HomeField::NoVideo => HomeField::AutoOverwrite,
        };
    }

    pub fn handle_char(&mut self, ch: char) {
        match self.focus {
            HomeField::Collection => self.collection.push(ch),
            HomeField::Directory => self.directory.push(ch),
            HomeField::CustomMirror => self.custom_mirror.push(ch),
            HomeField::Threads => {
                if ch.is_ascii_digit() {
                    self.threads.push(ch);
                }
            }
            HomeField::MirrorNerinyan
            | HomeField::MirrorCatboyCentral
            | HomeField::MirrorCatboyUs
            | HomeField::MirrorCatboyAsia
            | HomeField::MirrorOsuDirect
            | HomeField::MirrorSayobot
            | HomeField::MirrorNekoha
            | HomeField::SkipExisting
            | HomeField::AutoOverwrite
            | HomeField::NoVideo => {}
        }
    }

    pub fn backspace(&mut self) {
        match self.focus {
            HomeField::Collection => self.collection.pop(),
            HomeField::Directory => self.directory.pop(),
            HomeField::CustomMirror => self.custom_mirror.pop(),
            HomeField::Threads => self.threads.pop(),
            HomeField::MirrorNerinyan
            | HomeField::MirrorCatboyCentral
            | HomeField::MirrorCatboyUs
            | HomeField::MirrorCatboyAsia
            | HomeField::MirrorOsuDirect
            | HomeField::MirrorSayobot
            | HomeField::MirrorNekoha
            | HomeField::SkipExisting
            | HomeField::AutoOverwrite
            | HomeField::NoVideo => {}
        }
    }

    pub fn toggle_current(&mut self) {
        match self.focus {
            HomeField::MirrorNerinyan => {
                self.nerinyan = !self.nerinyan;
            }
            HomeField::MirrorCatboyCentral => {
                self.catboy_central = !self.catboy_central;
            }
            HomeField::MirrorCatboyUs => {
                self.catboy_us = !self.catboy_us;
            }
            HomeField::MirrorCatboyAsia => {
                self.catboy_asia = !self.catboy_asia;
            }
            HomeField::MirrorOsuDirect => {
                self.osu_direct = !self.osu_direct;
            }
            HomeField::MirrorSayobot => {
                self.sayobot = !self.sayobot;
            }
            HomeField::MirrorNekoha => {
                self.nekoha = !self.nekoha;
            }
            HomeField::SkipExisting => {
                self.skip_existing = !self.skip_existing;
                if self.skip_existing {
                    self.auto_overwrite = false;
                }
            }
            HomeField::AutoOverwrite => {
                self.auto_overwrite = !self.auto_overwrite;
                if self.auto_overwrite {
                    self.skip_existing = false;
                }
            }
            HomeField::NoVideo => {
                self.no_video = !self.no_video;
            }
            _ => {}
        }
    }

    pub fn set_error(&mut self, message: &str) {
        self.message = Some(AppMessage {
            kind: MessageKind::Error,
            text: message.to_string(),
        });
    }

    pub fn set_info(&mut self, message: &str) {
        self.message = Some(AppMessage {
            kind: MessageKind::Info,
            text: message.to_string(),
        });
    }

    pub fn build_request(&self) -> Result<DownloadRequest, String> {
        let collection_input = self.collection.value.trim();
        if collection_input.is_empty() {
            return Err("Collection ID or URL is required".to_string());
        }

        let directory = if self.directory.value.trim().is_empty() {
            self.default_directory.clone()
        } else {
            self.directory.value.trim().to_string()
        };

        let threads_value = if self.threads.value.trim().is_empty() {
            self.default_threads
        } else {
            parse_thread_count(&self.threads.value)?
        };

        if threads_value == 0 || threads_value > 50 {
            return Err("Thread count must be between 1 and 50".to_string());
        }

        let mut mirrors = Vec::new();

        if self.nerinyan
            && let Some(endpoint) = MirrorEndpoint::builtin(MirrorKind::Nerinyan, self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.osu_direct
            && let Some(endpoint) = MirrorEndpoint::builtin(MirrorKind::OsuDirect, self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.sayobot
            && let Some(endpoint) = MirrorEndpoint::builtin(MirrorKind::Sayobot, self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.nekoha
            && let Some(endpoint) = MirrorEndpoint::builtin(MirrorKind::Nekoha, self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.catboy_central
            && let Some(endpoint) =
                MirrorEndpoint::builtin(MirrorKind::Catboy(CatboyRegion::Central), self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.catboy_us
            && let Some(endpoint) =
                MirrorEndpoint::builtin(MirrorKind::Catboy(CatboyRegion::Us), self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.catboy_asia
            && let Some(endpoint) =
                MirrorEndpoint::builtin(MirrorKind::Catboy(CatboyRegion::Asia), self.no_video)
        {
            mirrors.push(endpoint);
        }

        let trimmed_custom = self.custom_mirror.value.trim();
        if !trimmed_custom.is_empty() {
            let custom_endpoint =
                MirrorEndpoint::custom(trimmed_custom).map_err(|err| err.to_string())?;
            mirrors.push(custom_endpoint);
        }

        if mirrors.is_empty() {
            return Err("Select at least one mirror".to_string());
        }

        Ok(DownloadRequest {
            collection_input: collection_input.to_string(),
            directory,
            mirrors,
            concurrent: threads_value,
            skip_existing: self.skip_existing,
            auto_overwrite: self.auto_overwrite,
        })
    }

    pub fn build_mirrors(&self) -> Vec<MirrorEndpoint> {
        let mut mirrors = Vec::new();

        if self.nerinyan
            && let Some(endpoint) = MirrorEndpoint::builtin(MirrorKind::Nerinyan, self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.osu_direct
            && let Some(endpoint) = MirrorEndpoint::builtin(MirrorKind::OsuDirect, self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.sayobot
            && let Some(endpoint) = MirrorEndpoint::builtin(MirrorKind::Sayobot, self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.nekoha
            && let Some(endpoint) = MirrorEndpoint::builtin(MirrorKind::Nekoha, self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.catboy_central
            && let Some(endpoint) =
                MirrorEndpoint::builtin(MirrorKind::Catboy(CatboyRegion::Central), self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.catboy_us
            && let Some(endpoint) =
                MirrorEndpoint::builtin(MirrorKind::Catboy(CatboyRegion::Us), self.no_video)
        {
            mirrors.push(endpoint);
        }

        if self.catboy_asia
            && let Some(endpoint) =
                MirrorEndpoint::builtin(MirrorKind::Catboy(CatboyRegion::Asia), self.no_video)
        {
            mirrors.push(endpoint);
        }

        let trimmed_custom = self.custom_mirror.value.trim();
        if !trimmed_custom.is_empty()
            && let Ok(custom_endpoint) = MirrorEndpoint::custom(trimmed_custom)
        {
            mirrors.push(custom_endpoint);
        }

        mirrors
    }

    pub fn resolved_threads(&self) -> u8 {
        if self.threads.value.trim().is_empty() {
            self.default_threads
        } else {
            parse_thread_count(&self.threads.value).unwrap_or(self.default_threads)
        }
    }
}

fn parse_thread_count(value: &str) -> Result<u8, String> {
    u8::from_str(value.trim()).map_err(|_| "Thread count must be between 1 and 50".to_string())
}
