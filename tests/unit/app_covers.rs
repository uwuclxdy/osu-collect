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
fn variant_accessors_return_a_handle_only_for_ready_covers() {
    let mut covers = Covers::new();
    covers.record_ready(
        1,
        Some(tiny_protocol(&covers)),
        Some(tiny_protocol(&covers)),
    );
    covers.record_missing(2);
    covers.mark_pending(3);

    assert!(covers.square_for(1).is_some(), "ready cover has a square");
    assert!(covers.wide_for(1).is_some(), "ready cover has a wide");
    assert!(
        covers.square_for(2).is_none(),
        "missing cover has no square"
    );
    assert!(covers.wide_for(3).is_none(), "pending cover has no wide");
    assert!(covers.square_for(4).is_none(), "unseen cover has no square");
}

#[test]
fn record_ready_keeps_variants_independent() {
    let mut covers = Covers::new();
    // Only the wide variant decoded; the square 404'd.
    covers.record_ready(1, None, Some(tiny_protocol(&covers)));
    assert!(
        covers.square_for(1).is_none(),
        "a failed square stays absent"
    );
    assert!(
        covers.wide_for(1).is_some(),
        "the wide variant is still served"
    );
}

#[test]
fn record_ready_with_no_variants_settles_missing() {
    let mut covers = Covers::new();
    covers.record_ready(1, None, None);
    assert!(
        matches!(covers.cache.get(&1), Some(CoverState::Missing)),
        "both variants failing settles Missing, not an empty Ready"
    );
    assert!(
        covers.is_cached(1),
        "the empty result still counts as cached so it is never re-fetched"
    );
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
fn is_settled_tracks_the_id_that_earned_the_dwell() {
    let mut covers = Covers::new();
    for _ in 0..COVER_DEBOUNCE_TICKS + 2 {
        covers.poll_prefetch(Some(1));
    }
    assert!(covers.is_settled(1), "the dwelt-on id is settled");
    // A counter-only reading would call 2 settled here: the counter is well past
    // the threshold, it just belongs to id 1.
    assert!(
        !covers.is_settled(2),
        "a neighbour never held the highlight"
    );

    covers.poll_prefetch(Some(2));
    assert!(
        !covers.is_settled(1),
        "the move away drops the old id immediately"
    );
    assert!(!covers.is_settled(2), "the new id starts its own dwell");
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
