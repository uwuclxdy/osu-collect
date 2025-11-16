use std::{collections::HashSet, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub(crate) struct OutstandingTracker {
    inner: Arc<Mutex<HashSet<u32>>>,
}

impl OutstandingTracker {
    pub(crate) fn new(initial: HashSet<u32>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(initial)),
        }
    }

    pub(crate) async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    pub(crate) async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    pub(crate) async fn remove_all<I>(&self, ids: I) -> usize
    where
        I: IntoIterator<Item = u32>,
    {
        let mut guard = self.inner.lock().await;
        for id in ids {
            guard.remove(&id);
        }
        guard.len()
    }

    pub(crate) async fn remove(&self, id: u32) -> Option<usize> {
        let mut guard = self.inner.lock().await;
        if guard.remove(&id) {
            Some(guard.len())
        } else {
            None
        }
    }

    pub(crate) async fn snapshot(&self) -> Vec<u32> {
        self.inner.lock().await.iter().copied().collect()
    }
}
