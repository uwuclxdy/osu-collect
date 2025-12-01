use super::common::{BeatmapReader, LocalBeatmap, LocalBeatmapset, LocalCollection};
use crate::realm_bridge::ffi;
use std::path::PathBuf;
use tracing::{debug, info, warn};

pub struct LazerReader {
    path: PathBuf,
}

impl LazerReader {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn realm_path(&self) -> PathBuf {
        self.path.join("client.realm")
    }
}

impl BeatmapReader for LazerReader {
    fn list_beatmapsets(&self) -> Result<Vec<LocalBeatmapset>, String> {
        let db_path = self.realm_path();
        if !db_path.exists() {
            return Err(format!("client.realm not found at {}", db_path.display()));
        }

        let db_path_str = db_path.to_str().ok_or("Invalid path encoding")?;

        let realm =
            ffi::open_realm(db_path_str).map_err(|e| format!("Failed to open realm: {e}"))?;

        let ffi_sets = realm.list_beatmapsets();

        let sets = ffi_sets
            .into_iter()
            .map(|s| LocalBeatmapset {
                id: s.id,
                beatmaps: s
                    .beatmaps
                    .into_iter()
                    .map(|b| LocalBeatmap {
                        id: b.id,
                        checksum: b.checksum,
                    })
                    .collect(),
            })
            .collect();

        Ok(sets)
    }

    fn list_collections(&self) -> Result<Vec<LocalCollection>, String> {
        let db_path = self.realm_path();
        info!(path = %db_path.display(), "Reading collections from Realm database");

        if !db_path.exists() {
            warn!(path = %db_path.display(), "client.realm not found");
            return Err(format!("client.realm not found at {}", db_path.display()));
        }

        let db_path_str = db_path.to_str().ok_or("Invalid path encoding")?;

        let realm =
            ffi::open_realm(db_path_str).map_err(|e| format!("Failed to open realm: {e}"))?;

        debug!("Realm database opened successfully");

        let ffi_collections = realm.list_collections();
        info!(
            count = ffi_collections.len(),
            "Retrieved collections from Realm"
        );

        for (i, c) in ffi_collections.iter().enumerate() {
            debug!(
                index = i,
                name = %c.name,
                beatmap_count = c.beatmap_checksums.len(),
                "Collection from Realm"
            );
        }

        let collections = ffi_collections
            .into_iter()
            .map(|c| LocalCollection {
                name: c.name,
                beatmap_checksums: c.beatmap_checksums.into_iter().collect(),
            })
            .collect();

        Ok(collections)
    }

    fn default_path() -> Option<PathBuf> {
        Self::find_installation()
    }
}

impl LazerReader {
    fn find_installation() -> Option<PathBuf> {
        let candidates = Self::candidate_paths();
        candidates
            .into_iter()
            .find(|p| p.join("client.realm").exists())
    }

    fn candidate_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        #[cfg(target_os = "windows")]
        {
            // %appdata%\osu (default user data directory)
            if let Some(data) = dirs::data_dir() {
                paths.push(data.join("osu"));
            }
        }

        #[cfg(target_os = "linux")]
        {
            // ~/.local/share/osu (default)
            if let Some(data) = dirs::data_local_dir() {
                paths.push(data.join("osu"));
            }
            // ~/.local/share/osu-lazer (alternative naming)
            if let Some(data) = dirs::data_local_dir() {
                paths.push(data.join("osu-lazer"));
            }
        }

        #[cfg(target_os = "macos")]
        {
            // ~/Library/Application Support/osu
            if let Some(data) = dirs::data_dir() {
                paths.push(data.join("osu"));
            }
        }

        paths
    }
}
