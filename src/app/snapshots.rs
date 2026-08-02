use crate::osu_db::{LocalBeatmapset, LocalCollection, Md5, OsuClient, checksum};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tracing::{debug, warn};

pub const SNAPSHOT_ENV_DIR: &str = "OSU_COLLECT_SNAPSHOT_DIR";
const SNAPSHOT_VERSION: u32 = 1;
static SAVE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionSnapshot {
    #[serde(default)]
    pub stable_hashes: Vec<String>,
    #[serde(default)]
    pub lazer_ids: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionSnapshotFile {
    pub collection_id: String,
    pub name: String,
    pub last_run_at: String,
    pub snapshot: CollectionSnapshot,
    pub version: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotDiff {
    pub manually_deleted: CollectionSnapshot,
    pub manually_added: CollectionSnapshot,
}

impl CollectionSnapshot {
    pub fn len(&self) -> usize {
        self.stable_hashes.len() + self.lazer_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stable_hashes.is_empty() && self.lazer_ids.is_empty()
    }
}

impl CollectionSnapshotFile {
    pub fn new(collection_id: u32, name: String, snapshot: CollectionSnapshot) -> Self {
        Self {
            collection_id: collection_id.to_string(),
            name,
            last_run_at: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_else(|_| "1970-01-01t00:00:00z".to_string()),
            snapshot,
            version: SNAPSHOT_VERSION,
        }
    }
}

pub fn snapshots_dir() -> Option<PathBuf> {
    if let Ok(custom) = env::var(SNAPSHOT_ENV_DIR) {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    platform_data_dir().map(snapshot_dir_in)
}

pub fn snapshot_dir_in(base: PathBuf) -> PathBuf {
    base.join("osu-collect").join("snapshots")
}

#[cfg(windows)]
fn platform_data_dir() -> Option<PathBuf> {
    dirs::data_dir()
}

#[cfg(not(windows))]
fn platform_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
}

pub fn snapshot_path(dir: &Path, collection_id: u32) -> PathBuf {
    dir.join(format!("collection-{collection_id}.json"))
}

pub fn load(path: &Path) -> Option<CollectionSnapshotFile> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            debug!(path = %path.display(), "no collection snapshot found");
            return None;
        }
        Err(err) => {
            warn!(path = %path.display(), error = %err, "failed to read collection snapshot");
            return None;
        }
    };

    let snapshot = match serde_json::from_str::<CollectionSnapshotFile>(&contents) {
        Ok(snapshot) => snapshot,
        Err(err) => {
            warn!(path = %path.display(), error = %err, "failed to parse collection snapshot");
            return None;
        }
    };

    if snapshot.version != SNAPSHOT_VERSION {
        warn!(
            path = %path.display(),
            version = snapshot.version,
            supported = SNAPSHOT_VERSION,
            "unsupported collection snapshot version"
        );
        return None;
    }

    Some(snapshot)
}

pub fn save(snapshot: &CollectionSnapshotFile, path: &Path) {
    let Ok(_guard) = SAVE_LOCK.lock() else {
        warn!(path = %path.display(), "failed to lock collection snapshot save");
        return;
    };

    let contents = match serde_json::to_string_pretty(snapshot) {
        Ok(contents) => contents,
        Err(err) => {
            warn!(error = %err, "failed to serialize collection snapshot");
            return;
        }
    };

    if let Err(err) = super::write_atomic(path, "json.tmp", &contents) {
        warn!(path = %path.display(), error = %err, "failed to save collection snapshot");
    } else {
        debug!(path = %path.display(), "saved collection snapshot");
    }
}

pub fn diff_snapshot(
    previous: Option<&CollectionSnapshot>,
    current: &CollectionSnapshot,
) -> SnapshotDiff {
    let Some(previous) = previous else {
        return SnapshotDiff::default();
    };

    SnapshotDiff {
        manually_deleted: CollectionSnapshot {
            stable_hashes: difference(&previous.stable_hashes, &current.stable_hashes),
            lazer_ids: difference(&previous.lazer_ids, &current.lazer_ids),
        },
        manually_added: CollectionSnapshot {
            stable_hashes: difference(&current.stable_hashes, &previous.stable_hashes),
            lazer_ids: difference(&current.lazer_ids, &previous.lazer_ids),
        },
    }
}

pub fn current_snapshots<'a>(
    client: OsuClient,
    collections: &[LocalCollection],
    beatmapsets: impl IntoIterator<Item = &'a LocalBeatmapset>,
    collection_id_for_name: impl Fn(&str) -> Option<u32>,
) -> HashMap<u32, CollectionSnapshotFile> {
    let checksum_index = checksum_beatmapset_index(beatmapsets);

    collections
        .iter()
        .filter_map(|collection| {
            let collection_id = collection_id_for_name(&collection.name)?;
            let snapshot = match client {
                OsuClient::Stable => CollectionSnapshot {
                    // stable_hashes is Vec<String> for JSON persistence; convert back from Md5
                    stable_hashes: sorted_unique(
                        collection
                            .beatmap_checksums
                            .iter()
                            .map(|&md5| checksum::to_hex(md5))
                            .collect(),
                    ),
                    lazer_ids: Vec::new(),
                },
                OsuClient::Lazer => CollectionSnapshot {
                    stable_hashes: Vec::new(),
                    lazer_ids: sorted_unique(
                        collection
                            .beatmap_checksums
                            .iter()
                            .filter_map(|cksum| checksum_index.get(cksum).copied())
                            .map(u64::from)
                            .collect(),
                    ),
                },
            };
            Some((
                collection_id,
                CollectionSnapshotFile::new(collection_id, collection.name.clone(), snapshot),
            ))
        })
        .collect()
}

/// Fold every entry the old baseline recorded as deleted back into a
/// freshly-built snapshot, so a completed run's baseline still says the user
/// deleted them.
///
/// `manually_deleted` is `previous \ current` ([`diff_snapshot`]). A snapshot is
/// the baseline the next scan diffs against. [`current_snapshots`] reads the
/// LOCAL library, where a deleted set is absent — so writing it verbatim asserts
/// the set is no longer deleted, which is exactly the inference that is wrong:
/// it is absent *because* the user deleted it and the run deliberately did not
/// re-fetch it.
///
/// Rebuilding the baseline rather than withholding the write keeps the rest of
/// the collection current: withholding freezes the whole baseline for as long as
/// anything is held back, so an unrelated local addition keeps re-reporting as
/// "added since last scan" forever. Only the deleted entries are carried over.
///
/// Sourcing the fold from the diff rather than the scan's missing list is what
/// keeps a marked-installed set's deletion recorded: marking it installed drops
/// it from the missing list (and the next scan's ignored-maps gate hides it from
/// the candidate list entirely), but it stays in `manually_deleted` for as long
/// as it is absent from the local library. A fold that read the missing list
/// would lose it; this one cannot.
///
/// **Assumption, and it fails open.** The carried-over hashes are LOCAL
/// collection hashes (the baseline was built from the local library, and the
/// next scan diffs against the same source). It holds only while the local
/// representation of a difficulty's hash is stable across scans. The failure
/// mode is silent and one-directional: if the local library re-hashes a
/// difficulty (e.g. across osu! versions), `manually_deleted` comes back empty
/// and sets the user deleted quietly return rather than erroring. Nothing here
/// can detect that; it needs the comparison keyed on something stable (set id
/// on both sides) instead.
pub fn retain_held_back(
    snapshot: &mut CollectionSnapshot,
    client: OsuClient,
    manually_deleted: &CollectionSnapshot,
) {
    match client {
        OsuClient::Stable => {
            let mut hashes = std::mem::take(&mut snapshot.stable_hashes);
            hashes.extend(manually_deleted.stable_hashes.iter().cloned());
            snapshot.stable_hashes = sorted_unique(hashes);
        }
        OsuClient::Lazer => {
            let mut ids = std::mem::take(&mut snapshot.lazer_ids);
            ids.extend(manually_deleted.lazer_ids.iter().copied());
            snapshot.lazer_ids = sorted_unique(ids);
        }
    }
}

fn checksum_beatmapset_index<'a>(
    beatmapsets: impl IntoIterator<Item = &'a LocalBeatmapset>,
) -> HashMap<Md5, u32> {
    let mut index = HashMap::new();
    for beatmapset in beatmapsets {
        for beatmap in &beatmapset.beatmaps {
            if !checksum::is_empty(&beatmap.checksum) {
                index.insert(beatmap.checksum, beatmapset.id);
            }
        }
    }
    index
}

fn difference<T>(left: &[T], right: &[T]) -> Vec<T>
where
    T: Clone + Eq + std::hash::Hash + Ord,
{
    let right: HashSet<&T> = right.iter().collect();
    let mut values: Vec<T> = left
        .iter()
        .filter(|value| !right.contains(value))
        .cloned()
        .collect();
    values.sort_unstable();
    values.dedup();
    values
}

fn sorted_unique<T: Ord>(mut values: Vec<T>) -> Vec<T> {
    values.sort_unstable();
    values.dedup();
    values
}

#[cfg(test)]
#[path = "../../tests/unit/collection_snapshots.rs"]
mod tests;
