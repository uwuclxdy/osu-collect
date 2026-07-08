//! The Get Maps `Filter` background tasks: the nzbasic fetch plus the lazy
//! `beatmapDetails` enrichment, both off the UI thread, reporting back over an
//! mpsc channel. Like search, the fetch is CTA-triggered — a new run cancels
//! any in-flight one. Details pages fetch one at a time (first page auto after
//! results land, `m` in the browse for more) so a huge result set never sweeps
//! the free instance unprompted.

use crate::app::{App, AppCommand, BrowseRow, FilterStatusMsg};
use osu_downloader::Error;
use osu_downloader::filter::{BeatmapDetails, FilterClient, FilterQuery, FilterResults};
use osu_downloader::search::BeatmapSetMeta;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::{sync::mpsc, sync::watch, task::JoinHandle};

/// One shared client for the whole session (connection reuse).
static FILTER_CLIENT: LazyLock<FilterClient> = LazyLock::new(FilterClient::new);

/// Monotonic filter generation. Each `schedule_filter` bumps it; a fetch or
/// details task whose generation is stale by completion drops its result
/// rather than clobbering the newer run's page.
static FILTER_GEN: AtomicU64 = AtomicU64::new(0);

/// Result of a filter run or a details page, sent back to the main loop.
#[derive(Debug)]
pub enum HomeFilterEvent {
    /// The query is in flight; show a loading indicator.
    Loading,
    /// The fetch resolved with matches.
    Results { results: FilterResults },
    /// A `beatmapDetails` page arrived; fold the metadata into the rows.
    Details { rows: Vec<BeatmapDetails> },
    /// A details page failed; rewind the pager so `m` retries it.
    DetailsFailed { reason: String, rewind_to: usize },
    /// The query matched nothing.
    Empty,
    /// The fetch failed; `reason` is a short user-facing message.
    Failed { reason: String },
}

/// Abort any in-flight fetch/details task and start a new fetch.
pub fn schedule_filter(
    query: FilterQuery,
    filter_handle: &mut Option<JoinHandle<()>>,
    filter_cancel_tx: &mut Option<watch::Sender<bool>>,
    details_handle: &mut Option<JoinHandle<()>>,
    home_filter_tx: &mpsc::UnboundedSender<HomeFilterEvent>,
) {
    if let Some(handle) = filter_handle.take() {
        handle.abort();
    }
    if let Some(tx) = filter_cancel_tx.take() {
        let _ = tx.send(true);
    }
    if let Some(handle) = details_handle.take() {
        handle.abort();
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

/// Fetch one `beatmapDetails` page for the current results. Aborts a prior
/// details task (one page in flight at a time); the fetch generation guards
/// against a stale page landing on a newer run's rows.
pub fn schedule_filter_details(
    page: Vec<u32>,
    rewind_to: usize,
    details_handle: &mut Option<JoinHandle<()>>,
    home_filter_tx: &mpsc::UnboundedSender<HomeFilterEvent>,
) {
    if let Some(handle) = details_handle.take() {
        handle.abort();
    }

    let generation = FILTER_GEN.load(Ordering::Relaxed);
    let tx = home_filter_tx.clone();
    let handle = tokio::spawn(async move {
        let result = FILTER_CLIENT.details(&page).await;
        if FILTER_GEN.load(Ordering::Relaxed) != generation {
            return;
        }
        let event = match result {
            Ok(rows) => HomeFilterEvent::Details { rows },
            Err(err) => HomeFilterEvent::DetailsFailed {
                reason: map_filter_error(&err),
                rewind_to,
            },
        };
        let _ = tx.send(event);
    });
    *details_handle = Some(handle);
}

/// Fold a filter event into the app. Returns a follow-up command (the
/// auto-fetch of the first details page after results land) for the runtime
/// loop to dispatch.
pub fn handle_home_filter_event(event: HomeFilterEvent, app: &mut App) -> Option<AppCommand> {
    match event {
        HomeFilterEvent::Loading => {
            app.home.filter.status_msg = FilterStatusMsg::Loading;
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
            let filter = &mut app.home.filter;
            filter.set_results(results.ids, results.size_map);
            filter.status_msg = FilterStatusMsg::Ready { sets, total_bytes };
            filter.browse.set_rows(rows);
            filter.mark_results_current();
            // Open the results immediately on a fresh fetch (search parity).
            filter.browse.descend();
            // Enrich what the user is about to look at: the first details page.
            Some(AppCommand::LoadFilterDetails)
        }
        HomeFilterEvent::Details { rows } => {
            fold_details(app, rows);
            None
        }
        HomeFilterEvent::DetailsFailed { reason, rewind_to } => {
            app.home.filter.rewind_details(rewind_to);
            app.toast_warn(format!("map details unavailable: {reason}"));
            None
        }
        HomeFilterEvent::Empty => {
            let filter = &mut app.home.filter;
            filter.status_msg = FilterStatusMsg::Empty;
            filter.set_results(Vec::new(), HashMap::new());
            filter.browse.set_rows(Vec::new());
            filter.clear_results_snapshot();
            None
        }
        HomeFilterEvent::Failed { reason } => {
            app.home.filter.status_msg = FilterStatusMsg::Error(reason);
            app.home.filter.clear_results_snapshot();
            None
        }
    }
}

/// Fold per-diff detail rows into the browse rows' set-level metadata. The
/// first diff of a set wins (title/artist/creator/status are set-level in the
/// source data anyway); rows keep their fetch order.
fn fold_details(app: &mut App, rows: Vec<BeatmapDetails>) {
    let mut meta_by_set: HashMap<u32, BeatmapSetMeta> = HashMap::new();
    for row in rows {
        meta_by_set.entry(row.set_id).or_insert(BeatmapSetMeta {
            id: row.set_id,
            title: row.title,
            artist: row.artist,
            creator: row.creator,
            status: row.approved,
            favourite_count: row.favourite_count,
            play_count: row.play_count,
            nsfw: false,
            video: false,
        });
    }
    for row in &mut app.home.filter.browse.rows {
        if row.meta.is_none()
            && let Some(meta) = meta_by_set.remove(&row.id)
        {
            row.meta = Some(meta);
        }
    }
}

/// Short user-facing message for a filter/details error. The hosted instance
/// is a free solo-dev service, so failures stay soft — a toast/status line,
/// never a hard dependency.
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
