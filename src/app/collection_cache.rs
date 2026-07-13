//! Session-lived osu!collector payload cache.
//!
//! The resolve (Get Maps collection field) and the update scan both fetch a full
//! [`Collection`] — ids, per-diff checksums, name, uploader — purely for display.
//! Handing that payload to the download request lets the pipeline skip an
//! identical refetch seconds later. The TTL only guards a form left open for
//! hours; a selective run downloads the id set its scan produced, so reusing that
//! same scan's payload is more consistent than pairing a stale id set with fresh
//! checksums.

use crate::core::collection::Collection;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Default)]
pub struct CollectionCache {
    entries: HashMap<u32, (Collection, Instant)>,
}

impl CollectionCache {
    /// Store a freshly fetched payload, dropping every expired entry first — a
    /// long session scanning many collections would otherwise hold MBs of
    /// checksums it can no longer use.
    pub fn insert(&mut self, id: u32, collection: Collection) {
        self.insert_at(id, collection, Instant::now());
    }

    /// The cached payload for `id`, or `None` when absent or past its TTL.
    pub fn get_fresh(&self, id: u32) -> Option<&Collection> {
        self.get_fresh_at(id, Instant::now())
    }

    fn insert_at(&mut self, id: u32, collection: Collection, now: Instant) {
        self.entries
            .retain(|_, (_, fetched)| now.duration_since(*fetched) < TTL);
        self.entries.insert(id, (collection, now));
    }

    fn get_fresh_at(&self, id: u32, now: Instant) -> Option<&Collection> {
        self.entries
            .get(&id)
            .filter(|(_, fetched)| now.duration_since(*fetched) < TTL)
            .map(|(collection, _)| collection)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/collection_cache.rs"]
mod tests;
