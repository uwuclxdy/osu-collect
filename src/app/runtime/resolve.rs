use crate::{
    app::home::ResolveState,
    core::collection::{Collection, CollectionService},
    utils,
};
use osu_downloader::collection::CollectionClient;
use tokio::{sync::mpsc, sync::watch, time};

const DEBOUNCE_MS: u64 = 300;

/// Something a resolve request had to say, stamped with the request that said
/// it.
///
/// The envelope exists so a superseded event is not something the handler can
/// act on: [`handle_home_resolve_event`] compares `generation` before it ever
/// looks at `kind`, so currency is decided once, for the event, rather than per
/// variant. Guarding variant by variant was tried and failed three times running
/// — each fix correct, each leaving the next variant unguarded for someone else
/// to find — because the check was something a new variant had to remember to
/// opt into. Here a new [`HomeResolveKind`] carries the stamp whether or not its
/// author thought about it, and there is no constructor that omits it.
///
/// Keyed on the REQUEST, not on the payload: `Cleared` is sent precisely because
/// the field named no collection, so it has no id to compare and any id-based
/// scheme has to special-case it. The generation is the same device
/// [`EnrichPager`] uses to drop a page returning after a superseding reseed.
///
/// [`EnrichPager`]: crate::app::find_source::EnrichPager
#[derive(Debug)]
pub struct HomeResolveEvent {
    /// The value [`HomeTab::resolve_generation`] held when this request was
    /// scheduled.
    ///
    /// [`HomeTab::resolve_generation`]: crate::app::HomeTab::resolve_generation
    pub generation: u64,
    pub kind: HomeResolveKind,
}

/// What a resolve request had to say. Never handled without its envelope.
#[derive(Debug)]
pub enum HomeResolveKind {
    /// Resolve started; show a loading indicator.
    Loading,
    /// Metadata fetched successfully. The whole payload rides along: the handler
    /// derives the display fields from it and parks it in the session collection
    /// cache, so a download press reuses it instead of refetching it verbatim.
    Resolved {
        collection_id: u32,
        collection: Collection,
    },
    /// Fetch failed; `reason` is a short user-facing message.
    Failed { reason: String },
    /// Field is empty or unparseable; clear any prior display.
    Cleared,
}

impl HomeResolveEvent {
    fn new(generation: u64, kind: HomeResolveKind) -> Self {
        Self { generation, kind }
    }
}

/// Kill the in-flight resolve: abort a running task and signal cancellation to
/// one still inside its debounce.
///
/// Named and callable on its own because it is a separate effect from
/// scheduling, not a step of it. The two rode a single `AppCommand` until a
/// cache hit needed exactly one of them, and suppressing the command to skip the
/// FETCH silently skipped the CANCEL as well — the effect nobody had written
/// down was the one that got dropped.
pub fn cancel_resolve(
    resolve_handle: &mut Option<tokio::task::JoinHandle<()>>,
    resolve_cancel_tx: &mut Option<watch::Sender<bool>>,
) {
    if let Some(handle) = resolve_handle.take() {
        handle.abort();
    }
    if let Some(tx) = resolve_cancel_tx.take() {
        let _ = tx.send(true);
    }
}

/// Start a new debounced resolve under `generation`, which every event this
/// request emits is stamped with.
///
/// Does NOT cancel the previous request: that is [`cancel_resolve`]'s job and
/// the caller sequences the two. Folding the cancel in here is what made
/// `ResolveCollectionUrl` a two-effect command, and a path that wanted only one
/// of them had to drop the other.
///
/// If `value` does not parse as a collection ID, sends `Cleared` immediately
/// and returns without spawning a task.
pub fn schedule_resolve(
    value: &str,
    generation: u64,
    resolve_handle: &mut Option<tokio::task::JoinHandle<()>>,
    resolve_cancel_tx: &mut Option<watch::Sender<bool>>,
    home_resolve_tx: &mpsc::UnboundedSender<HomeResolveEvent>,
) {
    let trimmed = value.trim();
    let Ok(collection_id) = utils::parse_collection_id(trimmed) else {
        // Not a parseable URL/ID — clear display immediately.
        let _ = home_resolve_tx.send(HomeResolveEvent::new(generation, HomeResolveKind::Cleared));
        return;
    };

    let (cancel_tx, cancel_rx) = watch::channel(false);
    *resolve_cancel_tx = Some(cancel_tx);

    let tx = home_resolve_tx.clone();
    let handle = tokio::spawn(async move {
        run_resolve(collection_id, generation, cancel_rx, tx).await;
    });
    *resolve_handle = Some(handle);
}

async fn run_resolve(
    collection_id: u32,
    generation: u64,
    mut cancel_rx: watch::Receiver<bool>,
    tx: mpsc::UnboundedSender<HomeResolveEvent>,
) {
    // Debounce: wait 300 ms, cancel if the field changes again.
    tokio::select! {
        _ = time::sleep(time::Duration::from_millis(DEBOUNCE_MS)) => {}
        _ = cancel_rx.changed() => return,
    }

    let _ = tx.send(HomeResolveEvent::new(generation, HomeResolveKind::Loading));

    let client = CollectionClient::new();
    let service = crate::core::collection::HttpCollectionService::new(client);

    tokio::select! {
        result = service.fetch_collection(collection_id) => {
            let kind = match result {
                Ok(collection) => HomeResolveKind::Resolved {
                    collection_id,
                    collection,
                },
                Err(err) => HomeResolveKind::Failed {
                    reason: user_facing_error(&err.to_string()),
                },
            };
            let _ = tx.send(HomeResolveEvent::new(generation, kind));
        }
        _ = cancel_rx.changed() => {}
    }
}

/// Collapse verbose API error messages to a short user-facing phrase.
fn user_facing_error(err: &str) -> String {
    if err.contains("not found") || err.contains("404") {
        "collection not found".to_string()
    } else if err.contains("rate limited") || err.contains("429") {
        "rate-limited, try again later".to_string()
    } else if err.contains("timed out") || err.contains("timeout") {
        "network timeout".to_string()
    } else {
        "network error".to_string()
    }
}

/// Fold a resolve response into the form, dropping anything a later request has
/// superseded.
///
/// The currency check runs ONCE, here, before `kind` is inspected — so it holds
/// for every variant including ones not written yet, and a superseded event is
/// never something the match arms can act on. Three rounds of per-variant
/// guards each fixed one variant and left the next unguarded; a check the arms
/// cannot see is what ends that.
///
/// `schedule_resolve`'s cancellation is best-effort — a `send` that already ran
/// still drains — so the generation, not the abort, is what makes a superseded
/// response harmless.
///
/// Harmless is not the same as worthless: a superseded response still ran a real
/// fetch, and [`bank_superseded`] keeps what it brought back. What bounds that is
/// the signature, not this paragraph — see there.
pub fn handle_home_resolve_event(event: HomeResolveEvent, home: &mut crate::app::HomeTab) {
    if event.generation != home.resolve_generation() {
        bank_superseded(event.kind, &mut home.collection_cache);
        return;
    }
    match event.kind {
        HomeResolveKind::Loading => {
            home.set_collection_resolve(ResolveState::Loading, "resolving\u{2026}");
        }
        HomeResolveKind::Resolved {
            collection_id,
            collection,
        } => {
            home.adopt_collection(collection_id, &collection);
            home.collection_cache.insert(collection_id, collection);
        }
        HomeResolveKind::Failed { reason } => {
            home.set_collection_resolve(ResolveState::Error, reason);
        }
        HomeResolveKind::Cleared => {
            home.clear_collection_resolve();
        }
    }
}

/// Everything a superseded response is still allowed to do.
///
/// A fetched payload is a fact about a collection id, not about the request that
/// asked for it, so losing the race to a later keystroke is no reason to throw
/// away bytes the network already delivered. Three readers want them: both
/// download arms' `prefetched`, and — since the re-arm landed — the settle, which
/// is what makes returning to that id instant instead of another debounce plus
/// fetch. Discarding here would defeat the re-arm in precisely the window the
/// re-arm exists for.
///
/// **The bound is the signature.** This takes the cache and nothing else, so the
/// form is not in scope and "a superseded response cannot affect what the user
/// sees" is enforced by what the function can reach rather than by whoever edits
/// it next being careful. A fifth [`HomeResolveKind`] can only ever be banked
/// here, never rendered.
///
/// Sibling of the reason [`FindSource::settle_route`] keeps its `size_cache`
/// while dropping the rows: a set's size is a fact about the beatmapset, not
/// about the run that found it.
///
/// [`FindSource::settle_route`]: crate::app::find_source::FindSource::settle_route
fn bank_superseded(kind: HomeResolveKind, cache: &mut crate::app::CollectionCache) {
    if let HomeResolveKind::Resolved {
        collection_id,
        collection,
    } = kind
    {
        cache.insert(collection_id, collection);
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/home_resolve.rs"]
mod tests;
