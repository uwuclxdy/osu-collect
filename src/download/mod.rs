mod cleanup;
mod client;
mod integrity;
mod outstanding;
mod passes;
mod pipeline;
mod precheck;
mod verified;

use client::{DownloadResult, create_download_client, download_beatmap};
pub use pipeline::{spawn_download, spawn_selective_download};
pub(crate) use {
    cleanup::CleanupTracker, outstanding::OutstandingTracker, verified::VerifiedRegistry,
};

use crate::mirrors::MirrorEndpoint;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::task::JoinHandle;

pub type DownloadId = u64;

pub struct DownloadHandle {
    shutdown: Arc<AtomicBool>,
    join_handle: JoinHandle<()>,
}

impl DownloadHandle {
    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    pub async fn wait(self) {
        let _ = self.join_handle.await;
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub collection_input: String,
    pub directory: String,
    pub mirrors: Vec<MirrorEndpoint>,
    pub concurrent: u8,
    pub skip_existing: bool,
    pub auto_overwrite: bool,
}

#[derive(Debug, Clone)]
pub struct SelectiveDownloadRequest {
    pub collection_ids: Vec<u32>,
    pub beatmapset_ids: Vec<u32>,
    pub directory: String,
    pub mirrors: Vec<MirrorEndpoint>,
    pub concurrent: u8,
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
    LowDiskSpace {
        id: DownloadId,
        available_bytes: u64,
    },
    VerifiedMapSizes {
        id: DownloadId,
        total_bytes: u64,
        count: u32,
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
        downloaded: u16,
        skipped: u16,
        failed: u16,
        unverified: u16,
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
    },
    StageChanged {
        id: DownloadId,
        stage: DownloadStage,
    },
    FailedMaps {
        id: DownloadId,
        beatmapset_ids: Vec<u32>,
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
    pub downloaded: u16,
    pub skipped: u16,
    pub failed: u16,
    pub unverified: u16,
    pub unverified_sets: Vec<u32>,
}
