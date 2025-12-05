mod cleanup;
pub(crate) mod client;
pub mod error;
pub mod http_client;
mod integrity;
mod passes;
mod pipeline;
mod precheck;
mod size_fetcher;
mod tracker;

pub use cleanup::CleanupTracker;
use client::download_beatmap;
pub use client::{DownloadResult, StatusReporter, create_download_client};
pub use error::DownloadError;
pub use pipeline::{spawn_download, spawn_selective_download};
pub use size_fetcher::check_mirror_availability;
pub use tracker::BeatmapTracker;

pub use crate::config::constants::status;

use crate::mirrors::MirrorEndpoint;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::task::JoinHandle;
use tracing::warn;

#[derive(Clone, Debug, Default)]
pub struct ShutdownToken {
    cancelled: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
}

impl ShutdownToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            completed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        if self.completed.load(Ordering::Acquire) {
            warn!("ShutdownToken: cancel called after completion");
        }
        if self.cancelled.swap(true, Ordering::SeqCst) {
            warn!("ShutdownToken: cancel called multiple times");
        }
    }

    pub fn mark_completed(&self) {
        self.completed.store(true, Ordering::Release);
    }
}

#[macro_export]
macro_rules! check_shutdown {
    ($token:expr) => {
        if ($token).is_cancelled() {
            return Ok($crate::download::DownloadResult::Aborted);
        }
    };
}

pub type DownloadId = u64;

pub struct DownloadHandle {
    shutdown: ShutdownToken,
    join_handle: JoinHandle<()>,
}

impl DownloadHandle {
    pub fn request_shutdown(&self) {
        self.shutdown.cancel();
    }

    pub async fn wait(self) {
        let _ = self.join_handle.await;
    }
}

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub directory: String,
    pub mirrors: Vec<MirrorEndpoint>,
    pub concurrent: u8,
    pub verify_zip_eocd: bool,
    pub max_retries: u8,
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub collection_input: String,
    pub config: DownloadConfig,
    pub skip_existing: bool,
    pub auto_overwrite: bool,
}

#[derive(Debug, Clone)]
pub struct SelectiveDownloadRequest {
    pub collection_ids: Vec<u32>,
    pub beatmapset_ids: Vec<u32>,
    pub config: DownloadConfig,
}

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    CollectionReady {
        id: DownloadId,
        collection_name: String,
        uploader: String,
        total_maps: usize,
        output_dir: String,
    },
    CollectionSizeResolved {
        id: DownloadId,
        total_bytes: u64,
    },
    LowDiskSpace {
        id: DownloadId,
        available_bytes: u64,
    },
    VerifiedMapSizes {
        id: DownloadId,
        total_bytes: u64,
    },
    BeatmapsRegistered {
        id: DownloadId,
        beatmap_ids: Vec<u32>,
    },
    BeatmapProgress {
        id: DownloadId,
        beatmapset_id: u32,
        thread_index: usize,
        downloaded: u64,
        total: u64,
    },
    DownloadTarget {
        id: DownloadId,
        remaining: usize,
    },
    BeatmapStatus {
        id: DownloadId,
        beatmapset_id: u32,
        stage: BeatmapStage,
        message: String,
    },
    OverallProgress {
        id: DownloadId,
        downloaded: u32,
        skipped: u32,
        failed: u32,
        unverified: u32,
    },
    Log {
        id: DownloadId,
        message: String,
    },
    ThreadStatus {
        id: DownloadId,
        thread_index: usize,
        message: String,
        rate_limited: bool,
        beatmapset_id: Option<u32>,
    },
    StageChanged {
        id: DownloadId,
        stage: DownloadStage,
    },
    FailedMaps {
        id: DownloadId,
        failures: Vec<(u32, String)>,
    },
    Finished {
        id: DownloadId,
        summary: DownloadSummary,
    },
    Failed {
        id: DownloadId,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatmapStage {
    Pending,
    Downloading,
    Success,
    Skipped,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStage {
    Pending,
    Resolving,
    Rechecking,
    Downloading,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct DownloadSummary {
    pub downloaded: u32,
    pub skipped: u32,
    pub failed: u32,
    pub unverified: u32,
}
