//! The Get Maps `Search` background task: runs an osu! API v2 `beatmapsets/search`
//! off the UI thread and reports results back over an mpsc channel. Search is
//! CTA-triggered (the `search` button / `load more`), never keystroke-driven, so
//! unlike the collection resolve there is no debounce — a new run cancels any
//! in-flight one immediately.

use crate::app::{App, BrowseRow, SearchStatusMsg};
use crate::core::search::{HttpSearchService, SearchClient, SearchQuery, SearchService};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::{sync::mpsc, sync::watch, task::JoinHandle};

/// One shared search service for the whole session, so the cached guest
/// `client_credentials` token survives across searches instead of being re-minted
/// each run (the user-token path delegates freshness to `ensure_valid`).
static SEARCH_SERVICE: LazyLock<HttpSearchService> =
    LazyLock::new(|| HttpSearchService::new(SearchClient::new()));

/// Monotonic search generation. Each `schedule_search` bumps it; a task whose
/// generation is stale by the time its request resolves drops the result rather
/// than clobbering the newer search's page (`abort` can't unsend an already
/// resolved-but-unsent result).
static SEARCH_GEN: AtomicU64 = AtomicU64::new(0);

/// Result of a search run, sent back to the main loop.
#[derive(Debug)]
pub enum HomeSearchEvent {
    /// The query is in flight; show a loading indicator.
    Loading,
    /// Results arrived. `append` distinguishes a `load more` page (append + dedup)
    /// from a fresh search (replace + descend into the browse).
    Results {
        entries: Vec<crate::core::search::BeatmapSetMeta>,
        total: u64,
        cursor: Option<String>,
        append: bool,
    },
    /// A fresh search returned nothing.
    Empty,
    /// The query failed; `reason` is a short user-facing message.
    Failed { reason: String },
}

/// Abort any in-flight search and start a new one. `append` is `true` for a
/// `load more` page (the query already carries the paging cursor).
pub fn schedule_search(
    query: SearchQuery,
    append: bool,
    search_handle: &mut Option<JoinHandle<()>>,
    search_cancel_tx: &mut Option<watch::Sender<bool>>,
    home_search_tx: &mpsc::UnboundedSender<HomeSearchEvent>,
) {
    if let Some(handle) = search_handle.take() {
        handle.abort();
    }
    if let Some(tx) = search_cancel_tx.take() {
        let _ = tx.send(true);
    }

    let generation = SEARCH_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let (cancel_tx, cancel_rx) = watch::channel(false);
    *search_cancel_tx = Some(cancel_tx);

    let tx = home_search_tx.clone();
    let handle = tokio::spawn(async move {
        run_search_task(query, append, generation, cancel_rx, tx).await;
    });
    *search_handle = Some(handle);
}

async fn run_search_task(
    query: SearchQuery,
    append: bool,
    generation: u64,
    mut cancel_rx: watch::Receiver<bool>,
    tx: mpsc::UnboundedSender<HomeSearchEvent>,
) {
    let _ = tx.send(HomeSearchEvent::Loading);

    tokio::select! {
        result = SEARCH_SERVICE.search(&query) => {
            // A newer search was dispatched while this one was in flight: drop the
            // stale result so it can't overwrite the current page/status.
            if SEARCH_GEN.load(Ordering::Relaxed) != generation {
                return;
            }
            let event = match result {
                Ok(results) => {
                    if results.beatmapsets.is_empty() && !append {
                        HomeSearchEvent::Empty
                    } else {
                        HomeSearchEvent::Results {
                            entries: results.beatmapsets,
                            total: results.total,
                            cursor: results.cursor_string,
                            append,
                        }
                    }
                }
                // `AppError`'s Display is already user-facing (`map_search_error`
                // maps 401 → "search requires login", 429 → "rate limited", …).
                Err(err) => HomeSearchEvent::Failed { reason: err.to_string() },
            };
            let _ = tx.send(event);
        }
        _ = cancel_rx.changed() => {}
    }
}

/// Fold a search result into the app: update the status line, (re)populate the
/// results browse, and descend into it on a fresh non-empty search.
pub fn handle_home_search_event(event: HomeSearchEvent, app: &mut App) {
    match event {
        HomeSearchEvent::Loading => {
            app.home.search.status_msg = SearchStatusMsg::Loading;
        }
        HomeSearchEvent::Results {
            entries,
            total,
            cursor,
            append,
        } => {
            let rows: Vec<BrowseRow> = entries
                .into_iter()
                .map(|meta| BrowseRow {
                    id: meta.id,
                    meta: Some(meta),
                })
                .collect();
            // Scope the search borrow so the guest nudge below can take `&mut app`.
            {
                let search = &mut app.home.search;
                search.next_cursor = cursor;
                search.status_msg = SearchStatusMsg::Ready { total };
                if append {
                    search.browse.append_rows(rows);
                } else {
                    search.browse.set_rows(rows);
                    // Open the results immediately on a fresh search.
                    search.browse.descend();
                }
            }
            // A search came back, so the token resolved. If that was a guest
            // token (logged out), nudge once toward login for the extra filters.
            app.nudge_guest_search_if_logged_out();
        }
        HomeSearchEvent::Empty => {
            let search = &mut app.home.search;
            search.status_msg = SearchStatusMsg::Empty;
            search.next_cursor = None;
            search.browse.set_rows(Vec::new());
        }
        HomeSearchEvent::Failed { reason } => {
            app.home.search.status_msg = SearchStatusMsg::Error(reason);
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/home_search.rs"]
mod tests;
