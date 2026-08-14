//! Cover-store unit tests: the ready/missing/pending lookup, the tick-based
//! prefetch debounce, and the lane-driven resize+encode round trip. All
//! headless — the halfblocks picker needs no terminal and the protocols are
//! built from tiny in-memory images, no network.

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

/// The 2x2 image a [`tiny_protocol`] carries, fitted into a 20x10 offer by the
/// halfblocks 10x20 font: one cell each way. The dispatch round-trip tests all
/// settle at this size.
const TINY_FITTED: Size = Size::new(1, 1);
/// The offer the layout makes a tiny image: the pane's allowance, not the
/// fitted size — the re-encode gate keys off the pane.
const TINY_OFFER: Size = Size::new(20, 10);

#[test]
fn ready_cover_protocols_answer_size_before_any_encode() {
    let mut covers = Covers::new();
    covers.record_ready(1, Some(tiny_protocol(&covers)), None);
    // The layout needs the fitted size the instant the cover is recorded; the
    // first encode rides a lane, so the seat must not wait for it.
    assert_eq!(
        covers
            .square_for(1)
            .and_then(|tp| tp.borrow().size_for(Resize::Fit(None), TINY_OFFER)),
        Some(TINY_FITTED),
        "a ready cover answers its fitted size immediately, pre-encode"
    );
}

#[test]
fn a_settled_offer_dispatches_one_encode_and_the_drain_restores_the_protocol() {
    let mut covers = Covers::new();
    covers.record_ready(1, Some(tiny_protocol(&covers)), None);
    for _ in 0..COVER_DEBOUNCE_TICKS + 2 {
        covers.poll_prefetch(Some(1));
    }
    assert!(covers.is_settled(1), "the dwell is earned");
    covers.square_offer().set(Some(TINY_OFFER));

    // The worker encodes in its own time; a bounded poll on the tick pass is
    // the whole contract — dispatch + drain until the protocol answers again.
    let mut restored = false;
    for _ in 0..50 {
        covers.poll_cover_encodes(Some(1));
        if covers
            .square_for(1)
            .and_then(|tp| tp.borrow().size_for(Resize::Fit(None), TINY_OFFER))
            .is_some()
        {
            restored = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(restored, "the drain restores the encoded protocol");

    // The same offer is now settled, so the gate stays closed: the tick pass
    // keeps running, no second encode is sent.
    for _ in 0..3 {
        covers.poll_cover_encodes(Some(1));
        assert!(
            covers
                .square_for(1)
                .and_then(|tp| tp.borrow().size_for(Resize::Fit(None), TINY_OFFER))
                .is_some(),
            "a settled offer is never re-sent"
        );
    }
}

#[test]
fn the_dispatch_gate_refuses_a_second_request_while_one_is_in_flight() {
    let mut covers = Covers::new();
    covers.record_ready(1, Some(tiny_protocol(&covers)), None);
    for _ in 0..COVER_DEBOUNCE_TICKS + 2 {
        covers.poll_prefetch(Some(1));
    }
    covers.square_offer().set(Some(TINY_OFFER));
    // The gate is `needs_resize` — the same check the tick pass dispatches on,
    // read through the public protocol so the in-flight window is observable.
    let gate = |covers: &Covers| {
        covers.square_for(1).is_some_and(|tp| {
            tp.borrow()
                .needs_resize(&Resize::Fit(None), TINY_OFFER)
                .is_some()
        })
    };
    assert!(gate(&covers), "a fresh protocol needs its first encode");

    covers.dispatch_settled_encodes(Some(1));
    assert!(
        !gate(&covers),
        "the request took the protocol: no re-send while one is in flight"
    );

    // Drain without dispatching: the worker's reply restores the protocol.
    let mut restored = false;
    for _ in 0..50 {
        covers.drain_lanes();
        if covers
            .square_for(1)
            .and_then(|tp| tp.borrow().size_for(Resize::Fit(None), TINY_OFFER))
            .is_some()
        {
            restored = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(restored, "the reply restores the protocol");
    assert!(!gate(&covers), "and the settled offer is not re-encoded");
}

#[test]
fn the_tick_pass_services_only_the_settled_highlight() {
    let mut covers = Covers::new();
    covers.record_ready(1, Some(tiny_protocol(&covers)), None);
    covers.record_ready(2, Some(tiny_protocol(&covers)), None);
    covers.square_offer().set(Some(TINY_OFFER));

    // Row 2 is highlighted but never held the dwell: nothing dispatches.
    covers.dispatch_settled_encodes(Some(2));
    assert!(
        covers.square_fitted_for(2).is_none(),
        "an unsettled highlight is not serviced"
    );
    assert!(
        covers.square_fitted_for(1).is_none(),
        "and no other row is touched"
    );

    // Settle 1, then move the highlight to 2: the settle belongs to a row that
    // is no longer highlighted, and the move resets the counter (the
    // scroll-fires-nothing leg of the dwell gate).
    for _ in 0..COVER_DEBOUNCE_TICKS + 2 {
        covers.poll_prefetch(Some(1));
    }
    covers.poll_prefetch(Some(2));
    covers.dispatch_settled_encodes(Some(2));
    assert!(
        covers.square_fitted_for(2).is_none(),
        "the dwell reset when the highlight moved"
    );
    assert!(
        covers.square_fitted_for(1).is_none(),
        "the moved-away row is not serviced"
    );

    // Row 2 earns its own dwell: now it is the only row serviced.
    for _ in 0..COVER_DEBOUNCE_TICKS + 2 {
        covers.poll_prefetch(Some(2));
    }
    covers.dispatch_settled_encodes(Some(2));
    assert!(
        covers.square_fitted_for(2).is_some(),
        "the settled highlight is serviced"
    );
    assert!(
        covers.square_fitted_for(1).is_none(),
        "and only the settled highlight"
    );
}

#[test]
fn the_drain_runs_without_a_highlight_so_left_behind_requests_still_restore() {
    let mut covers = Covers::new();
    covers.record_ready(1, Some(tiny_protocol(&covers)), None);
    for _ in 0..COVER_DEBOUNCE_TICKS + 2 {
        covers.poll_prefetch(Some(1));
    }
    covers.square_offer().set(Some(TINY_OFFER));
    covers.dispatch_settled_encodes(Some(1));
    assert!(
        covers
            .square_for(1)
            .and_then(|tp| tp.borrow().size_for(Resize::Fit(None), TINY_OFFER))
            .is_none(),
        "the request is with the worker"
    );

    // The highlight leaves (browse closed): the dispatch gate closes, but the
    // drain must still run so the left-behind request restores its protocol —
    // a revisit finds the cover encodable again, not stuck in flight.
    let mut restored = false;
    for _ in 0..50 {
        covers.poll_cover_encodes(None);
        if covers
            .square_for(1)
            .and_then(|tp| tp.borrow().size_for(Resize::Fit(None), TINY_OFFER))
            .is_some()
        {
            restored = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(restored, "the drain restores a set the highlight has left");
}

#[test]
fn a_failed_encode_settles_that_variant_text_only() {
    let mut covers = Covers::new();
    covers.record_ready(
        1,
        Some(tiny_protocol(&covers)),
        Some(tiny_protocol(&covers)),
    );
    apply_lane_reply(
        &mut covers.cache,
        1,
        LaneReply::Failed,
        CoverVariant::Square,
    );
    assert!(
        covers.square_for(1).is_none(),
        "the failed variant is dropped"
    );
    assert!(
        covers.square_fitted_for(1).is_none(),
        "and its fitted size with it"
    );
    assert!(
        covers.wide_for(1).is_some(),
        "the other variant is untouched"
    );
    assert!(covers.wide_fitted_for(1).is_none(), "and stays unfitted");

    // A reply for an unknown set is dropped without a panic.
    apply_lane_reply(&mut covers.cache, 99, LaneReply::Failed, CoverVariant::Wide);
}
