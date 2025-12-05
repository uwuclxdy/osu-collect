use crate::download::{BeatmapStage, DownloadId, DownloadStage, DownloadSummary, status};
use std::{
    collections::HashMap,
    collections::VecDeque,
    time::{Duration, Instant},
};

use crate::config::constants::{COMPLETION_PREFIXES, MAX_LOG_LINES, SPEED_STALE_AFTER};

#[derive(Debug, Default, Clone)]
pub struct DownloadStats {
    pub downloaded: u32,
    pub skipped: u32,
    pub failed: u32,
    pub unverified: u32,
    pub bytes_downloaded: u64,
    pub total_collection_bytes: Option<u64>,
    pub verified_bytes: u64,
}

pub struct BeatmapRow {
    pub stage: BeatmapStage,
    pub message: String,
    pub progress: Option<(u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct FailedBeatmap {
    pub id: u32,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ThreadStatusLine {
    pub message: String,
    pub rate_limited: bool,
    bytes_downloaded: u64,
    last_update: Option<Instant>,
    speed_bytes_per_sec: f64,
    active_beatmap: Option<u32>,
}

impl ThreadStatusLine {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            rate_limited: false,
            bytes_downloaded: 0,
            last_update: None,
            speed_bytes_per_sec: 0.0,
            active_beatmap: None,
        }
    }

    pub fn speed_bytes_per_sec(&self) -> f64 {
        match self.last_update {
            Some(last) if last.elapsed() <= SPEED_STALE_AFTER => self.speed_bytes_per_sec,
            _ => 0.0,
        }
    }

    pub fn is_completion_message(message: &str) -> bool {
        COMPLETION_PREFIXES
            .iter()
            .any(|prefix| message.starts_with(prefix))
    }

    pub fn is_idle_message(message: &str) -> bool {
        message.trim().eq_ignore_ascii_case("idle")
    }

    pub fn should_display(&self) -> bool {
        !(Self::is_idle_message(&self.message) || Self::is_completion_message(&self.message))
    }
}

pub struct CollectionPage {
    pub id: DownloadId,
    pub title: String,
    pub stage: DownloadStage,
    pub total_maps: usize,
    pub download_target: usize,
    pub stats: DownloadStats,
    pub output_dir: Option<String>,
    pub uploader: Option<String>,
    pub beatmaps: Vec<BeatmapRow>,
    pub thread_statuses: Vec<ThreadStatusLine>,
    index: HashMap<u32, usize>,
    pub logs: VecDeque<String>,
    pub summary: Option<DownloadSummary>,
    pub failed_maps: Vec<FailedBeatmap>,
    pub progress_label_style_locked: bool,
    pub progress_label_bold_when_locked: bool,
    pub low_disk_space: Option<u64>,
}

impl CollectionPage {
    pub fn new(id: DownloadId, title: String, concurrent: usize) -> Self {
        Self {
            id,
            title,
            stage: DownloadStage::Pending,
            total_maps: 0,
            download_target: 0,
            stats: DownloadStats::default(),
            output_dir: None,
            uploader: None,
            beatmaps: Vec::new(),
            thread_statuses: (0..concurrent)
                .map(|_| ThreadStatusLine::new("Idle"))
                .collect(),
            index: HashMap::new(),
            logs: VecDeque::new(),
            summary: None,
            failed_maps: Vec::new(),
            progress_label_style_locked: false,
            progress_label_bold_when_locked: true,
            low_disk_space: None,
        }
    }

    pub fn all_threads_rate_limited(&self) -> bool {
        !self.thread_statuses.is_empty()
            && self
                .thread_statuses
                .iter()
                .all(|status| status.rate_limited)
    }

    pub fn register_beatmaps(&mut self, ids: &[u32]) {
        self.beatmaps.clear();
        self.index.clear();
        self.failed_maps.clear();
        for (idx, beatmapset_id) in ids.iter().copied().enumerate() {
            self.index.insert(beatmapset_id, idx);
            self.beatmaps.push(BeatmapRow {
                stage: BeatmapStage::Pending,
                message: String::from("Waiting"),
                progress: None,
            });
        }
    }

    pub fn update_progress(&mut self, beatmapset_id: u32, downloaded: u64, _total: u64) {
        if let Some(idx) = self.index.get(&beatmapset_id).copied()
            && let Some(row) = self.beatmaps.get_mut(idx)
        {
            if let Some((prev_downloaded, _)) = row.progress {
                self.stats.bytes_downloaded =
                    self.stats.bytes_downloaded.saturating_sub(prev_downloaded) + downloaded;
            } else {
                self.stats.bytes_downloaded += downloaded;
            }
            row.progress = Some((downloaded, _total));
        }
    }

    pub fn update_status(&mut self, beatmapset_id: u32, stage: BeatmapStage, message: &str) {
        if let Some(idx) = self.index.get(&beatmapset_id).copied()
            && let Some(row) = self.beatmaps.get_mut(idx)
        {
            row.stage = stage;
            row.message = message.to_string();
            if matches!(
                stage,
                BeatmapStage::Success
                    | BeatmapStage::Skipped
                    | BeatmapStage::Failed
                    | BeatmapStage::Aborted
            ) {
                row.progress = None;
            }
        }
    }

    pub fn push_log(&mut self, message: &str) {
        self.logs.push_front(message.to_string());
        while self.logs.len() > MAX_LOG_LINES {
            self.logs.pop_back();
        }
    }

    pub fn update_thread_status(
        &mut self,
        thread_index: usize,
        message: &str,
        rate_limited: bool,
        beatmapset_id: Option<u32>,
    ) {
        if let Some(status) = self.thread_statuses.get_mut(thread_index) {
            let is_assignment = message.starts_with(status::DOWNLOADING)
                || message.starts_with(status::RECHECKING_PREFIX);
            let is_completion = ThreadStatusLine::is_completion_message(message);

            if let Some(job_id) = beatmapset_id {
                if !is_assignment {
                    if let Some(current) = status.active_beatmap {
                        if current != job_id {
                            return;
                        }
                    } else if is_completion {
                        return;
                    }
                }
                status.active_beatmap = Some(job_id);
            } else if is_completion {
                return;
            }

            status.message = message.to_string();
            status.rate_limited = rate_limited;

            if (beatmapset_id.is_some() && is_completion)
                || ThreadStatusLine::is_idle_message(message)
            {
                status.active_beatmap = None;
            }
        }
    }

    pub fn update_thread_progress(&mut self, thread_index: usize, downloaded: u64) {
        if let Some(status) = self.thread_statuses.get_mut(thread_index) {
            let now = Instant::now();
            if let Some(last_update) = status.last_update {
                let elapsed = now.duration_since(last_update);
                if elapsed > Duration::from_millis(50) {
                    let bytes_delta = downloaded.saturating_sub(status.bytes_downloaded);
                    let speed = bytes_delta as f64 / elapsed.as_secs_f64();
                    status.speed_bytes_per_sec = status.speed_bytes_per_sec * 0.7 + speed * 0.3;
                    status.bytes_downloaded = downloaded;
                    status.last_update = Some(now);
                }
            } else {
                status.bytes_downloaded = downloaded;
                status.last_update = Some(now);
            }
        }
    }

    pub fn reset_thread_speed(&mut self, thread_index: usize) {
        if let Some(status) = self.thread_statuses.get_mut(thread_index) {
            status.bytes_downloaded = 0;
            status.last_update = None;
            status.speed_bytes_per_sec = 0.0;
        }
    }

    pub fn cumulative_speed(&self) -> f64 {
        self.thread_statuses
            .iter()
            .map(|s| s.speed_bytes_per_sec())
            .sum()
    }

    pub fn set_failed_maps(&mut self, failures: Vec<(u32, String)>) {
        let mut entries: Vec<FailedBeatmap> = failures
            .into_iter()
            .map(|(id, reason)| FailedBeatmap { id, reason })
            .collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        self.failed_maps = entries;
    }

    pub fn total_downloaded_bytes(&self) -> u64 {
        self.stats
            .bytes_downloaded
            .saturating_add(self.stats.verified_bytes)
    }
}
