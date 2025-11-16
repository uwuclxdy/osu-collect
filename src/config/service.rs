use super::model::Config;
use crate::utils::{AppError, Result};
use std::{
    env, fs,
    path::{Path, PathBuf},
};
use tracing::warn;

const CONFIG_SUBDIR: &str = "osu-collect";
const CONFIG_FILE: &str = "config.toml";
const CONFIG_ENV_PATH: &str = "OSU_COLLECT_CONFIG";

#[derive(Debug, Clone, Copy, Default)]
pub struct ConfigService;

impl ConfigService {
    pub fn new() -> Self {
        Self
    }

    pub fn load(&self) -> Result<Config> {
        let path = self
            .config_path()
            .ok_or_else(|| AppError::config("Unable to determine config directory"))?;
        self.load_path(&path)
    }

    pub fn load_or_default(&self) -> Config {
        match self.load() {
            Ok(config) => config,
            Err(err) => {
                if let Some(path) = self.config_path() {
                    warn!(path = %path.display(), error = %err, "Falling back to default config");
                } else {
                    warn!(error = %err, "Falling back to default config");
                }
                Config::default()
            }
        }
    }

    pub fn load_path(&self, path: impl AsRef<Path>) -> Result<Config> {
        let contents = std::fs::read_to_string(path.as_ref())?;
        toml::from_str::<Config>(&contents)
            .map_err(|err| AppError::config_dynamic(format!("Invalid config file: {}", err)))
    }

    pub fn save(&self, config: &Config) -> Result<PathBuf> {
        let path = self
            .config_path()
            .ok_or_else(|| AppError::config("Unable to find config directory"))?;

        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string_pretty(config).map_err(|err| {
            AppError::config_dynamic(format!("Failed to serialize config: {}", err))
        })?;
        fs::write(&path, contents)?;
        Ok(path)
    }

    pub fn config_path(&self) -> Option<PathBuf> {
        if let Ok(custom_path) = env::var(CONFIG_ENV_PATH) {
            let trimmed = custom_path.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }

        dirs::config_dir().map(|dir| dir.join(CONFIG_SUBDIR).join(CONFIG_FILE))
    }
}
