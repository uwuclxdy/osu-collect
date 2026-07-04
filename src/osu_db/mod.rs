pub mod checksum;
pub mod lazer;
pub mod stable;

pub use checksum::Md5;
pub use lazer::LazerReader;
pub use stable::StableReader;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OsuClient {
    Stable,
    #[default]
    Lazer,
}

impl OsuClient {
    pub fn toggle(&mut self) {
        *self = match self {
            OsuClient::Stable => OsuClient::Lazer,
            OsuClient::Lazer => OsuClient::Stable,
        };
    }

    /// Short lowercase display label (`stable` / `lazer`).
    pub fn label(self) -> &'static str {
        match self {
            OsuClient::Stable => "stable",
            OsuClient::Lazer => "lazer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalBeatmap {
    pub checksum: Md5,
}

#[derive(Debug, Clone)]
pub struct LocalBeatmapset {
    pub id: u32,
    pub beatmaps: Box<[LocalBeatmap]>,
}

#[derive(Debug, Clone)]
pub struct LocalCollection {
    pub name: String,
    pub beatmap_checksums: Box<[Md5]>,
}

pub trait BeatmapReader {
    fn list_beatmapsets(&self) -> Result<Vec<LocalBeatmapset>, String>;
    fn list_collections(&self) -> Result<Vec<LocalCollection>, String>;
    fn default_path() -> Option<PathBuf>;
}

fn find_installation(
    candidates: impl IntoIterator<Item = PathBuf>,
    db_file: &str,
) -> Option<PathBuf> {
    candidates
        .into_iter()
        .find(|path| path.join(db_file).exists())
}

fn require_db(path: &Path, db_file: &str) -> Result<PathBuf, String> {
    let db_path = path.join(db_file);
    if db_path.exists() {
        Ok(db_path)
    } else {
        Err(format!("{db_file} not found at {}", db_path.display()))
    }
}
