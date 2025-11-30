use crate::{
    mirrors,
    utils::{AppError, Result},
};
use serde::{Deserialize, Serialize};

const fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct Config {
    pub mirror: MirrorConfig,
    pub download: DownloadConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MirrorConfig {
    #[serde(default = "default_true")]
    pub nerinyan: bool,
    #[serde(default, alias = "catboy")]
    pub catboy_central: bool,
    #[serde(default)]
    pub catboy_us: bool,
    #[serde(default)]
    pub catboy_asia: bool,
    #[serde(default)]
    pub osu_direct: bool,
    #[serde(default)]
    pub sayobot: bool,
    #[serde(default)]
    pub nekoha: bool,
    #[serde(default)]
    pub url: Option<Box<str>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
#[derive(Default)]
pub struct DownloadConfig {
    pub skip_existing: bool,
    pub concurrent: Option<u8>,
    pub no_video: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub enabled: bool,
    pub level: LogLevel,
    pub format: LogFormat,
    pub file_dir: Option<Box<str>>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum LogFormat {
    #[default]
    Compact,
    Pretty,
}

impl Default for MirrorConfig {
    fn default() -> Self {
        Self {
            nerinyan: true,
            catboy_central: false,
            catboy_us: false,
            catboy_asia: false,
            osu_direct: false,
            sayobot: false,
            nekoha: false,
            url: None,
        }
    }
}

impl MirrorConfig {
    pub fn custom_template(&self) -> Option<&str> {
        self.url.as_deref().and_then(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        })
    }

    fn any_enabled(&self) -> bool {
        self.nerinyan
            || self.catboy_central
            || self.catboy_us
            || self.catboy_asia
            || self.osu_direct
            || self.sayobot
            || self.nekoha
            || self.custom_template().is_some()
    }
}

impl DownloadConfig {
    pub const DEFAULT_THREADS: u8 = 3;

    pub fn resolved_concurrent(&self) -> u8 {
        self.concurrent.unwrap_or(Self::DEFAULT_THREADS)
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            level: LogLevel::Info,
            format: LogFormat::Compact,
            file_dir: None,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if !self.mirror.any_enabled() {
            return Err(AppError::config("Enable at least one mirror"));
        }

        if let Some(custom) = self.mirror.custom_template() {
            mirrors::validate_template(custom)?;
        }

        if let Some(concurrent) = self.download.concurrent {
            if concurrent == 0 {
                return Err(AppError::config("Thread count must be at least 1"));
            }

            if concurrent > 50 {
                eprintln!(
                    "Warning: thread count set to {}, which is unusually high.",
                    concurrent
                );
                eprintln!("Recommended maximum is 20 to avoid rate limiting.");
            }
        }

        if self.logging.enabled
            && let Some(dir) = self.logging.file_dir.as_deref()
            && dir.trim().is_empty()
        {
            return Err(AppError::config(
                "logging.file_dir cannot be empty when logging is enabled",
            ));
        }

        Ok(())
    }
}
