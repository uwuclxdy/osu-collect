use crate::{
    app::home::ResolveState,
    core::collection::{Collection, CollectionService},
    utils,
};
use osu_downloader::collection::CollectionClient;
use std::collections::HashSet;
use tokio::{sync::mpsc, sync::watch, time};

const DEBOUNCE_MS: u64 = 300;

/// Result of a URL-field resolve attempt, sent back to the main loop.
#[derive(Debug)]
pub enum HomeResolveEvent {
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

/// Abort any previous resolve task and start a new debounced one.
///
/// If `value` does not parse as a collection ID, sends `Cleared` immediately
/// and returns without spawning a task.
pub fn schedule_resolve(
    value: &str,
    resolve_handle: &mut Option<tokio::task::JoinHandle<()>>,
    resolve_cancel_tx: &mut Option<watch::Sender<bool>>,
    home_resolve_tx: &mpsc::UnboundedSender<HomeResolveEvent>,
) {
    // Abort any in-flight task.
    if let Some(handle) = resolve_handle.take() {
        handle.abort();
    }
    // Signal cancellation to any task that is still starting up.
    if let Some(tx) = resolve_cancel_tx.take() {
        let _ = tx.send(true);
    }

    let trimmed = value.trim();
    let Ok(collection_id) = utils::parse_collection_id(trimmed) else {
        // Not a parseable URL/ID — clear display immediately.
        let _ = home_resolve_tx.send(HomeResolveEvent::Cleared);
        return;
    };

    let (cancel_tx, cancel_rx) = watch::channel(false);
    *resolve_cancel_tx = Some(cancel_tx);

    let tx = home_resolve_tx.clone();
    let handle = tokio::spawn(async move {
        run_resolve(collection_id, cancel_rx, tx).await;
    });
    *resolve_handle = Some(handle);
}

async fn run_resolve(
    collection_id: u32,
    mut cancel_rx: watch::Receiver<bool>,
    tx: mpsc::UnboundedSender<HomeResolveEvent>,
) {
    // Debounce: wait 300 ms, cancel if the field changes again.
    tokio::select! {
        _ = time::sleep(time::Duration::from_millis(DEBOUNCE_MS)) => {}
        _ = cancel_rx.changed() => return,
    }

    let _ = tx.send(HomeResolveEvent::Loading);

    let client = CollectionClient::new();
    let service = crate::core::collection::HttpCollectionService::new(client);

    tokio::select! {
        result = service.fetch_collection(collection_id) => {
            let event = match result {
                Ok(collection) => HomeResolveEvent::Resolved {
                    collection_id,
                    collection,
                },
                Err(err) => HomeResolveEvent::Failed {
                    reason: user_facing_error(&err.to_string()),
                },
            };
            let _ = tx.send(event);
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

pub fn handle_home_resolve_event(event: HomeResolveEvent, home: &mut crate::app::HomeTab) {
    match event {
        HomeResolveEvent::Loading => {
            home.set_collection_resolve(ResolveState::Loading, "resolving…");
        }
        HomeResolveEvent::Resolved {
            collection_id,
            collection,
        } => {
            let map_count = collection.beatmapsets.len();
            let maps_word = if map_count == 1 { "mapset" } else { "mapsets" };
            home.set_collection_resolve(
                ResolveState::Success,
                format!("\"{}\" · {} {}", collection.name, map_count, maps_word),
            );

            let beatmapset_ids: Vec<u32> =
                collection.beatmapsets.iter().map(|set| set.id).collect();
            // One diff id per unique set feeds the batch-enrichment pager (the
            // endpoint takes diff ids; the set metadata rides nested in each row).
            // Sets with no diffs stay id-only in the preview.
            let mut seen = HashSet::with_capacity(beatmapset_ids.len());
            let enrich_pairs: Vec<(u32, u32)> = collection
                .beatmapsets
                .iter()
                .filter(|set| seen.insert(set.id))
                .filter_map(|set| set.beatmaps.first().map(|diff| (set.id, diff.id)))
                .collect();
            // Derived from the full collection (the lib stays the single source of
            // folder naming) since the app side keeps only name + id.
            let folder_name = collection.folder_name();

            home.set_resolved_collection(collection_id, beatmapset_ids);
            home.resolved_enrich_pairs = enrich_pairs;
            home.resolved_folder_name = Some(folder_name);
            home.collection_cache.insert(collection_id, collection);
        }
        HomeResolveEvent::Failed { reason } => {
            home.set_collection_resolve(ResolveState::Error, reason);
        }
        HomeResolveEvent::Cleared => {
            home.clear_collection_resolve();
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/home_resolve.rs"]
mod tests;
