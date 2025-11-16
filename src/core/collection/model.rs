use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Collection {
    pub id: u32,
    pub name: Box<str>,
    pub uploader: Uploader,
    pub beatmapsets: Vec<Beatmapset>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Uploader {
    pub id: u32,
    pub username: Box<str>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Beatmapset {
    pub id: u32,
    #[serde(default)]
    pub beatmaps: Vec<Beatmap>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Beatmap {
    pub id: u32,
    pub checksum: Box<str>,
}
