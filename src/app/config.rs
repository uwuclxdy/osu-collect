use super::{
    home::InputField,
    messages::{AppMessage, MessageKind},
};
use crate::config::{Config, DownloadConfig, LogFormat, LogLevel, LoggingConfig, MirrorConfig};

const LOG_LEVELS: [LogLevel; 5] = [
    LogLevel::Error,
    LogLevel::Warn,
    LogLevel::Info,
    LogLevel::Debug,
    LogLevel::Trace,
];

const LOG_FORMATS: [LogFormat; 2] = [LogFormat::Compact, LogFormat::Pretty];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    MirrorNerinyan,
    MirrorCatboyCentral,
    MirrorCatboyUs,
    MirrorCatboyAsia,
    MirrorOsuDirect,
    MirrorSayobot,
    MirrorCustomUrl,
    DownloadSkipExisting,
    DownloadThreads,
    DownloadNoVideo,
    LoggingEnabled,
    LoggingLevel,
    LoggingFormat,
    LoggingDirectory,
}

impl ConfigField {
    pub fn is_text_input(self) -> bool {
        matches!(
            self,
            ConfigField::MirrorCustomUrl
                | ConfigField::DownloadThreads
                | ConfigField::LoggingDirectory
        )
    }
}

pub struct ConfigTab {
    pub nerinyan: bool,
    pub catboy_central: bool,
    pub catboy_us: bool,
    pub catboy_asia: bool,
    pub osu_direct: bool,
    pub sayobot: bool,
    pub custom_mirror: InputField,
    pub skip_existing: bool,
    pub threads: InputField,
    pub no_video: bool,
    pub logging_enabled: bool,
    pub logging_level: LogLevel,
    pub logging_format: LogFormat,
    pub logging_dir: InputField,
    pub focus: ConfigField,
    pub message: Option<AppMessage>,
}

impl ConfigTab {
    pub fn new(config: &Config) -> Self {
        Self {
            nerinyan: config.mirror.nerinyan,
            catboy_central: config.mirror.catboy_central,
            catboy_us: config.mirror.catboy_us,
            catboy_asia: config.mirror.catboy_asia,
            osu_direct: config.mirror.osu_direct,
            sayobot: config.mirror.sayobot,
            custom_mirror: InputField {
                label: "Custom mirror URL",
                value: config.mirror.custom_template().unwrap_or("").to_string(),
                placeholder: "https://example.com/d/{id}".to_string(),
            },
            skip_existing: config.download.skip_existing,
            threads: InputField {
                label: "Default thread count",
                value: config
                    .download
                    .concurrent
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                placeholder: format!("leave blank (default {})", DownloadConfig::DEFAULT_THREADS),
            },
            no_video: config.download.no_video,
            logging_enabled: config.logging.enabled,
            logging_level: config.logging.level,
            logging_format: config.logging.format,
            logging_dir: InputField {
                label: "Log file directory",
                value: config.logging.file_dir.as_deref().unwrap_or("").to_string(),
                placeholder: "~/.local/share/osu-collect/logs".to_string(),
            },
            focus: ConfigField::MirrorNerinyan,
            message: None,
        }
    }

    pub fn next_field(&mut self) {
        self.focus = match self.focus {
            ConfigField::MirrorNerinyan => ConfigField::MirrorCatboyCentral,
            ConfigField::MirrorCatboyCentral => ConfigField::MirrorCatboyUs,
            ConfigField::MirrorCatboyUs => ConfigField::MirrorCatboyAsia,
            ConfigField::MirrorCatboyAsia => ConfigField::MirrorOsuDirect,
            ConfigField::MirrorOsuDirect => ConfigField::MirrorSayobot,
            ConfigField::MirrorSayobot => ConfigField::MirrorCustomUrl,
            ConfigField::MirrorCustomUrl => ConfigField::DownloadSkipExisting,
            ConfigField::DownloadSkipExisting => ConfigField::DownloadThreads,
            ConfigField::DownloadThreads => ConfigField::DownloadNoVideo,
            ConfigField::DownloadNoVideo => ConfigField::LoggingEnabled,
            ConfigField::LoggingEnabled => ConfigField::LoggingLevel,
            ConfigField::LoggingLevel => ConfigField::LoggingFormat,
            ConfigField::LoggingFormat => ConfigField::LoggingDirectory,
            ConfigField::LoggingDirectory => ConfigField::MirrorNerinyan,
        };
    }

    pub fn prev_field(&mut self) {
        self.focus = match self.focus {
            ConfigField::MirrorNerinyan => ConfigField::LoggingDirectory,
            ConfigField::MirrorCatboyCentral => ConfigField::MirrorNerinyan,
            ConfigField::MirrorCatboyUs => ConfigField::MirrorCatboyCentral,
            ConfigField::MirrorCatboyAsia => ConfigField::MirrorCatboyUs,
            ConfigField::MirrorOsuDirect => ConfigField::MirrorCatboyAsia,
            ConfigField::MirrorSayobot => ConfigField::MirrorOsuDirect,
            ConfigField::MirrorCustomUrl => ConfigField::MirrorSayobot,
            ConfigField::DownloadSkipExisting => ConfigField::MirrorCustomUrl,
            ConfigField::DownloadThreads => ConfigField::DownloadSkipExisting,
            ConfigField::DownloadNoVideo => ConfigField::DownloadThreads,
            ConfigField::LoggingEnabled => ConfigField::DownloadNoVideo,
            ConfigField::LoggingLevel => ConfigField::LoggingEnabled,
            ConfigField::LoggingFormat => ConfigField::LoggingLevel,
            ConfigField::LoggingDirectory => ConfigField::LoggingFormat,
        };
    }

    pub fn handle_char(&mut self, ch: char) {
        self.clear_message();
        match self.focus {
            ConfigField::MirrorCustomUrl => self.custom_mirror.value.push(ch),
            ConfigField::DownloadThreads => {
                if ch.is_ascii_digit() {
                    self.threads.value.push(ch);
                }
            }
            ConfigField::LoggingDirectory => self.logging_dir.value.push(ch),
            _ => {}
        }
    }

    pub fn backspace(&mut self) {
        self.clear_message();
        match self.focus {
            ConfigField::MirrorCustomUrl => {
                self.custom_mirror.value.pop();
            }
            ConfigField::DownloadThreads => {
                self.threads.value.pop();
            }
            ConfigField::LoggingDirectory => {
                self.logging_dir.value.pop();
            }
            _ => {}
        }
    }

    pub fn toggle_current(&mut self) {
        self.clear_message();
        match self.focus {
            ConfigField::MirrorNerinyan => self.nerinyan = !self.nerinyan,
            ConfigField::MirrorCatboyCentral => self.catboy_central = !self.catboy_central,
            ConfigField::MirrorCatboyUs => self.catboy_us = !self.catboy_us,
            ConfigField::MirrorCatboyAsia => self.catboy_asia = !self.catboy_asia,
            ConfigField::MirrorOsuDirect => self.osu_direct = !self.osu_direct,
            ConfigField::MirrorSayobot => self.sayobot = !self.sayobot,
            ConfigField::DownloadSkipExisting => self.skip_existing = !self.skip_existing,
            ConfigField::DownloadNoVideo => self.no_video = !self.no_video,
            ConfigField::LoggingEnabled => self.logging_enabled = !self.logging_enabled,
            ConfigField::LoggingLevel => self.cycle_logging_level(),
            ConfigField::LoggingFormat => self.cycle_logging_format(),
            _ => {}
        }
    }

    pub fn cycle_logging_level(&mut self) {
        self.logging_level = next_value(LOG_LEVELS, self.logging_level);
    }

    pub fn cycle_logging_format(&mut self) {
        self.logging_format = next_value(LOG_FORMATS, self.logging_format);
    }

    pub fn build_config(&self) -> Result<Config, String> {
        let concurrent = self.parse_concurrent()?;
        let mirror = MirrorConfig {
            nerinyan: self.nerinyan,
            catboy_central: self.catboy_central,
            catboy_us: self.catboy_us,
            catboy_asia: self.catboy_asia,
            osu_direct: self.osu_direct,
            sayobot: self.sayobot,
            url: self
                .trimmed_custom_mirror()
                .map(|value| value.into_boxed_str()),
        };

        let download = DownloadConfig {
            skip_existing: self.skip_existing,
            concurrent,
            no_video: self.no_video,
        };

        let logging = LoggingConfig {
            enabled: self.logging_enabled,
            level: self.logging_level,
            format: self.logging_format,
            file_dir: self
                .trimmed_logging_dir()
                .map(|value| value.into_boxed_str()),
        };

        Ok(Config {
            mirror,
            download,
            logging,
        })
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.message = Some(AppMessage {
            kind: MessageKind::Error,
            text: message.into(),
        });
    }

    pub fn set_info(&mut self, message: impl Into<String>) {
        self.message = Some(AppMessage {
            kind: MessageKind::Info,
            text: message.into(),
        });
    }

    pub fn clear_message(&mut self) {
        self.message = None;
    }

    fn parse_concurrent(&self) -> Result<Option<u8>, String> {
        let trimmed = self.threads.value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let value = trimmed
            .parse::<u8>()
            .map_err(|_| "Thread count must be a valid number between 1 and 50".to_string())?;
        if value == 0 || value > 50 {
            return Err("Thread count must be between 1 and 50".to_string());
        }

        Ok(Some(value))
    }

    fn trimmed_custom_mirror(&self) -> Option<String> {
        let trimmed = self.custom_mirror.value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn trimmed_logging_dir(&self) -> Option<String> {
        let trimmed = self.logging_dir.value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

fn next_value<T: Copy + PartialEq, const N: usize>(values: [T; N], current: T) -> T {
    values
        .iter()
        .position(|&value| value == current)
        .map(|idx| values[(idx + 1) % values.len()])
        .unwrap_or(values[0])
}
