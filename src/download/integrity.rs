use crate::core::collection::Beatmapset;
use std::{collections::HashSet, sync::Arc};

#[derive(Clone)]
pub(crate) struct ExpectationData {
    pub(crate) by_set: HashSet<u32>,
}

pub(crate) struct ExpectationIndex {
    data: Arc<ExpectationData>,
}

impl ExpectationIndex {
    pub(crate) fn new(beatmapsets: &[Beatmapset]) -> Self {
        let by_set: HashSet<u32> = beatmapsets.iter().map(|set| set.id).collect();
        let data = ExpectationData { by_set };
        Self {
            data: Arc::new(data),
        }
    }

    pub(crate) fn from_ids(ids: &[u32]) -> Self {
        let by_set: HashSet<u32> = ids.iter().copied().collect();
        let data = ExpectationData { by_set };
        Self {
            data: Arc::new(data),
        }
    }

    pub(crate) fn data(&self) -> Arc<ExpectationData> {
        Arc::clone(&self.data)
    }
}
