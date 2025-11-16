use std::{
    collections::HashSet,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::fs;

#[derive(Clone, Default)]
pub(crate) struct CleanupTracker {
    pending: Arc<Mutex<HashSet<PathBuf>>>,
}

pub(crate) struct CleanupOutcome {
    pub removed: usize,
    pub failures: Vec<(PathBuf, String)>,
}

impl CleanupTracker {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn track(&self, path: &Path) {
        let mut guard = self.pending.lock().expect("cleanup tracker poisoned");
        guard.insert(path.to_path_buf());
    }

    pub fn mark_complete(&self, path: &Path) {
        let mut guard = self.pending.lock().expect("cleanup tracker poisoned");
        guard.remove(path);
    }

    pub fn mark_removed(&self, path: &Path) {
        let mut guard = self.pending.lock().expect("cleanup tracker poisoned");
        guard.remove(path);
    }

    pub async fn cleanup_incomplete(&self) -> CleanupOutcome {
        let paths: Vec<PathBuf> = {
            let mut guard = self.pending.lock().expect("cleanup tracker poisoned");
            guard.drain().collect()
        };

        let mut removed = 0;
        let mut failures = Vec::new();

        for path in paths {
            match fs::remove_file(&path).await {
                Ok(_) => removed += 1,
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => failures.push((path.clone(), err.to_string())),
            }
        }

        CleanupOutcome { removed, failures }
    }
}
