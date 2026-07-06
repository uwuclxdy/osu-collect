use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};
use tracing::{debug, warn};

pub const IGNORED_MAPS_FILE: &str = "ignored-beatmapsets.json";
pub const IGNORED_MAPS_ENV_PATH: &str = "OSU_COLLECT_IGNORED_MAPS";
const SCHEMA_VERSION: u32 = 1;
static SAVE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Beatmapsets the user manually marked as installed. They are hidden from the
/// update-source missing list until a later scan detects a genuine install, at
/// which point [`reconcile_installed`] un-ignores them.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredMapsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub beatmapset_ids: Vec<u32>,
}

impl IgnoredMapsFile {
    pub fn ids(&self) -> HashSet<u32> {
        self.beatmapset_ids.iter().copied().collect()
    }

    pub fn insert_all(&mut self, ids: impl IntoIterator<Item = u32>) {
        self.beatmapset_ids.extend(ids);
        normalize(&mut self.beatmapset_ids);
    }

    pub fn remove_all(&mut self, ids: &HashSet<u32>) {
        self.beatmapset_ids.retain(|id| !ids.contains(id));
    }
}

pub fn ignored_maps_path() -> Option<PathBuf> {
    if let Ok(custom) = env::var(IGNORED_MAPS_ENV_PATH) {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    platform_data_dir().map(ignored_maps_path_in)
}

pub fn ignored_maps_path_in(base: PathBuf) -> PathBuf {
    base.join("osu-collect").join(IGNORED_MAPS_FILE)
}

#[cfg(windows)]
fn platform_data_dir() -> Option<PathBuf> {
    dirs::data_dir()
}

#[cfg(not(windows))]
fn platform_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
}

pub fn load(path: &Path) -> IgnoredMapsFile {
    match fs::read_to_string(path) {
        Err(_) => {
            debug!(path = %path.display(), "no ignored maps file found");
            IgnoredMapsFile {
                schema_version: SCHEMA_VERSION,
                ..Default::default()
            }
        }
        Ok(contents) => match serde_json::from_str::<IgnoredMapsFile>(&contents) {
            Ok(mut ignored) => {
                ignored.schema_version = SCHEMA_VERSION;
                normalize(&mut ignored.beatmapset_ids);
                ignored
            }
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to parse ignored maps file");
                IgnoredMapsFile {
                    schema_version: SCHEMA_VERSION,
                    ..Default::default()
                }
            }
        },
    }
}

pub fn save(ignored: &IgnoredMapsFile, path: &Path) {
    let Ok(_guard) = SAVE_LOCK.lock() else {
        warn!(path = %path.display(), "failed to lock ignored maps save");
        return;
    };

    let mut ignored = ignored.clone();
    ignored.schema_version = SCHEMA_VERSION;
    normalize(&mut ignored.beatmapset_ids);

    let contents = match serde_json::to_string_pretty(&ignored) {
        Ok(contents) => contents,
        Err(err) => {
            warn!(error = %err, "failed to serialize ignored maps");
            return;
        }
    };

    if let Err(err) = super::write_atomic(path, "json.tmp", &contents) {
        warn!(path = %path.display(), error = %err, "failed to save ignored maps");
    } else {
        debug!(path = %path.display(), "saved ignored maps");
    }
}

/// Add `ids` to the ignore list (the manual "mark installed" action).
pub fn record_ignored(path: &Path, ids: impl IntoIterator<Item = u32>) {
    let mut ignored = load(path);
    ignored.insert_all(ids);
    save(&ignored, path);
}

/// Drop any ignored id that is now genuinely installed (present in
/// `installed`), persist the trimmed list, and return the ids still to hide for
/// this scan. The auto-clear half of the reconcile pattern: a real install
/// un-hides a manually-ignored set so it never lingers once detection catches
/// up.
pub fn reconcile_installed(path: &Path, installed: &HashSet<u32>) -> HashSet<u32> {
    let mut ignored = load(path);
    let resolved: HashSet<u32> = ignored.ids().intersection(installed).copied().collect();
    if !resolved.is_empty() {
        ignored.remove_all(&resolved);
        save(&ignored, path);
    }
    ignored.ids()
}

fn normalize(ids: &mut Vec<u32>) {
    ids.sort_unstable();
    ids.dedup();
}

#[cfg(test)]
#[path = "../../tests/unit/ignored_maps.rs"]
mod tests;
