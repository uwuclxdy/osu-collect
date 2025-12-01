use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
}

#[derive(Debug, Clone)]
pub struct LocalBeatmap {
    pub id: u32,
    pub checksum: String,
}

#[derive(Debug, Clone)]
pub struct LocalBeatmapset {
    pub id: u32,
    pub beatmaps: Vec<LocalBeatmap>,
}

#[derive(Debug, Clone)]
pub struct LocalCollection {
    pub name: String,
    pub beatmap_checksums: Vec<String>,
}

pub trait BeatmapReader {
    fn list_beatmapsets(&self) -> Result<Vec<LocalBeatmapset>, String>;
    fn list_collections(&self) -> Result<Vec<LocalCollection>, String>;
    fn default_path() -> Option<PathBuf>;
}
