use super::{migrator, model::Config};
use crate::utils::{AppError, Result};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};
use tracing::warn;

use super::constants::{CONFIG_ENV_PATH, CONFIG_FILE, CONFIG_SUBDIR};

pub fn config_path() -> Option<PathBuf> {
    if let Ok(custom_path) = env::var(CONFIG_ENV_PATH) {
        let trimmed = custom_path.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    dirs::config_dir().map(|dir| dir.join(CONFIG_SUBDIR).join(CONFIG_FILE))
}

pub fn load_config() -> Result<Config> {
    let path =
        config_path().ok_or_else(|| AppError::config("unable to determine config directory"))?;
    load_config_from(&path)
}

pub fn load_config_or_default() -> Config {
    match load_config() {
        Ok(config) => config,
        Err(err) => {
            if let Some(path) = config_path() {
                warn!(path = %path.display(), error = %err, "Falling back to default config");
            } else {
                warn!(error = %err, "Falling back to default config");
            }
            Config::default()
        }
    }
}

pub fn load_config_from(path: impl AsRef<Path>) -> Result<Config> {
    let path = path.as_ref();
    migrator::migrate_in_place(path);
    let contents = std::fs::read_to_string(path)?;
    let config = toml::from_str::<Config>(&contents)
        .map_err(|err| AppError::config_dynamic(format!("invalid config file: {}", err)))?;

    // Strip any unknown/garbage keys: serde ignores them on read, so persist a
    // clean round-trip back to disk. Only write when the serialization differs
    // from the on-disk bytes, so a clean config is never needlessly rewritten.
    if let Ok(clean) = toml::to_string_pretty(&config)
        && clean != contents
        && let Err(err) = write_config_atomic(path, &clean)
    {
        warn!(path = %path.display(), error = %err, "failed to rewrite cleaned config");
    }

    Ok(config)
}

pub fn save_config(config: &Config) -> Result<PathBuf> {
    let path = config_path().ok_or_else(|| AppError::config("unable to find config directory"))?;

    let contents = toml::to_string_pretty(config)
        .map_err(|err| AppError::config_dynamic(format!("failed to serialize config: {}", err)))?;

    write_config_atomic(&path, &contents)?;
    Ok(path)
}

/// Atomically write `contents` to `path` via a temp file + rename, creating the
/// parent directory if missing.
fn write_config_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("toml.tmp");
    {
        let mut tmp = fs::File::create(&tmp_path)?;
        tmp.write_all(contents.as_bytes())?;
        tmp.sync_all()?;
    }
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(AppError::from(err));
    }
    Ok(())
}
