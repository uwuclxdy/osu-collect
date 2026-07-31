//! The Get Maps `Search` background task: runs an osu! API v2 `beatmapsets/search`
//! off the UI thread and reports results back over an mpsc channel. Search is
//! CTA-triggered (the `search` button / `load more`), never keystroke-driven, so
//! unlike the collection resolve there is no debounce — a new run cancels any
//! in-flight one immediately.

use crate::app::{App, AppCommand, BrowseRow, FindBackend, FindStatusMsg};
use crate::core::search::{SearchQuery, SearchService, shared_service};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::{sync::mpsc, sync::watch, task::JoinHandle};

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
        result = shared_service().search(&query) => {
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
/// results browse, and descend into it on a fresh non-empty search. Returns a
/// follow-up command (a size probe of the checked osu results) for the runtime
/// loop to dispatch, mirroring the filter handler.
pub fn handle_home_search_event(event: HomeSearchEvent, app: &mut App) -> Option<AppCommand> {
    match event {
        HomeSearchEvent::Loading => {
            app.home.find.status_msg = FindStatusMsg::Loading;
            None
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
            // Feed the session cache: these rows never enrich, but the collection
            // / update browses reuse it to skip refetching a title osu just gave.
            for row in &rows {
                if let Some(meta) = &row.meta {
                    app.home
                        .meta_cache
                        .entry(row.id)
                        .or_insert_with(|| meta.clone());
                }
            }
            app.home.find.next_cursor = cursor;
            app.home.find.status_msg = FindStatusMsg::ReadySearch { total };
            if append {
                app.home.find.browse.append_rows(rows);
            } else {
                app.home.find.browse.set_rows(rows, &app.home.meta_cache);
                // These rows came from osu for the current inputs; record the
                // backend (download subdir prefix) + snapshot the inputs so the
                // `view N maps` button stays accurate after a later edit.
                app.home.find.note_results_backend(FindBackend::Osu);
                app.home.find.mark_results_current();
                // Open the results immediately on a fresh search.
                app.open_find_browse();
            }
            // A search came back, so the token resolved. If that was a guest
            // token (logged out), nudge once toward login for the extra filters.
            app.nudge_guest_search_if_logged_out();
            // Backfill sizes for whatever is checked (carried-over selections on a
            // fresh page, the full page on `load more`); these rows are always osu.
            Some(AppCommand::ProbeFindSizes)
        }
        HomeSearchEvent::Empty => {
            app.home.find.status_msg = FindStatusMsg::Empty;
            app.home.find.next_cursor = None;
            app.home
                .find
                .browse
                .set_rows(Vec::new(), &app.home.meta_cache);
            app.home.find.clear_results_snapshot();
            None
        }
        HomeSearchEvent::Failed { reason } => {
            app.home.find.status_msg = FindStatusMsg::Error(reason);
            app.home.find.clear_results_snapshot();
            None
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/home_search.rs"]
mod tests;
