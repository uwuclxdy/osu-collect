use dashmap::DashSet;
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::fs;
use tracing::{info, warn};

#[derive(Clone, Default)]
pub struct CleanupTracker {
    pending: Arc<DashSet<PathBuf>>,
}

pub struct CleanupOutcome {
    pub removed: usize,
    pub failures: Vec<(PathBuf, String)>,
}

impl CleanupTracker {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(DashSet::new()),
        }
    }

    pub fn track(&self, path: &Path) {
        let path_buf = path.to_path_buf();
        if !self.pending.insert(path_buf.clone()) {
            info!(path = %path_buf.display(), "CleanupTracker: path already tracked");
        }
    }

    pub fn mark_complete(&self, path: &Path) {
        if self.pending.remove(path).is_none() {
            warn!(path = %path.display(), "CleanupTracker: mark_complete called for untracked path");
        }
    }

    pub fn mark_removed(&self, path: &Path) {
        self.pending.remove(path);
    }

    pub async fn cleanup_incomplete(&self) -> CleanupOutcome {
        let paths: Vec<PathBuf> = self.pending.iter().map(|r| r.clone()).collect();
        let mut removed = 0;
        let mut failures = Vec::new();

        for path in paths {
            match fs::remove_file(&path).await {
                Ok(_) => {
                    removed += 1;
                    self.pending.remove(&path);
                }
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    self.pending.remove(&path);
                }
                Err(err) => failures.push((path.clone(), err.to_string())),
            }
        }

        CleanupOutcome { removed, failures }
    }
}
