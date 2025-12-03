use crate::core::collection::Beatmapset;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tracing::trace;

#[derive(Clone)]
pub(crate) struct ExpectationData {
    pub(crate) by_set: HashSet<u32>,
}

pub(crate) struct ExpectationIndex {
    data: RwLock<Arc<ExpectationData>>,
}

impl ExpectationIndex {
    pub(crate) fn new(beatmapsets: &[Beatmapset]) -> Self {
        let by_set: HashSet<u32> = beatmapsets.iter().map(|set| set.id).collect();
        let data = ExpectationData { by_set };
        Self {
            data: RwLock::new(Arc::new(data)),
        }
    }

    pub(crate) fn snapshot(&self) -> Arc<ExpectationData> {
        self.data.read().expect("ExpectationIndex poisoned").clone()
    }
}

pub(crate) async fn verify_download_integrity(
    expected_set_id: u32,
    path: PathBuf,
    _expectations: Arc<ExpectationIndex>,
) {
    trace!(set_id = expected_set_id, file = %path.display(), "Accepting downloaded archive without zip validation");
}
