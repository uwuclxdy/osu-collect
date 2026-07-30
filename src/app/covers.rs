//! Session-lived beatmapset cover-image store for the flat set-browse preview.
//!
//! Holds the terminal-graphics [`Picker`] and a per-set cache of decoded cover
//! protocols. The picker defaults to [`Picker::halfblocks`], which queries no
//! terminal, keeping a constructed [`crate::app::App`] deterministic; the
//! runtime swaps in the queried picker once raw mode is live, because querying
//! before it saves and restores COOKED termios and silently kills keyboard
//! input. Neither constructor is side-effect-free under tmux: `ratatui-image`
//! spawns `tmux set -p allow-passthrough on` from both. Progressive
//! enhancement throughout: a set with no cached cover (or one that 404'd or
//! failed to decode) renders text-only, no panic, no toast.
//!
//! The fetch itself is fire-and-forget on the runtime side
//! ([`crate::app::runtime`]); this struct only caches the result and drives the
//! tick-based prefetch debounce so a fast scroll doesn't fire a request per row.
//! That same debounce gates the RENDER ([`Covers::is_settled`]): the iTerm2
//! protocol re-sends the whole base64 image on every render, so a cover drawn
//! for a highlight still on the move costs a full image per keystroke.

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
    /// At least one variant decoded. See [`ReadyCover`].
    Ready(ReadyCover),
    /// Both variants 404'd, failed to fetch, or failed to decode; text-only.
    Missing,
}

/// A settled cover's two render protocols: the square `list@2x` backs the
/// right-hand column, the wide `card@2x` backs the full-width banner. Each is
/// independently optional — one variant can fail while the other lands — and
/// each is boxed so the large protocol doesn't inflate the `Pending`/`Missing`
/// slots (`clippy::large_enum_variant`). A `Ready` always carries at least one:
/// both-`None` settles as [`CoverState::Missing`].
pub struct ReadyCover {
    square: Option<Box<RefCell<StatefulProtocol>>>,
    wide: Option<Box<RefCell<StatefulProtocol>>>,
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

    /// The square (`list@2x`) protocol for `set_id`, or `None` when its cover is
    /// pending, missing, that variant failed, or it was never fetched — the
    /// column render then falls back to text-only.
    pub fn square_for(&self, set_id: u32) -> Option<&RefCell<StatefulProtocol>> {
        match self.cache.get(&set_id) {
            Some(CoverState::Ready(ready)) => ready.square.as_deref(),
            _ => None,
        }
    }

    /// The wide (`card@2x`) protocol for `set_id`, or `None` under the same
    /// conditions as [`Self::square_for`] — the banner render then has nothing to
    /// draw and the preview falls back to the column (or text-only).
    pub fn wide_for(&self, set_id: u32) -> Option<&RefCell<StatefulProtocol>> {
        match self.cache.get(&set_id) {
            Some(CoverState::Ready(ready)) => ready.wide.as_deref(),
            _ => None,
        }
    }

    /// Whether `set_id` has any cache entry (pending, ready, or missing), so the
    /// prefetch never re-requests a cover already known or in flight.
    fn is_cached(&self, set_id: u32) -> bool {
        self.cache.contains_key(&set_id)
    }

    /// Whether `set_id` has held the highlight long enough for its cover to be
    /// worth putting on screen. The same settling signal the fetch debounce
    /// uses, so a cover appears only where a fetch would also have fired.
    ///
    /// Keyed to the id and not to the counter alone: [`Self::stable_ticks`]
    /// advances only on a `Tick`, and a held key starves ticks, so a bare
    /// counter reads stale-high for the row the highlight just moved to.
    pub fn is_settled(&self, set_id: u32) -> bool {
        self.last_seen == Some(set_id) && self.stable_ticks >= COVER_DEBOUNCE_TICKS
    }

    /// Mark a fetch as in flight (claimed by the prefetch before it spawns).
    pub fn mark_pending(&mut self, set_id: u32) {
        self.cache.insert(set_id, CoverState::Pending);
    }

    /// Store a settled cover's variant protocols. Both-`None` would render
    /// identically to `Missing` but must still settle the id so the prefetch
    /// never re-requests it, so it routes to [`Self::record_missing`].
    pub fn record_ready(
        &mut self,
        set_id: u32,
        square: Option<StatefulProtocol>,
        wide: Option<StatefulProtocol>,
    ) {
        if square.is_none() && wide.is_none() {
            self.record_missing(set_id);
            return;
        }
        self.cache.insert(
            set_id,
            CoverState::Ready(ReadyCover {
                square: square.map(|p| Box::new(RefCell::new(p))),
                wide: wide.map(|p| Box::new(RefCell::new(p))),
            }),
        );
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
