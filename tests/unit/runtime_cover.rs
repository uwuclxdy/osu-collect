//! Cover-fetch runtime tests: the CDN URL builder, the fail-soft Missing fold,
//! and the loaded-event → lane-bound-protocol wiring. No network — only the
//! pure URL builder and the in-memory event handler.

use super::*;
use crate::app::App;
use crate::app::find_source::BrowseRow;
use crate::app::home::GetMapsSource;
use crate::config::Config;
use image::{DynamicImage, Rgb, RgbImage};
use ratatui::layout::Size;
use ratatui_image::Resize;
use std::collections::HashMap;

#[test]
fn variant_urls_build_the_square_and_wide_asset_paths() {
    // `list@2x` is square (300x300) for the column; `card@2x` is the wide
    // (800x280) banner asset. Both the variant name and the `@2x` suffix are
    // load-bearing — a different variant changes the aspect the layout picks on.
    assert_eq!(
        square_cover_url(1234),
        "https://assets.ppy.sh/beatmaps/1234/covers/list@2x.jpg"
    );
    assert_eq!(
        wide_cover_url(1234),
        "https://assets.ppy.sh/beatmaps/1234/covers/card@2x.jpg"
    );
}

#[test]
fn missing_event_caches_and_blocks_a_refetch() {
    let mut app = App::new(Config::default());
    handle_home_cover_event(HomeCoverEvent::Missing { set_id: 42 }, &mut app);

    assert!(
        app.covers.square_for(42).is_none() && app.covers.wide_for(42).is_none(),
        "a missing cover yields no render protocol for either variant"
    );
    // Settled Missing counts as cached, so the debounced prefetch never re-emits
    // a fetch for that row however long it stays highlighted.
    for _ in 0..8 {
        assert_eq!(
            app.covers.poll_prefetch(Some(42)),
            None,
            "a cached Missing must not schedule another fetch"
        );
    }
}

#[test]
fn loaded_event_builds_lane_bound_protocols_that_restore_through_the_tick_hook() {
    let mut app = App::new(Config::default());
    // The find browse must be the active Get Maps source and hold a highlight
    // for the tick hook to compute one.
    app.home.source = GetMapsSource::Find;
    app.home
        .find
        .browse
        .set_rows(vec![BrowseRow { id: 42, meta: None }], &HashMap::new());
    app.home.find.browse.descend();

    let img = || DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, Rgb([10, 20, 30])));
    handle_home_cover_event(
        HomeCoverEvent::Loaded {
            set_id: 42,
            square: Some(img()),
            wide: None,
        },
        &mut app,
    );
    // The dwell, as a session parked on the row would earn it.
    for _ in 0..8 {
        app.poll_cover_prefetch();
    }
    assert!(app.covers.is_settled(42), "the highlight settled");
    // The render's write-back, as the browse draw would leave it each frame.
    app.covers.square_offer().set(Some(Size::new(20, 10)));

    // The tick hook dispatches the encode to the lane and drains the reply; a
    // bounded poll is the whole contract.
    let mut restored = false;
    for _ in 0..50 {
        app.poll_cover_prefetch();
        if app
            .covers
            .square_for(42)
            .and_then(|tp| tp.borrow().size_for(Resize::Fit(None), Size::new(20, 10)))
            .is_some()
        {
            restored = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        restored,
        "a Loaded event's protocols encode and restore off the UI thread"
    );
}
