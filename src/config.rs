use serde::{Deserialize, Serialize};
use crate::error::{AppError, Result};

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub mirror: MirrorConfig,
    pub download: DownloadConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MirrorConfig {
    pub url: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DownloadConfig {
    pub skip_existing: bool,
    pub concurrent: u8,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mirror: MirrorConfig {
                url: "https://api.nerinyan.moe/d/{id}".to_string(),
            },
            download: DownloadConfig {
                skip_existing: false,
                concurrent: 1,
            },
        }
    }
}

impl Config {
    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if !self.mirror.url.contains("{id}") {
            return Err(AppError::other_static(
                "Mirror URL must contain {id} placeholder"
            ));
        }

        if !self.mirror.url.starts_with("http://") && !self.mirror.url.starts_with("https://") {
            return Err(AppError::other_static(
                "Mirror URL must start with http:// or https://"
            ));
        }

        if self.download.concurrent == 0 {
            return Err(AppError::other_static(
                "Concurrent downloads must be at least 1"
            ));
        }

        if self.download.concurrent > 50 {
            eprintln!("Warning: concurrent downloads set to {}, which is unusually high.",
                      self.download.concurrent);
            eprintln!("Recommended maximum is 20 to avoid rate limiting.");
        }

        Ok(())
    }

    /// Merge CLI arguments into config
    pub fn merge_with_cli(
        mut self,
        mirror: Option<String>,
        skip_existing: bool,
    ) -> Self {
        if let Some(mirror_url) = mirror {
            self.mirror.url = mirror_url;
        }

        if skip_existing {
            self.download.skip_existing = true;
        }

        self
    }
}

/// Load configuration from file or use defaults
pub fn load_config() -> Config {
    if let Some(config_dir) = dirs::config_dir() {
        let config_path = config_dir.join("osu-collect").join("config.toml");
        if let Ok(contents) = std::fs::read_to_string(&config_path) {
            if let Ok(config) = toml::from_str(&contents) {
                return config;
            }
        }
    }
    Config::default()
}
