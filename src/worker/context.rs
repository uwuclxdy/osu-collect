use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::download::{BeatmapTracker, CleanupTracker, DownloadEvent, DownloadId, ShutdownToken};
use dashmap::DashSet;
use tokio::sync::mpsc::UnboundedSender;

use super::MirrorPool;

#[derive(Clone)]
pub struct StatusSink {
    inner: Arc<dyn Fn(DownloadEvent) + Send + Sync>,
}

impl StatusSink {
    pub fn from_sender(tx: UnboundedSender<DownloadEvent>) -> Self {
        Self::from_fn(move |event| {
            let _ = tx.send(event);
        })
    }

    pub fn from_fn<F>(callback: F) -> Self
    where
        F: Fn(DownloadEvent) + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(callback),
        }
    }

    pub fn noop() -> Self {
        Self::from_fn(|_| {})
    }

    pub fn emit(&self, event: DownloadEvent) {
        (self.inner)(event);
    }
}

impl Default for StatusSink {
    fn default() -> Self {
        Self::noop()
    }
}

#[derive(Clone)]
pub struct DownloadContext {
    pub id: DownloadId,
    pub thread_count: usize,
    pub skip_existing: bool,
    pub auto_overwrite: bool,
    pub max_retries: u8,
    pub verify_zip_eocd: bool,
    pub shutdown: ShutdownToken,
    pub client: reqwest::Client,
    pub mirror_pool: MirrorPool,
    pub output_dir: Arc<PathBuf>,
    pub tracker: BeatmapTracker,
    pub cleanup_tracker: CleanupTracker,
    pub initial_unverified: Arc<DashSet<u32>>,
    pub status: StatusSink,
    pub progress_watchdog: Duration,
}

impl DownloadContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: DownloadId,
        thread_count: usize,
        skip_existing: bool,
        auto_overwrite: bool,
        verify_zip_eocd: bool,
        max_retries: u8,
        shutdown: ShutdownToken,
        client: reqwest::Client,
        mirror_pool: MirrorPool,
        output_dir: PathBuf,
        tracker: BeatmapTracker,
        initial_unverified: Arc<DashSet<u32>>,
        status: StatusSink,
        progress_watchdog: Duration,
    ) -> Self {
        Self::with_status_sink(
            id,
            thread_count,
            skip_existing,
            auto_overwrite,
            verify_zip_eocd,
            max_retries,
            shutdown,
            client,
            mirror_pool,
            output_dir,
            tracker,
            initial_unverified,
            status,
            progress_watchdog,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_status_sink(
        id: DownloadId,
        thread_count: usize,
        skip_existing: bool,
        auto_overwrite: bool,
        verify_zip_eocd: bool,
        max_retries: u8,
        shutdown: ShutdownToken,
        client: reqwest::Client,
        mirror_pool: MirrorPool,
        output_dir: PathBuf,
        tracker: BeatmapTracker,
        initial_unverified: Arc<DashSet<u32>>,
        status: StatusSink,
        progress_watchdog: Duration,
    ) -> Self {
        Self {
            id,
            thread_count,
            skip_existing,
            auto_overwrite,
            verify_zip_eocd,
            max_retries,
            shutdown,
            client,
            mirror_pool,
            output_dir: Arc::new(output_dir),
            tracker,
            cleanup_tracker: CleanupTracker::new(),
            initial_unverified,
            status,
            progress_watchdog,
        }
    }

    pub fn emit(&self, event: DownloadEvent) {
        self.status.emit(event);
    }

    pub fn status_sink(&self) -> StatusSink {
        self.status.clone()
    }

    pub fn consume_unverified(&self, beatmapset_id: u32) -> bool {
        self.initial_unverified.remove(&beatmapset_id).is_some()
    }
}
