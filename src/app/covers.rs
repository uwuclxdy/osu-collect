//! Session-lived beatmapset cover-image store for the flat set-browse preview.
//!
//! Holds the terminal-graphics [`Picker`] and a per-set cache of decoded cover
//! protocols. The picker is queried once at runtime startup (before raw mode)
//! and defaults to [`Picker::halfblocks`] — deterministic and test-safe, so a
//! constructed [`crate::app::App`] never touches the terminal. Progressive
//! enhancement throughout: a set with no cached cover (or one that 404'd or
//! failed to decode) renders text-only, no panic, no toast.
//!
//! The fetch itself is fire-and-forget on the runtime side
//! ([`crate::app::runtime`]); this struct only caches the result and drives the
//! tick-based prefetch debounce so a fast scroll doesn't fire a request per row.

use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::cell::RefCell;
use std::collections::HashMap;

/// How many consecutive ticks a set id must stay the highlight before its cover
/// is fetched. At the 50ms tick rate (`tui::terminal`) this is ~200ms, long
/// enough that key-repeat scrolling past rows never fires a request per row.
const COVER_DEBOUNCE_TICKS: u32 = 4;

/// The load state of one beatmapset's cover. `Ready` wraps the resize protocol
/// in a `RefCell` because rendering it (`resize_encode_render`) mutates the
/// cached encoding, yet the draw path borrows the app immutably.
pub enum CoverState {
    /// A fetch is in flight; do not re-request.
    Pending,
    /// The cover decoded; the protocol renders it letterboxed into any area.
    /// Boxed so the large protocol doesn't inflate every `Pending`/`Missing`
    /// cache slot (`clippy::large_enum_variant`).
    Ready(Box<RefCell<StatefulProtocol>>),
    /// The cover 404'd, failed to fetch, or failed to decode; render text-only.
    Missing,
}

/// Cover-image store held on [`crate::app::App`]. See the module docs.
pub struct Covers {
    /// Terminal-graphics picker. Halfblocks by default (no terminal query, so
    /// `App::new` is test-safe); the runtime overrides it with a queried picker
    /// before entering raw mode.
    pub picker: Picker,
    cache: HashMap<u32, CoverState>,
    /// Debounce bookkeeping: the last highlighted set id and how many
    /// consecutive ticks it has held the highlight.
    last_seen: Option<u32>,
    stable_ticks: u32,
}

impl Default for Covers {
    fn default() -> Self {
        Self::new()
    }
}

impl Covers {
    pub fn new() -> Self {
        Self {
            picker: Picker::halfblocks(),
            cache: HashMap::new(),
            last_seen: None,
            stable_ticks: 0,
        }
    }

    /// The ready protocol for `set_id`, or `None` when its cover is pending,
    /// missing, or was never fetched — the render then falls back to text-only.
    pub fn protocol_for(&self, set_id: u32) -> Option<&RefCell<StatefulProtocol>> {
        match self.cache.get(&set_id) {
            Some(CoverState::Ready(protocol)) => Some(&**protocol),
            _ => None,
        }
    }

    /// Whether `set_id` has any cache entry (pending, ready, or missing), so the
    /// prefetch never re-requests a cover already known or in flight.
    fn is_cached(&self, set_id: u32) -> bool {
        self.cache.contains_key(&set_id)
    }

    /// Mark a fetch as in flight (claimed by the prefetch before it spawns).
    pub fn mark_pending(&mut self, set_id: u32) {
        self.cache.insert(set_id, CoverState::Pending);
    }

    /// Store a decoded cover's render protocol.
    pub fn record_ready(&mut self, set_id: u32, protocol: StatefulProtocol) {
        self.cache
            .insert(set_id, CoverState::Ready(Box::new(RefCell::new(protocol))));
    }

    /// Record that a cover has no image (404 / fetch / decode failure); settles
    /// the id so it is never re-fetched this session.
    pub fn record_missing(&mut self, set_id: u32) {
        self.cache.insert(set_id, CoverState::Missing);
    }

    /// Advance the debounce for one tick given the currently-highlighted set id
    /// (`None` = no flat browse open / no highlighted row). Returns the id to
    /// fetch once it has held the highlight for [`COVER_DEBOUNCE_TICKS`] and is
    /// not already cached, marking it `Pending` so the same tick's fetch is
    /// claimed exactly once. A change in highlight resets the counter.
    pub fn poll_prefetch(&mut self, highlighted: Option<u32>) -> Option<u32> {
        if highlighted != self.last_seen {
            self.last_seen = highlighted;
            self.stable_ticks = 0;
            return None;
        }
        let set_id = highlighted?;
        self.stable_ticks = self.stable_ticks.saturating_add(1);
        if self.stable_ticks >= COVER_DEBOUNCE_TICKS && !self.is_cached(set_id) {
            self.mark_pending(set_id);
            return Some(set_id);
        }
        None
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app_covers.rs"]
mod tests;
