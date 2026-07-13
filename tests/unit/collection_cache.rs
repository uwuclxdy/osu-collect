use super::{CollectionCache, TTL};
use crate::core::collection::{test_beatmapset, test_collection};
use std::time::{Duration, Instant};

fn collection(id: u32) -> crate::core::collection::Collection {
    test_collection(id, vec![test_beatmapset(id * 10, &["abc"])])
}

#[test]
fn fresh_entry_hits() {
    let mut cache = CollectionCache::default();
    cache.insert(7, collection(7));

    let hit = cache.get_fresh(7).expect("a just-inserted entry is fresh");
    assert_eq!(hit.id, 7);
    assert_eq!(hit.beatmapsets.len(), 1);
}

#[test]
fn absent_entry_misses() {
    let cache = CollectionCache::default();
    assert!(cache.get_fresh(7).is_none());
}

#[test]
fn expired_entry_misses() {
    let now = Instant::now();
    let mut cache = CollectionCache::default();
    cache.insert_at(7, collection(7), now);

    assert!(
        cache
            .get_fresh_at(7, now + TTL - Duration::from_secs(1))
            .is_some(),
        "still inside the TTL"
    );
    assert!(
        cache.get_fresh_at(7, now + TTL).is_none(),
        "a payload at exactly the TTL is stale — the pipeline refetches"
    );
}

#[test]
fn insert_prunes_expired_entries() {
    let now = Instant::now();
    let mut cache = CollectionCache::default();
    cache.insert_at(1, collection(1), now);
    cache.insert_at(2, collection(2), now + Duration::from_secs(1));

    let later = now + TTL + Duration::from_secs(1);
    cache.insert_at(3, collection(3), later);

    // 1 expired before the insert and is gone; 2 was still fresh at `later - 1s`
    // but not at `later`, so it drops too. Only the new entry survives.
    assert_eq!(cache.entries.len(), 1);
    assert!(cache.entries.contains_key(&3));
}

#[test]
fn insert_keeps_still_fresh_entries() {
    let now = Instant::now();
    let mut cache = CollectionCache::default();
    cache.insert_at(1, collection(1), now);
    cache.insert_at(2, collection(2), now + Duration::from_secs(60));

    assert_eq!(cache.entries.len(), 2);
    assert!(
        cache
            .get_fresh_at(1, now + Duration::from_secs(60))
            .is_some(),
        "the prune must not evict an entry inside its TTL"
    );
}
