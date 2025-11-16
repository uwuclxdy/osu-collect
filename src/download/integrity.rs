use crate::core::collection::Beatmapset;
use md5::{Digest, Md5};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use tokio::task;
use tracing::{debug, trace};
use zip::ZipArchive;

#[derive(Clone)]
pub(crate) struct BeatmapsetExpectation {
    pub(crate) beatmap_hashes: HashMap<u32, String>,
}

#[derive(Clone)]
pub(crate) struct ExpectationData {
    pub(crate) by_set: HashMap<u32, BeatmapsetExpectation>,
    pub(crate) by_beatmap: HashMap<u32, u32>,
}

pub(crate) struct ExpectationIndex {
    data: RwLock<Arc<ExpectationData>>,
}

impl ExpectationIndex {
    pub(crate) fn new(beatmapsets: &[Beatmapset]) -> Self {
        let mut by_set = HashMap::new();
        let mut by_beatmap = HashMap::new();

        for set in beatmapsets {
            if set.beatmaps.is_empty() {
                continue;
            }

            let mut hashes = HashMap::new();
            for beatmap in &set.beatmaps {
                by_beatmap.insert(beatmap.id, set.id);
                hashes.insert(beatmap.id, beatmap.checksum.to_lowercase());
            }

            by_set.insert(
                set.id,
                BeatmapsetExpectation {
                    beatmap_hashes: hashes,
                },
            );
        }

        let data = ExpectationData { by_set, by_beatmap };
        Self {
            data: RwLock::new(Arc::new(data)),
        }
    }

    pub(crate) fn snapshot(&self) -> Arc<ExpectationData> {
        self.data.read().expect("ExpectationIndex poisoned").clone()
    }

    pub(crate) fn overwrite_set_hashes(
        &self,
        set_id: u32,
        new_hashes: HashMap<u32, String>,
    ) -> bool {
        let mut guard = self
            .data
            .write()
            .expect("ExpectationIndex poisoned while writing");
        let data = Arc::make_mut(&mut guard);

        let previous = data
            .by_set
            .get(&set_id)
            .map(|expectation| expectation.beatmap_hashes.clone());
        let changed = previous
            .as_ref()
            .map(|old| old != &new_hashes)
            .unwrap_or(true);

        data.by_set.insert(
            set_id,
            BeatmapsetExpectation {
                beatmap_hashes: new_hashes.clone(),
            },
        );

        if let Some(old_hashes) = previous {
            for beatmap_id in old_hashes.keys() {
                data.by_beatmap.remove(beatmap_id);
            }
        }

        for beatmap_id in new_hashes.keys() {
            data.by_beatmap.insert(*beatmap_id, set_id);
        }
        if changed {
            debug!(
                set_id,
                beatmaps = new_hashes.len(),
                "Expectation hashes updated"
            );
        } else {
            trace!(set_id, "Expectation hashes unchanged after refresh");
        }
        changed
    }
}

pub(crate) enum ArchiveOutcome {
    Valid {
        beatmapset_id: u32,
    },
    Invalid {
        beatmapset_id: Option<u32>,
        reason: String,
    },
    NotPartOfCollection,
}

pub(crate) async fn verify_download_integrity(
    expected_set_id: u32,
    path: PathBuf,
    expectations: Arc<ExpectationIndex>,
) -> Result<(), ArchiveOutcome> {
    trace!(set_id = expected_set_id, file = %path.display(), "Verifying downloaded archive");
    let path_clone = path.clone();
    let expectation_snapshot = expectations.snapshot();
    let outcome = match task::spawn_blocking(move || {
        inspect_archive(&path_clone, expectation_snapshot.as_ref())
    })
    .await
    {
        Ok(result) => result,
        Err(err) => {
            return Err(ArchiveOutcome::Invalid {
                beatmapset_id: Some(expected_set_id),
                reason: format!("Integrity validation task failed: {}", err),
            });
        }
    };

    match outcome {
        ArchiveOutcome::Valid { beatmapset_id } if beatmapset_id == expected_set_id => Ok(()),
        other => Err(other),
    }
}

pub(crate) fn collect_archive_checksums(path: &Path) -> Result<HashMap<u32, String>, String> {
    trace!(file = %path.display(), "Collecting archive checksums");
    let file = std::fs::File::open(path)
        .map_err(|err| format!("Failed to open archive for checksum refresh: {}", err))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|err| format!("Failed to parse archive for checksum refresh: {}", err))?;

    let mut hashes = HashMap::new();

    for idx in 0..archive.len() {
        let mut entry = archive
            .by_index(idx)
            .map_err(|err| format!("Failed to read archive entry: {}", err))?;

        if !entry.name().ends_with(".osu") {
            continue;
        }

        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|err| format!("Failed to read .osu file: {}", err))?;

        let text = std::str::from_utf8(&data)
            .map_err(|_| "Invalid UTF-8 in .osu file during checksum refresh".to_string())?;

        if let Some(beatmap_id) = extract_beatmap_id(text) {
            let mut hasher = Md5::new();
            hasher.update(&data);
            hashes.insert(beatmap_id, format!("{:032x}", hasher.finalize()));
        }
    }

    Ok(hashes)
}

pub(crate) fn inspect_archive(path: &Path, expectations: &ExpectationData) -> ArchiveOutcome {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) => {
            return ArchiveOutcome::Invalid {
                beatmapset_id: None,
                reason: format!("Failed to open file: {}", err),
            };
        }
    };

    let mut archive = match ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(err) => {
            return ArchiveOutcome::Invalid {
                beatmapset_id: None,
                reason: format!("Invalid archive: {}", err),
            };
        }
    };

    let mut detected_set: Option<u32> = None;
    let mut validated = HashSet::new();

    for idx in 0..archive.len() {
        let mut entry = match archive.by_index(idx) {
            Ok(entry) => entry,
            Err(err) => {
                return ArchiveOutcome::Invalid {
                    beatmapset_id: detected_set,
                    reason: format!("Failed to read archive entry: {}", err),
                };
            }
        };

        if !entry.name().ends_with(".osu") {
            continue;
        }

        let mut data = Vec::new();
        if let Err(err) = entry.read_to_end(&mut data) {
            return ArchiveOutcome::Invalid {
                beatmapset_id: detected_set,
                reason: format!("Failed to read .osu file: {}", err),
            };
        }

        let text = match std::str::from_utf8(&data) {
            Ok(text) => text,
            Err(_) => {
                return ArchiveOutcome::Invalid {
                    beatmapset_id: detected_set,
                    reason: "Invalid UTF-8 in .osu file".to_string(),
                };
            }
        };

        let beatmap_id = match extract_beatmap_id(text) {
            Some(id) => id,
            None => continue,
        };

        let expected_set_id = match expectations.by_beatmap.get(&beatmap_id) {
            Some(set_id) => *set_id,
            None => continue,
        };

        if let Some(existing) = detected_set {
            if existing != expected_set_id {
                return ArchiveOutcome::Invalid {
                    beatmapset_id: Some(existing),
                    reason: format!(
                        "Archive mixes beatmapsets {} and {}",
                        existing, expected_set_id
                    ),
                };
            }
        } else {
            detected_set = Some(expected_set_id);
        }

        if let Some(expectation) = expectations.by_set.get(&expected_set_id) {
            if let Some(expected_checksum) = expectation.beatmap_hashes.get(&beatmap_id) {
                let mut hasher = Md5::new();
                hasher.update(&data);
                let actual_checksum = format!("{:032x}", hasher.finalize());
                if !actual_checksum.eq_ignore_ascii_case(expected_checksum) {
                    return ArchiveOutcome::Invalid {
                        beatmapset_id: Some(expected_set_id),
                        reason: format!("Checksum mismatch for beatmap {}", beatmap_id),
                    };
                }
                validated.insert(beatmap_id);
            }
        }
    }

    let set_id = match detected_set {
        Some(id) => id,
        None => return ArchiveOutcome::NotPartOfCollection,
    };

    let expectation = match expectations.by_set.get(&set_id) {
        Some(exp) if !exp.beatmap_hashes.is_empty() => exp,
        _ => return ArchiveOutcome::NotPartOfCollection,
    };

    for required in expectation.beatmap_hashes.keys() {
        if !validated.contains(required) {
            return ArchiveOutcome::Invalid {
                beatmapset_id: Some(set_id),
                reason: format!("Beatmap {} missing from archive", required),
            };
        }
    }

    ArchiveOutcome::Valid {
        beatmapset_id: set_id,
    }
}

fn extract_beatmap_id(contents: &str) -> Option<u32> {
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("BeatmapID:") {
            if let Ok(value) = rest.trim().parse::<i64>() {
                if value > 0 {
                    return Some(value as u32);
                }
            }
        }
    }
    None
}

impl ArchiveOutcome {
    pub(crate) fn is_checksum_mismatch(&self, expected_set: u32) -> bool {
        matches!(
            self,
            ArchiveOutcome::Invalid {
                beatmapset_id: Some(actual),
                reason,
            } if *actual == expected_set && reason.starts_with("Checksum mismatch")
        )
    }
}
