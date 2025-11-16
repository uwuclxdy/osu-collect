use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};

use crate::download::{CleanupTracker, VerifiedRegistry};

#[derive(Clone)]
pub struct DownloadContext {
    pub client: reqwest::Client,
    pub output_dir: Arc<PathBuf>,
    pub skip_existing: bool,
    pub auto_overwrite: bool,
    pub shutdown: Arc<AtomicBool>,
    pub verified_registry: Option<VerifiedRegistry>,
    pub cleanup_tracker: CleanupTracker,
}

impl DownloadContext {
    pub fn new(
        client: reqwest::Client,
        output_dir: Arc<PathBuf>,
        skip_existing: bool,
        auto_overwrite: bool,
        shutdown: Arc<AtomicBool>,
        verified_registry: Option<VerifiedRegistry>,
        cleanup_tracker: CleanupTracker,
    ) -> Self {
        Self {
            client,
            output_dir,
            skip_existing,
            auto_overwrite,
            shutdown,
            verified_registry,
            cleanup_tracker,
        }
    }
}
