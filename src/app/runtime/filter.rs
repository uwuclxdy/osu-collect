//! The Get Maps `Filter` background task: the nzbasic fetch, off the UI thread,
//! reporting back over an mpsc channel. Like search, the fetch is CTA-triggered
//! — a new run cancels any in-flight one. Result rows land id-only; their
//! set-level metadata is backfilled by the shared osu-batch enrichment pager
//! (`src/app/runtime/enrich.rs`), not a nzbasic `beatmapDetails` call.

use crate::app::{App, AppCommand, BrowseRow, EnrichTarget, FindBackend, FindStatusMsg};
use osu_downloader::Error;
use osu_downloader::filter::{FilterClient, FilterQuery, FilterResults};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::{sync::mpsc, sync::watch, task::JoinHandle};

/// One shared client for the whole session (connection reuse).
static FILTER_CLIENT: LazyLock<FilterClient> = LazyLock::new(FilterClient::new);

/// Monotonic filter generation. Each `schedule_filter` bumps it; a fetch whose
/// generation is stale by completion drops its result rather than clobbering the
/// newer run's page.
static FILTER_GEN: AtomicU64 = AtomicU64::new(0);

/// Result of a filter run, sent back to the main loop.
#[derive(Debug)]
pub enum HomeFilterEvent {
    /// The query is in flight; show a loading indicator.
    Loading,
    /// The fetch resolved with matches.
    Results { results: FilterResults },
    /// The query matched nothing.
    Empty,
    /// The fetch failed; `reason` is a short user-facing message.
    Failed { reason: String },
}

/// Abort any in-flight fetch and start a new one.
pub fn schedule_filter(
    query: FilterQuery,
    filter_handle: &mut Option<JoinHandle<()>>,
    filter_cancel_tx: &mut Option<watch::Sender<bool>>,
    home_filter_tx: &mpsc::UnboundedSender<HomeFilterEvent>,
) {
    if let Some(handle) = filter_handle.take() {
        handle.abort();
    }
    if let Some(tx) = filter_cancel_tx.take() {
        let _ = tx.send(true);
    }

    let generation = FILTER_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let (cancel_tx, cancel_rx) = watch::channel(false);
    *filter_cancel_tx = Some(cancel_tx);

    let tx = home_filter_tx.clone();
    let handle = tokio::spawn(async move {
        run_filter_task(query, generation, cancel_rx, tx).await;
    });
    *filter_handle = Some(handle);
}

async fn run_filter_task(
    query: FilterQuery,
    generation: u64,
    mut cancel_rx: watch::Receiver<bool>,
    tx: mpsc::UnboundedSender<HomeFilterEvent>,
) {
    let _ = tx.send(HomeFilterEvent::Loading);

    tokio::select! {
        result = FILTER_CLIENT.fetch(&query) => {
            if FILTER_GEN.load(Ordering::Relaxed) != generation {
                return;
            }
            let event = match result {
                Ok(results) if results.set_ids.is_empty() => HomeFilterEvent::Empty,
                Ok(results) => HomeFilterEvent::Results { results },
                Err(err) => HomeFilterEvent::Failed { reason: map_filter_error(&err) },
            };
            let _ = tx.send(event);
        }
        _ = cancel_rx.changed() => {}
    }
}

/// Fold a filter event into the app. Returns a follow-up command (the auto-fetch
/// of the first enrichment page after id-only results land) for the runtime loop
/// to dispatch.
pub fn handle_home_filter_event(event: HomeFilterEvent, app: &mut App) -> Option<AppCommand> {
    match event {
        HomeFilterEvent::Loading => {
            app.home.find.status_msg = FindStatusMsg::Loading;
            None
        }
        HomeFilterEvent::Results { results } => {
            let sets = results.set_ids.len();
            let total_bytes = results.size_map.values().sum();
            let rows: Vec<BrowseRow> = results
                .set_ids
                .iter()
                .map(|&id| BrowseRow { id, meta: None })
                .collect();
            let find = &mut app.home.find;
            find.size_map = results.size_map;
            find.status_msg = FindStatusMsg::ReadyFilter { sets, total_bytes };
            // `set_rows` clears the pager; seed it with the matching diff ids so
            // the id-only rows backfill from the osu-batch endpoint.
            find.browse.set_rows(rows);
            find.browse.seed_enrichment(results.ids);
            // These rows came from nzbasic; record the backend + snapshot inputs.
            find.note_results_backend(FindBackend::Nzbasic);
            find.mark_results_current();
            // Open the results immediately on a fresh fetch (search parity).
            find.browse.descend();
            // Enrich what the user is about to look at: the first page.
            Some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Find,
            })
        }
        HomeFilterEvent::Empty => {
            let find = &mut app.home.find;
            find.status_msg = FindStatusMsg::Empty;
            find.size_map = HashMap::new();
            // Clears the rows and the enrichment pager in one call.
            find.browse.set_rows(Vec::new());
            find.clear_results_snapshot();
            None
        }
        HomeFilterEvent::Failed { reason } => {
            app.home.find.status_msg = FindStatusMsg::Error(reason);
            app.home.find.clear_results_snapshot();
            None
        }
    }
}

/// Short user-facing message for a filter error. The hosted instance is a free
/// solo-dev service, so failures stay soft — a toast/status line, never a hard
/// dependency.
fn map_filter_error(err: &Error) -> String {
    match err {
        Error::RateLimited { .. } => "rate limited by nzbasic (429), try again later".to_string(),
        Error::HttpStatus(status) => format!("nzbasic filter failed: HTTP {status}"),
        Error::Timeout => "nzbasic request timed out".to_string(),
        Error::Network(msg) => format!("nzbasic unreachable: {msg}"),
        Error::Parse(msg) => format!("unexpected nzbasic response: {msg}"),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/home_filter.rs"]
mod tests;
