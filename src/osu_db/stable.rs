use super::{
    BeatmapReader, LocalBeatmap, LocalBeatmapset, LocalCollection, checksum, find_installation,
    require_db,
};
use osu_db::{collection::CollectionList, listing::Listing};
use std::{collections::HashMap, path::PathBuf};

pub struct StableReader {
    path: PathBuf,
}

impl StableReader {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn osu_db_path(&self) -> Result<PathBuf, String> {
        require_db(&self.path, "osu!.db")
    }

    fn collection_db_path(&self) -> Result<PathBuf, String> {
        require_db(&self.path, "collection.db")
    }
}

impl BeatmapReader for StableReader {
    fn list_beatmapsets(&self) -> Result<Vec<LocalBeatmapset>, String> {
        let db_path = self.osu_db_path()?;
        let metadata = std::fs::metadata(&db_path).map_err(|e| e.to_string())?;
        if metadata.len() < 20 {
            return Ok(Vec::new());
        }

        let listing = match Listing::from_file(&db_path) {
            Ok(listing) => listing,
            Err(e) => {
                tracing::warn!(
                    path = %db_path.display(),
                    error = %e,
                    "Failed to parse osu!.db, returning empty beatmapset list"
                );
                return Ok(Vec::new());
            }
        };

        let mut sets: HashMap<u32, Vec<LocalBeatmap>> = HashMap::new();

        for beatmap in listing.beatmaps {
            // Skip beatmaps with invalid IDs (unsubmitted beatmaps have negative IDs)
            if beatmap.beatmapset_id <= 0 || beatmap.beatmap_id <= 0 {
                continue;
            }

            let beatmapset_id = beatmap.beatmapset_id as u32;
            let cksum = beatmap
                .hash
                .as_deref()
                .and_then(checksum::parse_hex)
                .unwrap_or(checksum::EMPTY);

            sets.entry(beatmapset_id)
                .or_default()
                .push(LocalBeatmap { checksum: cksum });
        }

        // Boxing growth-doubled vecs strands allocator pages: a measured shrink
        // attempt (sorted `Box<[Md5]>`, 2026-05) saved ~nothing for that reason.
        // If RSS matters again, reserve exact capacity upfront or swap allocators.
        Ok(sets
            .into_iter()
            .map(|(id, beatmaps)| LocalBeatmapset {
                id,
                beatmaps: beatmaps.into_boxed_slice(),
            })
            .collect())
    }

    fn list_collections(&self) -> Result<Vec<LocalCollection>, String> {
        let db_path = self.collection_db_path()?;
        let metadata = std::fs::metadata(&db_path).map_err(|e| e.to_string())?;
        if metadata.len() < 8 {
            return Ok(Vec::new());
        }

        let collection_list = match CollectionList::from_file(&db_path) {
            Ok(list) => list,
            Err(e) => {
                tracing::warn!(
                    path = %db_path.display(),
                    error = %e,
                    "Failed to parse collection.db, returning empty collection list"
                );
                return Ok(Vec::new());
            }
        };

        let collections = collection_list
            .collections
            .into_iter()
            .map(|c| LocalCollection {
                name: c.name.unwrap_or_default(),
                beatmap_checksums: c
                    .beatmap_hashes
                    .into_iter()
                    .flatten()
                    .map(|h| checksum::parse_hex(&h).unwrap_or(checksum::EMPTY))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            })
            .collect();

        Ok(collections)
    }

    fn default_path() -> Option<PathBuf> {
        find_installation(Self::candidate_paths(), "osu!.db")
    }
}

impl StableReader {
    fn candidate_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        #[cfg(target_os = "windows")]
        {
            // %localappdata%\osu! (default)
            if let Some(local) = dirs::data_local_dir() {
                paths.push(local.join("osu!"));
            }
            // C:\osu! (common alternative recommended for easier access)
            paths.push(PathBuf::from("C:\\osu!"));
        }

        #[cfg(target_os = "linux")]
        {
            if let Some(home) = dirs::home_dir() {
                let username = whoami::username().unwrap_or_default();

                // osu-winello: read path from osupath config file
                let osupath_file = home.join(".local/share/osuconfig/osupath");
                if let Ok(content) = std::fs::read_to_string(&osupath_file) {
                    let path = PathBuf::from(content.trim());
                    if path.exists() {
                        paths.push(path);
                    }
                }

                // osu-wine direct installation (not in drive_c)
                // ~/.local/share/osu-wine/OSU/
                paths.push(home.join(".local/share/osu-wine/OSU"));
                paths.push(home.join(".local/share/osu-wine/osu!"));

                // Common Wine prefix locations from docs
                let wine_prefixes = [
                    // Standard manual installations
                    home.join(".wine_osu"),
                    home.join(".wineosu"),
                    home.join(".wine"),
                    home.join("osu/prefix"),
                    // osu-wine package: ~/.local/share/osu-wine/WINE.win32/
                    home.join(".local/share/osu-wine/WINE.win32"),
                    // AUR package: ~/.local/share/wineprefixes/osu-stable/
                    home.join(".local/share/wineprefixes/osu-stable"),
                    // osu-winello: ~/.local/share/osuconfig/WINE.win32/
                    home.join(".local/share/osuconfig/WINE.win32"),
                ];

                for prefix in wine_prefixes {
                    // Direct install: <prefix>/drive_c/osu! (recommended)
                    paths.push(prefix.join("drive_c/osu!"));
                    // Default installer location (within user profile)
                    paths.push(
                        prefix
                            .join("drive_c/users")
                            .join(&username)
                            .join("Local Settings/Application Data/osu!"),
                    );
                    // AppData location (Windows 7+ style path)
                    paths.push(
                        prefix
                            .join("drive_c/users")
                            .join(&username)
                            .join("AppData/Local/osu!"),
                    );
                }

                // Lutris: ~/Games/osu!/
                paths.push(home.join("Games/osu!/drive_c/osu!"));
                // Also check for Lutris wine runners
                paths.push(home.join(".local/share/lutris/runners/wine"));

                // Bottles (Flatpak): ~/.var/app/com.usebottles.bottles/data/bottles/bottles/
                let bottles_base =
                    home.join(".var/app/com.usebottles.bottles/data/bottles/bottles");
                if bottles_base.exists()
                    && let Ok(entries) = std::fs::read_dir(&bottles_base)
                {
                    for entry in entries.flatten() {
                        let bottle_path = entry.path();
                        paths.push(bottle_path.join("drive_c/osu!"));
                        paths.push(
                            bottle_path
                                .join("drive_c/users")
                                .join(&username)
                                .join("Local Settings/Application Data/osu!"),
                        );
                        paths.push(
                            bottle_path
                                .join("drive_c/users")
                                .join(&username)
                                .join("AppData/Local/osu!"),
                        );
                    }
                }

                // AUR package game data: ~/.local/share/osu-stable/
                paths.push(home.join(".local/share/osu-stable"));
            }
        }

        #[cfg(target_os = "macos")]
        {
            // Wineskin bundle locations
            paths.push(PathBuf::from(
                "/Applications/osu!.app/Contents/Resources/drive_c/osu!",
            ));
            paths.push(PathBuf::from(
                "/Applications/osu!.app/Contents/Resources/drive_c/Program Files/osu!",
            ));

            if let Some(home) = dirs::home_dir() {
                let username = whoami::username().unwrap_or_default();

                // User Applications folder
                paths.push(home.join("Applications/osu!.app/Contents/Resources/drive_c/osu!"));
                paths.push(
                    home.join(
                        "Applications/osu!.app/Contents/Resources/drive_c/Program Files/osu!",
                    ),
                );

                // Check within Wineskin bundle user paths
                for app_path in [
                    PathBuf::from("/Applications/osu!.app/Contents/Resources"),
                    home.join("Applications/osu!.app/Contents/Resources"),
                ] {
                    paths.push(
                        app_path
                            .join("drive_c/users")
                            .join(&username)
                            .join("Local Settings/Application Data/osu!"),
                    );
                    paths.push(
                        app_path
                            .join("drive_c/users")
                            .join(&username)
                            .join("AppData/Local/osu!"),
                    );
                }
            }
        }

        paths
    }
}
