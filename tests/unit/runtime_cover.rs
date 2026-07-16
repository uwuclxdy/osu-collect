//! Cover-fetch runtime tests: the CDN URL builder and the fail-soft Missing
//! fold. No network — only the pure URL builder and the in-memory event handler.

use super::*;
use crate::app::App;
use crate::config::Config;

#[test]
fn cover_url_builds_the_assets_ppy_card_path() {
    assert_eq!(
        cover_url(1234),
        "https://assets.ppy.sh/beatmaps/1234/covers/card.jpg"
    );
}

#[test]
fn missing_event_caches_and_blocks_a_refetch() {
    let mut app = App::new(Config::default());
    handle_home_cover_event(HomeCoverEvent::Missing { set_id: 42 }, &mut app);

    assert!(
        app.covers.protocol_for(42).is_none(),
        "a missing cover yields no render protocol"
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
