//! Cover-store unit tests: the ready/missing/pending lookup and the tick-based
//! prefetch debounce. All headless — the halfblocks picker needs no terminal
//! and the protocols are built from tiny in-memory images, no network.

use super::*;

/// A 2x2 solid image → a cheap [`StatefulProtocol`] via the halfblocks picker.
fn tiny_protocol(covers: &Covers) -> ratatui_image::protocol::StatefulProtocol {
    let img = image::RgbImage::from_pixel(2, 2, image::Rgb([10, 20, 30]));
    covers
        .picker
        .new_resize_protocol(image::DynamicImage::ImageRgb8(img))
}

#[test]
fn protocol_for_returns_a_handle_only_for_ready_covers() {
    let mut covers = Covers::new();
    let protocol = tiny_protocol(&covers);
    covers.record_ready(1, protocol);
    covers.record_missing(2);
    covers.mark_pending(3);

    assert!(
        covers.protocol_for(1).is_some(),
        "ready cover has a protocol"
    );
    assert!(covers.protocol_for(2).is_none(), "missing cover has none");
    assert!(covers.protocol_for(3).is_none(), "pending cover has none");
    assert!(covers.protocol_for(4).is_none(), "unseen cover has none");
}

#[test]
fn record_missing_caches_the_missing_state() {
    let mut covers = Covers::new();
    covers.record_missing(7);
    assert!(
        matches!(covers.cache.get(&7), Some(CoverState::Missing)),
        "a 404 / decode failure settles as Missing, not absent"
    );
    assert!(
        covers.is_cached(7),
        "a settled Missing counts as cached so it is never re-fetched"
    );
}

#[test]
fn poll_prefetch_fires_once_after_the_debounce_and_then_stops() {
    let mut covers = Covers::new();
    // First sighting arms the debounce, no fetch yet.
    assert_eq!(covers.poll_prefetch(Some(42)), None, "first tick arms only");
    // Three more stable ticks below the threshold.
    for _ in 0..3 {
        assert_eq!(covers.poll_prefetch(Some(42)), None, "below the debounce");
    }
    // The fourth stable tick crosses the threshold and fetches exactly once.
    assert_eq!(
        covers.poll_prefetch(Some(42)),
        Some(42),
        "fires once stable past the debounce"
    );
    // Now marked Pending, so it never fires again for the same row.
    for _ in 0..3 {
        assert_eq!(
            covers.poll_prefetch(Some(42)),
            None,
            "a cached id never re-fetches"
        );
    }
}

#[test]
fn poll_prefetch_resets_when_the_highlight_moves() {
    let mut covers = Covers::new();
    covers.poll_prefetch(Some(1));
    covers.poll_prefetch(Some(1));
    // A new highlight restarts the counter, so the old id's partial stability
    // is discarded and nothing fetches on the switch tick.
    assert_eq!(
        covers.poll_prefetch(Some(2)),
        None,
        "switch resets the debounce"
    );
    // Closing the browse (None) also resets without fetching.
    assert_eq!(covers.poll_prefetch(None), None, "no browse, no fetch");
}
