//! Cover-fetch runtime tests: the CDN URL builder and the fail-soft Missing
//! fold. No network — only the pure URL builder and the in-memory event handler.

use super::*;
use crate::app::App;
use crate::config::Config;

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
