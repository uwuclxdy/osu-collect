use dashmap::DashMap;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum BeatmapState {
    Pending,
    InProgress,
    Verified,
    Failed,
}

#[derive(Clone, Debug)]
struct BeatmapEntry {
    state: BeatmapState,
    metadata: BeatmapMetadata,
}

impl BeatmapEntry {
    fn new(state: BeatmapState) -> Self {
        Self {
            state,
            metadata: BeatmapMetadata::default(),
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct BeatmapMetadata {
    pub path: Option<PathBuf>,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub file_id: Option<FileIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ValidationCacheKey {
    FileId {
        file_id: FileIdentity,
        size: u64,
    },
    Path {
        path: PathBuf,
        size: u64,
        mtime: Option<SystemTime>,
    },
}

#[derive(Default)]
struct TrackerCounters {
    pending: AtomicUsize,
    verified: AtomicUsize,
    failed: AtomicUsize,
}

impl TrackerCounters {
    fn pending(&self) -> usize {
        self.pending.load(Ordering::Relaxed)
    }

    fn inc_pending(&self) {
        self.pending.fetch_add(1, Ordering::Relaxed);
    }

    fn dec_pending(&self) {
        Self::dec(&self.pending);
    }

    fn inc_verified(&self) {
        self.verified.fetch_add(1, Ordering::Relaxed);
    }

    fn dec_verified(&self) {
        Self::dec(&self.verified);
    }

    fn inc_failed(&self) {
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    fn dec_failed(&self) {
        Self::dec(&self.failed);
    }

    fn dec(counter: &AtomicUsize) {
        let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            (current > 0).then_some(current - 1)
        });
    }
}

#[derive(Clone)]
pub struct BeatmapTracker {
    inner: Arc<DashMap<u32, BeatmapEntry>>,
    validation_cache: Arc<DashMap<ValidationCacheKey, bool>>,
    counters: Arc<TrackerCounters>,
}

impl Default for BeatmapTracker {
    fn default() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            validation_cache: Arc::new(DashMap::new()),
            counters: Arc::new(TrackerCounters::default()),
        }
    }
}

#[allow(dead_code)]
impl BeatmapTracker {
    pub fn new(initial: HashSet<u32>) -> Self {
        let counters = Arc::new(TrackerCounters::default());
        let inner = Arc::new(DashMap::with_capacity(initial.len()));
        for id in initial {
            inner.insert(id, BeatmapEntry::new(BeatmapState::Pending));
            counters.inc_pending();
        }
        Self {
            inner,
            validation_cache: Arc::new(DashMap::new()),
            counters,
        }
    }

    pub fn with_verified(initial: HashSet<u32>, verified: HashSet<u32>) -> Self {
        let total_capacity = initial.len() + verified.len();
        let counters = Arc::new(TrackerCounters::default());
        let inner = Arc::new(DashMap::with_capacity(total_capacity));
        for id in initial {
            inner.insert(id, BeatmapEntry::new(BeatmapState::Pending));
            counters.inc_pending();
        }
        for id in verified {
            inner.insert(id, BeatmapEntry::new(BeatmapState::Verified));
            counters.inc_verified();
        }
        Self {
            inner,
            validation_cache: Arc::new(DashMap::new()),
            counters,
        }
    }

    pub fn pending_count(&self) -> usize {
        self.counters.pending()
    }

    pub fn is_all_complete(&self) -> bool {
        self.pending_count() == 0
            && self.inner.iter().all(|entry| {
                matches!(
                    entry.value().state,
                    BeatmapState::Verified | BeatmapState::Failed
                )
            })
    }

    pub fn mark_verified(&self, id: u32) -> bool {
        if let Some(mut entry) = self.inner.get_mut(&id) {
            self.transition(entry.state, BeatmapState::Verified);
            entry.state = BeatmapState::Verified;
            true
        } else {
            self.inner
                .insert(id, BeatmapEntry::new(BeatmapState::Verified));
            self.counters.inc_verified();
            true
        }
    }

    pub fn mark_verified_batch<I>(&self, ids: I) -> usize
    where
        I: IntoIterator<Item = u32>,
    {
        let mut count = 0;
        for id in ids {
            if self.mark_verified(id) {
                count += 1;
            }
        }
        count
    }

    pub fn mark_failed(&self, id: u32) -> bool {
        if let Some(mut entry) = self.inner.get_mut(&id) {
            self.transition(entry.state, BeatmapState::Failed);
            entry.state = BeatmapState::Failed;
            true
        } else {
            false
        }
    }

    pub fn mark_pending(&self, id: u32) -> bool {
        if let Some(mut entry) = self.inner.get_mut(&id) {
            if entry.state == BeatmapState::Failed {
                self.transition(BeatmapState::Failed, BeatmapState::Pending);
                entry.state = BeatmapState::Pending;
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn is_pending(&self, id: u32) -> bool {
        self.inner
            .get(&id)
            .is_some_and(|entry| entry.state == BeatmapState::Pending)
    }

    pub fn is_verified(&self, id: u32) -> bool {
        self.inner
            .get(&id)
            .is_some_and(|entry| entry.state == BeatmapState::Verified)
    }

    pub fn remove_pending(&self, id: u32) -> Option<usize> {
        if let Some(mut entry) = self.inner.get_mut(&id)
            && entry.state == BeatmapState::Pending
        {
            entry.state = BeatmapState::InProgress;
            self.counters.dec_pending();
            return Some(self.pending_count());
        }
        None
    }

    pub fn pending_snapshot(&self) -> Vec<u32> {
        self.inner
            .iter()
            .filter(|entry| entry.value().state == BeatmapState::Pending)
            .map(|entry| *entry.key())
            .collect()
    }

    pub fn check_validation_cache(
        &self,
        path: &Path,
        size: u64,
        mtime: Option<SystemTime>,
    ) -> Option<bool> {
        self.check_validation_cache_with_identity(path, size, mtime, None)
    }

    pub fn check_validation_cache_with_identity(
        &self,
        path: &Path,
        size: u64,
        mtime: Option<SystemTime>,
        file_id: Option<FileIdentity>,
    ) -> Option<bool> {
        let key = ValidationCacheKey::from_lookup(path, size, mtime, file_id);
        self.validation_cache.get(&key).map(|entry| *entry)
    }

    pub fn cache_validation_result(
        &self,
        path: PathBuf,
        size: u64,
        mtime: Option<SystemTime>,
        valid: bool,
    ) {
        self.cache_validation_result_with_identity(path, size, mtime, None, valid);
    }

    pub fn cache_validation_result_with_identity(
        &self,
        path: PathBuf,
        size: u64,
        mtime: Option<SystemTime>,
        file_id: Option<FileIdentity>,
        valid: bool,
    ) {
        let key = ValidationCacheKey::from_owned(path, size, mtime, file_id);
        self.validation_cache.insert(key, valid);
    }

    pub fn invalidate_cache(&self, path: &Path) {
        self.invalidate_cache_with_identity(path, None);
    }

    pub fn invalidate_cache_with_identity(&self, path: &Path, file_id: Option<FileIdentity>) {
        self.validation_cache.retain(|key, _| match key {
            ValidationCacheKey::Path {
                path: cached_path, ..
            } => cached_path != path,
            ValidationCacheKey::FileId {
                file_id: cached_id, ..
            } => file_id.map(|target| target != *cached_id).unwrap_or(true),
        });
    }

    pub fn metadata(&self, id: u32) -> Option<BeatmapMetadata> {
        self.inner.get(&id).map(|entry| entry.metadata.clone())
    }

    pub fn set_metadata(&self, id: u32, metadata: BeatmapMetadata) {
        if let Some(mut entry) = self.inner.get_mut(&id) {
            entry.metadata = metadata;
        }
    }

    fn transition(&self, from: BeatmapState, to: BeatmapState) {
        if from == to {
            return;
        }

        match from {
            BeatmapState::Pending => self.counters.dec_pending(),
            BeatmapState::Verified => self.counters.dec_verified(),
            BeatmapState::Failed => self.counters.dec_failed(),
            BeatmapState::InProgress => {}
        }

        match to {
            BeatmapState::Pending => self.counters.inc_pending(),
            BeatmapState::Verified => self.counters.inc_verified(),
            BeatmapState::Failed => self.counters.inc_failed(),
            BeatmapState::InProgress => {}
        }
    }
}

impl ValidationCacheKey {
    fn from_lookup(
        path: &Path,
        size: u64,
        mtime: Option<SystemTime>,
        file_id: Option<FileIdentity>,
    ) -> Self {
        if let Some(file_id) = file_id {
            ValidationCacheKey::FileId { file_id, size }
        } else {
            ValidationCacheKey::Path {
                path: path.to_path_buf(),
                size,
                mtime,
            }
        }
    }

    fn from_owned(
        path: PathBuf,
        size: u64,
        mtime: Option<SystemTime>,
        file_id: Option<FileIdentity>,
    ) -> Self {
        if let Some(file_id) = file_id {
            ValidationCacheKey::FileId { file_id, size }
        } else {
            ValidationCacheKey::Path { path, size, mtime }
        }
    }
}
