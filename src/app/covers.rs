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
//!
//! Resize+encode lives on two std-thread lanes (one per cover variant), never
//! on the UI thread: the tick pass ([`Covers::poll_cover_encodes`]) sends a
//! request when a settled variant's pane offer outgrows its encoding, and the
//! render only ever draws the cached result. The request channel is the
//! [`ThreadProtocol`]'s own and carries no caller key, so the lane records
//! dispatch order and the worker's one-reply-per-request FIFO routes each
//! reply back to the set that issued it — the crate's private protocol id is
//! a stale-guard on top of that route, not the route itself. A variant in
//! flight renders nothing (its protocol is with the worker); the layout holds
//! its seat off the last-fitted size the dispatch recorded, so a flight never
//! reflows the preview text.

use ratatui::layout::Size;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::thread::{ResizeRequest, ResizeResponse, ThreadProtocol};
use ratatui_image::{Resize, ResizeEncodeRender};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use tracing::{debug, error};

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
    square: Option<Box<RefCell<ThreadProtocol>>>,
    wide: Option<Box<RefCell<ThreadProtocol>>>,
    /// The pane offer each variant's current encoding answers, recorded at
    /// dispatch so the layout holds its seat while the encode is in flight
    /// (`size_for` answers nothing then). `None` until the first dispatch.
    square_fitted: Option<Size>,
    wide_fitted: Option<Size>,
}

/// One variant's off-UI-thread resize+encode lane: a worker thread blocking on
/// `rx`, replying over its own channel. The request channel doubles as every
/// [`ThreadProtocol`] of that variant's lane, so each request carries the
/// protocol it encodes. The crate's request/response carry no caller-supplied
/// key, so the lane records the dispatch order (`in_flight`) and the worker's
/// FIFO replies — one per request — let the drain route each reply back to
/// the set that issued it.
struct CoverLane {
    tx: Sender<ResizeRequest>,
    rx: Receiver<LaneReply>,
    /// The sets with requests in flight on this lane, oldest first. The worker
    /// replies strictly in request order, so the front of this queue is the
    /// set the next reply belongs to. Only this mapping lets a shared lane
    /// route a reply to the right protocol: the crate's private protocol id is
    /// a stale-guard on top of it, not the route.
    in_flight: VecDeque<u32>,
}

/// A lane worker's verdict for one request. `Resized` carries the encoded
/// protocol back to its set's slot; `Failed` (an encode error, or a request
/// the worker refused) settles that variant text-only — the request consumed
/// the protocol, so there is nothing to restore. Boxed so the enum stays the
/// size of the `Failed` marker (`clippy::large_enum_variant`).
enum LaneReply {
    Resized(Box<ResizeResponse>),
    Failed,
}

/// Which variant a reply belongs to: the drain knows it from the lane it
/// drained and routes the reply to that variant's slot.
#[derive(Clone, Copy)]
enum CoverVariant {
    Square,
    Wide,
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
    /// The two resize+encode lanes, one per cover variant.
    square_lane: CoverLane,
    wide_lane: CoverLane,
    /// The pane's offer for each variant, written back by the render every
    /// frame: the seated variant's offered size, `None` for the other (and
    /// both `None` when nothing seats). Cells, so the draw path — which
    /// borrows the app immutably — can write them; the tick pass reads them
    /// for the settled highlight.
    square_offer: Cell<Option<Size>>,
    wide_offer: Cell<Option<Size>>,
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
            square_lane: spawn_cover_lane("cover-square"),
            wide_lane: spawn_cover_lane("cover-wide"),
            square_offer: Cell::new(None),
            wide_offer: Cell::new(None),
        }
    }

    /// The square (`list@2x`) protocol for `set_id`, or `None` when its cover is
    /// pending, missing, that variant failed, or it was never fetched — the
    /// column render then falls back to text-only.
    pub fn square_for(&self, set_id: u32) -> Option<&RefCell<ThreadProtocol>> {
        match self.cache.get(&set_id) {
            Some(CoverState::Ready(ready)) => ready.square.as_deref(),
            _ => None,
        }
    }

    /// The wide (`card@2x`) protocol for `set_id`, or `None` under the same
    /// conditions as [`Self::square_for`] — the banner render then has nothing to
    /// draw and the preview falls back to the column (or text-only).
    pub fn wide_for(&self, set_id: u32) -> Option<&RefCell<ThreadProtocol>> {
        match self.cache.get(&set_id) {
            Some(CoverState::Ready(ready)) => ready.wide.as_deref(),
            _ => None,
        }
    }

    /// The last-fitted size the square variant's current encoding answers, for
    /// the layout to hold its seat while an encode is in flight.
    pub fn square_fitted_for(&self, set_id: u32) -> Option<Size> {
        match self.cache.get(&set_id) {
            Some(CoverState::Ready(ready)) => ready.square_fitted,
            _ => None,
        }
    }

    /// The last-fitted size the wide variant's current encoding answers, for
    /// the layout to hold its seat while an encode is in flight.
    pub fn wide_fitted_for(&self, set_id: u32) -> Option<Size> {
        match self.cache.get(&set_id) {
            Some(CoverState::Ready(ready)) => ready.wide_fitted,
            _ => None,
        }
    }

    /// The render's write-back cell for the square column's pane offer: the
    /// size the layout seated the variant at this frame, or `None`. See the
    /// struct docs on [`Self`] for the write-back contract.
    pub fn square_offer(&self) -> &Cell<Option<Size>> {
        &self.square_offer
    }

    /// The render's write-back cell for the wide column's pane offer.
    pub fn wide_offer(&self) -> &Cell<Option<Size>> {
        &self.wide_offer
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

    /// Store a settled cover's variant protocols, each bound to its variant's
    /// lane so the tick pass can encode off the UI thread. Both-`None` would
    /// render identically to `Missing` but must still settle the id so the
    /// prefetch never re-requests it, so it routes to [`Self::record_missing`].
    ///
    /// Runs at most once per set (the prefetch gates on [`Self::is_cached`]),
    /// which is what [`apply_lane_reply`] leans on: the `Ready` slot a lane
    /// reply lands into is the same protocol that dispatched. A future
    /// re-fetch/replace path must preserve that — replacing a `Ready`
    /// mid-flight lets a stale reply drop on the protocol-id mismatch, or a
    /// `Failed` reply null out the replacement's slot.
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
                square: square.map(|p| {
                    Box::new(RefCell::new(ThreadProtocol::new(
                        self.square_lane.tx.clone(),
                        Some(p),
                    )))
                }),
                wide: wide.map(|p| {
                    Box::new(RefCell::new(ThreadProtocol::new(
                        self.wide_lane.tx.clone(),
                        Some(p),
                    )))
                }),
                square_fitted: None,
                wide_fitted: None,
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

    /// Advance the off-thread resize+encode pass for one tick: dispatch the
    /// settled highlight's seated variants, then drain both lanes' replies.
    /// `highlighted` is the same value the prefetch debounce runs on, so the
    /// dispatch is gated by the same dwell (scrolling fires no encodes). The
    /// drain runs unconditionally so a request the highlight left behind still
    /// restores its protocol.
    pub fn poll_cover_encodes(&mut self, highlighted: Option<u32>) {
        self.dispatch_settled_encodes(highlighted);
        self.drain_lanes();
    }

    /// Send one resize request per seated, settled variant whose encoding no
    /// longer matches the pane's current offer. The request carries the
    /// protocol itself ([`ResizeEncodeRender::resize_encode`] takes it out of
    /// the [`ThreadProtocol`]), so a variant has at most one request in flight
    /// by construction — [`ResizeEncodeRender::needs_resize`] answers `None`
    /// both while one is in flight and once the current encoding matches the
    /// offer. The rect encoded at is the fitted rect `needs_resize` returns
    /// (== the protocol's own `size_for` of the offer), not the offer itself:
    /// encoding at the offer would never settle.
    fn dispatch_settled_encodes(&mut self, highlighted: Option<u32>) {
        let Some(set_id) = highlighted.filter(|&id| self.is_settled(id)) else {
            return;
        };
        let Some(CoverState::Ready(ready)) = self.cache.get_mut(&set_id) else {
            return;
        };
        let square_offer = self.square_offer.get();
        if let Some(tp) = ready.square.as_deref_mut()
            && let Some(offer) = square_offer
        {
            let mut tp = tp.borrow_mut();
            if let Some(rect) = tp.needs_resize(&Resize::Fit(None), offer) {
                // Unreachable: fit_area_proportionally floors each fitted
                // dimension at 1 for a nonzero image in a nonzero offer.
                // Skip rather than send a request the crate would panic on —
                // and skip only this variant: the wide dispatch below must
                // still run, or a zero-size offer would starve the wide lane.
                if rect.width > 0 && rect.height > 0 {
                    ready.square_fitted = Some(rect);
                    tp.resize_encode(&Resize::Fit(None), rect);
                    self.square_lane.in_flight.push_back(set_id);
                } else {
                    debug!(set_id, "cover resize offer refused: zero-size rect");
                }
            }
        }
        let wide_offer = self.wide_offer.get();
        if let Some(tp) = ready.wide.as_deref_mut()
            && let Some(offer) = wide_offer
        {
            let mut tp = tp.borrow_mut();
            if let Some(rect) = tp.needs_resize(&Resize::Fit(None), offer) {
                // Unreachable: fit_area_proportionally floors each fitted
                // dimension at 1 for a nonzero image in a nonzero offer.
                // Skip rather than send a request the crate would panic on.
                if rect.width > 0 && rect.height > 0 {
                    ready.wide_fitted = Some(rect);
                    tp.resize_encode(&Resize::Fit(None), rect);
                    self.wide_lane.in_flight.push_back(set_id);
                } else {
                    debug!(set_id, "cover resize offer refused: zero-size rect");
                }
            }
        }
    }

    /// Apply every lane reply that has landed since the last tick, routing each
    /// to the set + variant it names. The reply stream is FIFO with the
    /// dispatch stream (one worker, one reply per request), so the set a reply
    /// belongs to is the oldest dispatch still in flight. `try_recv` keeps the
    /// tick non-blocking: the UI thread never waits on a worker.
    fn drain_lanes(&mut self) {
        while let Ok(reply) = self.square_lane.rx.try_recv() {
            let Some(set_id) = self.square_lane.in_flight.pop_front() else {
                error!("cover lane square: reply without a matching dispatch");
                break;
            };
            apply_lane_reply(&mut self.cache, set_id, reply, CoverVariant::Square);
        }
        while let Ok(reply) = self.wide_lane.rx.try_recv() {
            let Some(set_id) = self.wide_lane.in_flight.pop_front() else {
                error!("cover lane wide: reply without a matching dispatch");
                break;
            };
            apply_lane_reply(&mut self.cache, set_id, reply, CoverVariant::Wide);
        }
    }
}

/// Spawn a lane worker: blocking recv → resize+encode → reply, forever. The
/// worker never touches the terminal (encoding is pure computation on the
/// protocol the UI thread built and rendered) and never panics on a closed
/// channel — recv/send errors end the loop and the other end simply stops
/// getting replies. The encode is wrapped in `catch_unwind` because
/// `ResizeRequest`'s fields are private, so a zero-size request can't be
/// refused by inspection here (the dispatch site guards that, where the rect
/// is visible); a panic converts to `Failed` and the lane lives on.
fn spawn_cover_lane(name: &'static str) -> CoverLane {
    let (tx, request_rx) = channel::<ResizeRequest>();
    let (reply_tx, rx) = channel::<LaneReply>();
    let worker = move || loop {
        let Ok(request) = request_rx.recv() else {
            break;
        };
        let reply = match catch_unwind(AssertUnwindSafe(|| request.resize_encode())) {
            Ok(Ok(response)) => LaneReply::Resized(Box::new(response)),
            Ok(Err(error)) => {
                debug!(%error, "cover encode failed");
                LaneReply::Failed
            }
            Err(_) => {
                debug!("cover encode panicked");
                LaneReply::Failed
            }
        };
        if reply_tx.send(reply).is_err() {
            break;
        }
    };
    match thread::Builder::new().name(name.to_string()).spawn(worker) {
        Ok(_) => {}
        // Fail-soft: with no worker the requests never reply and every cover
        // stays text-only — the same absence a missing cover renders as.
        Err(error) => error!(%error, "cover lane {name} failed to spawn"),
    }
    CoverLane {
        tx,
        rx,
        in_flight: VecDeque::new(),
    }
}

/// Route one lane reply into the cache. A `Resized` restores the encoded
/// protocol into its set's variant slot; `update_resized_protocol` accepts it
/// only while the protocol's private id still matches the issuing request, so
/// a reply for a protocol that was replaced is dropped (logged at `debug`).
/// A `Failed` settles that variant text-only: the request consumed the
/// protocol, so there is nothing to restore, and dropping the slot keeps the
/// layout from re-encoding a variant whose encode cannot succeed.
fn apply_lane_reply(
    cache: &mut HashMap<u32, CoverState>,
    set_id: u32,
    reply: LaneReply,
    variant: CoverVariant,
) {
    let Some(CoverState::Ready(ready)) = cache.get_mut(&set_id) else {
        return;
    };
    let (slot, fitted) = match variant {
        CoverVariant::Square => (&mut ready.square, &mut ready.square_fitted),
        CoverVariant::Wide => (&mut ready.wide, &mut ready.wide_fitted),
    };
    match reply {
        LaneReply::Resized(response) => {
            if let Some(tp) = slot.as_deref_mut() {
                let restored = tp.borrow_mut().update_resized_protocol(*response);
                if !restored {
                    debug!(set_id, "stale cover resize reply dropped");
                }
            }
        }
        LaneReply::Failed => {
            *slot = None;
            *fitted = None;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app_covers.rs"]
mod tests;
