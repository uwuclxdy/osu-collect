use super::{SIZE_SAMPLE_CAP, estimate_total_bytes, fetch_collection_sizes, sample_unknown};
use crate::download::DownloadEvent;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[test]
fn sample_keeps_every_id_when_within_the_cap() {
    // A set at or under the cap is probed whole, in order — the estimate is exact.
    assert_eq!(sample_unknown(&[3, 1, 2]), vec![3, 1, 2]);
}

#[test]
fn sample_strides_across_the_id_range_past_the_cap() {
    let unknown: Vec<u32> = (0..200).collect();
    let sample = sample_unknown(&unknown);

    assert_eq!(
        sample.len(),
        SIZE_SAMPLE_CAP,
        "sample caps at SIZE_SAMPLE_CAP"
    );
    // Evenly spread, not the low-id prefix a first-N pick would take: the sample
    // reaches the high end of the id range (a prefix would stop below the cap).
    assert!(
        sample.iter().any(|&id| id as usize >= SIZE_SAMPLE_CAP),
        "stride must reach past a low-id prefix"
    );
}

#[test]
fn sample_covers_every_unknown_id_uses_exact_total() {
    // 3 unknown ids, sample == all 3 → known_sum + sample_bytes verbatim.
    assert_eq!(estimate_total_bytes(10, 300, 3, 3), 310);
}

#[test]
fn sample_smaller_than_unknown_scales_by_mean() {
    // a sample of 2 averaging 100 bytes/set stands in for 10 unknown ids.
    assert_eq!(estimate_total_bytes(0, 200, 2, 10), 1000);
}

#[test]
fn all_missed_sample_contributes_nothing_and_never_divides_by_zero() {
    // sample landed (len 5) but every probe missed, so sample_bytes stayed 0.
    assert_eq!(estimate_total_bytes(500, 0, 5, 20), 500);
}

#[test]
fn empty_sample_falls_back_to_known_sum_only() {
    assert_eq!(estimate_total_bytes(42, 999, 0, 10), 42);
}

#[test]
fn zero_unknown_returns_known_sum_verbatim() {
    assert_eq!(estimate_total_bytes(777, 0, 0, 0), 777);
}

#[tokio::test]
async fn fully_known_ids_emit_the_exact_sum_without_probing() {
    // Every id is already in `known_sizes`, so `unknown` is empty and the fn
    // returns before ever constructing a `SizeFetcher` — no network call.
    let known_sizes: HashMap<u32, u64> = [(1, 100), (2, 200)].into_iter().collect();
    let captured: Arc<Mutex<Option<DownloadEvent>>> = Arc::new(Mutex::new(None));
    let captured_clone = Arc::clone(&captured);
    let emit = move |event: DownloadEvent| *captured_clone.lock().unwrap() = Some(event);

    fetch_collection_sizes(1, &[1, 2], &known_sizes, &emit).await;

    let DownloadEvent::CollectionSizeResolved { total_bytes, .. } =
        captured.lock().unwrap().take().expect("event emitted")
    else {
        panic!("expected CollectionSizeResolved");
    };
    assert_eq!(total_bytes, 300);
}
