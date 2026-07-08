//! `download_history.json` — past download runs, persisted across launches.
//!
//! A run's record is written the moment it settles (crash-safe), while its
//! full [`CollectionPage`] stays retained for the session so the Downloads tab
//! can still preview it (failed list, recheck). Records for retained pages sit
//! in [`DownloadHistory::pending`] — in the file, hidden from the list — and
//! promote to the visible [`DownloadHistory::records`] when the page drops
//! (cancel, over-cap eviction, app exit). Every removal path funnels through
//! `App::remove_download_page` / the exit flush, so no run is ever lost.

use crate::app::collection::CollectionPage;
use crate::download::{DownloadId, DownloadStage};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, warn};

pub const HISTORY_FILE: &str = "download_history.json";
pub const HISTORY_ENV_PATH: &str = "OSU_COLLECT_DOWNLOAD_HISTORY";
/// Ring cap: bounds the file, the visible records list, and the in-session
/// settled-page retention (each evicts oldest-first past it). The newest runs
/// always win the ring — a session settling ≥cap runs pushes older sessions'
/// records out. The combined Downloads list can transiently show up to
/// retained-pages + records rows (≤ 2×cap past entries).
pub const HISTORY_CAP: usize = 50;
const SCHEMA_VERSION: u32 = 1;

/// How a recorded run ended. Cancel has no [`DownloadStage`] variant — a page
/// removed before reaching a terminal stage is recorded as `Cancelled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryStage {
    Finished,
    Failed,
    Cancelled,
}

impl HistoryStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// One past run, as shown in the Downloads list and persisted to disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub title: String,
    pub stage: HistoryStage,
    pub downloaded: u32,
    pub skipped: u32,
    pub failed: u32,
    pub total_maps: usize,
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Unix seconds when the run settled / was recorded.
    pub finished_at: u64,
}

impl HistoryRecord {
    fn from_page(page: &CollectionPage) -> Self {
        let stage = match page.stage {
            DownloadStage::Completed => HistoryStage::Finished,
            DownloadStage::Failed => HistoryStage::Failed,
            _ => HistoryStage::Cancelled,
        };
        Self {
            title: page.title.clone(),
            stage,
            downloaded: page.stats.downloaded,
            skipped: page.stats.skipped,
            failed: page.stats.failed,
            total_maps: page.total_maps,
            output_dir: page.output_dir.clone(),
            finished_at: now_unix_secs(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryFile {
    schema_version: u32,
    #[serde(default)]
    records: Vec<HistoryRecord>,
}

/// Runtime store over `download_history.json`. `Default` is an in-memory,
/// no-persistence store (what `App::new` starts with — the runtime swaps in
/// the disk-backed store, so tests never touch the user's real file).
#[derive(Debug, Default)]
pub struct DownloadHistory {
    path: Option<PathBuf>,
    /// Past runs shown in the Downloads list, newest first.
    pub records: Vec<HistoryRecord>,
    /// Persisted records for settled runs still retained as live pages —
    /// written to the file for crash safety, hidden from the list until the
    /// page drops. Push order = settle order (oldest first).
    pending: Vec<(DownloadId, HistoryRecord)>,
}

impl DownloadHistory {
    /// Disk-backed store at the default path (env override honored).
    pub fn load() -> Self {
        Self::load_from(history_path())
    }

    /// Disk-backed store at an explicit path; `None` stays in-memory.
    pub fn load_from(path: Option<PathBuf>) -> Self {
        let records = path.as_deref().map(load_records).unwrap_or_default();
        Self {
            path,
            records,
            pending: Vec::new(),
        }
    }

    /// Record a run that reached a terminal stage while its page stays
    /// retained. Overwrites an existing pending record for the same run, so a
    /// late `Finished` refreshes the counts instead of duplicating.
    pub fn record_settled(&mut self, page: &CollectionPage) {
        let record = HistoryRecord::from_page(page);
        match self.pending.iter_mut().find(|(id, _)| *id == page.id) {
            Some((_, existing)) => *existing = record,
            None => self.pending.push((page.id, record)),
        }
        self.save();
    }

    /// Record a page being dropped (cancel, eviction, app exit): promote its
    /// pending record to the visible list, or — for a run that never settled —
    /// write a fresh one (recorded as cancelled).
    pub fn record_removed(&mut self, page: &CollectionPage) {
        let record = match self.pending.iter().position(|(id, _)| *id == page.id) {
            Some(pos) => self.pending.remove(pos).1,
            None => HistoryRecord::from_page(page),
        };
        self.records.insert(0, record);
        self.records.truncate(HISTORY_CAP);
        self.save();
    }

    fn save(&self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        // Pending runs settled most recently, so they lead the ring; the cap
        // drops the oldest visible records first.
        let mut records: Vec<HistoryRecord> =
            self.pending.iter().rev().map(|(_, r)| r.clone()).collect();
        records.extend(self.records.iter().cloned());
        records.truncate(HISTORY_CAP);
        let file = HistoryFile {
            schema_version: SCHEMA_VERSION,
            records,
        };
        let contents = match serde_json::to_string_pretty(&file) {
            Ok(contents) => contents,
            Err(err) => {
                warn!(error = %err, "failed to serialize download history");
                return;
            }
        };
        if let Err(err) = super::write_atomic(path, "json.tmp", &contents) {
            warn!(path = %path.display(), error = %err, "failed to save download history");
        } else {
            debug!(path = %path.display(), "saved download history");
        }
    }
}

pub fn history_path() -> Option<PathBuf> {
    if let Ok(custom) = env::var(HISTORY_ENV_PATH) {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    super::platform_data_dir().map(|base| base.join("osu-collect").join(HISTORY_FILE))
}

fn load_records(path: &Path) -> Vec<HistoryRecord> {
    let Ok(contents) = fs::read_to_string(path) else {
        debug!(path = %path.display(), "no download history file found");
        return Vec::new();
    };
    match serde_json::from_str::<HistoryFile>(&contents) {
        Ok(mut file) => {
            file.records.truncate(HISTORY_CAP);
            file.records
        }
        Err(err) => {
            warn!(path = %path.display(), error = %err, "failed to parse download history");
            Vec::new()
        }
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "../../tests/unit/download_history.rs"]
mod tests;
