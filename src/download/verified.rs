use std::{
    collections::HashSet,
    sync::{Arc, RwLock},
};

#[derive(Clone, Default)]
pub(crate) struct VerifiedRegistry {
    inner: Arc<RwLock<HashSet<u32>>>,
}

impl VerifiedRegistry {
    pub(crate) fn new(initial: HashSet<u32>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(initial)),
        }
    }

    pub(crate) fn insert(&self, id: u32) -> bool {
        self.inner
            .write()
            .expect("verified registry poisoned")
            .insert(id)
    }

    pub(crate) fn contains(&self, id: u32) -> bool {
        self.inner
            .read()
            .expect("verified registry poisoned")
            .contains(&id)
    }
}
