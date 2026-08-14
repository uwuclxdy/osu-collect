//! nzbasic-only details enrichment: walks the raw diff ids of the loaded
//! nzbasic find results in pages, fetching per-diff `beatmapDetails`. Each
//! LANDED page is the pairing source for the osu-batch enrichment pager — the
//! details rows carry `SetId`, so the handler derives ONE representative diff
//! per set (`find_source::representative_seeds`) and queues it, pruned against
//! the session cache and sets already seeded this run (~3.5x fewer
//! `GET /beatmaps?ids[]=` calls than paging every diff). A failed page falls
//! back to queueing its slice's raw diff ids, and a 200-subset response falls
//! the ids it omitted back the same way — titles never depend on this
//! endpoint returning every row. nzbasic-route only: the osu route never
//! fetches details (its rows carry full metadata already), and collection
//! browse&pick never does either.
//!
//! Stale-id contract: the diff ids come only from the same session's
//! [`FilterResults::ids`](osu_downloader::filter::FilterResults) via the
//! details walk, so the whole-batch-500s-on-one-unknown-id hazard cannot fire.
//! Never pass an id from another route.
//!
//! Fail-soft: a failed page drops its columns silently (no rewind, no retry —
//! the walk owns its cursor and never repeats a slice) but seeds the osu-batch
//! fallback, so the failure costs the preview's extra columns and nothing
//! else. The osu-batch titles still render, and the shared enrichment pager
//! owns its own cursor + retry. Never a toast — the free instance is not a
//! hard dependency.
//!
//! Fire-and-forget (like the size probe, no stored handle): a page left in flight
//! at reseed or quit just lands to a generation-mismatched browse (or a dropped
//! receiver) and is discarded. Not aborting means a fast-paged browse never loses
//! an in-flight page to a supersede — every dispatched page runs to completion and
//! folds if it is still current.

use crate::app::find_source::shortfall_ids;
use crate::app::{App, AppCommand, EnrichTarget};
use osu_downloader::filter::{BeatmapDetails, FilterClient};
use std::sync::LazyLock;
use tokio::sync::mpsc;

/// One reqwest client for every details page — its own pool to the BBD host,
/// separate from the filter task's client but configured identically.
static DETAILS_CLIENT: LazyLock<FilterClient> = LazyLock::new(FilterClient::new);

/// A details page result, sent back to the main loop.
#[derive(Debug)]
pub enum HomeDetailsEvent {
    /// A page arrived; fold its per-set columns into the find browse and
    /// derive its one-per-set seeds for the osu-batch pager. `ids` is the
    /// requested slice, so the handler can route a 200-subset shortfall —
    /// the requested ids the response omitted — through raw seeding instead
    /// of stranding them.
    Loaded {
        generation: u64,
        ids: Vec<u32>,
        rows: Vec<BeatmapDetails>,
    },
    /// A page failed. No rewind — the walk already advanced past this slice
    /// and its extra columns are lost for the run (fail-soft, as before). The
    /// slice's raw `ids` fall back to seeding the osu-batch pager unpaired,
    /// which is exactly the pre-rework behavior: titles never wait on this
    /// endpoint.
    Failed { generation: u64, ids: Vec<u32> },
}

/// Fetch one diff-id page. `generation` tags the request so a page returning
/// after a superseding reseed is dropped by [`handle_home_details_event`]
/// rather than folded into new rows.
pub fn schedule_details(
    generation: u64,
    page: Vec<u32>,
    tx: &mpsc::UnboundedSender<HomeDetailsEvent>,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let event = match DETAILS_CLIENT.details(&page).await {
            Ok(rows) => HomeDetailsEvent::Loaded {
                generation,
                ids: page,
                rows,
            },
            Err(_) => HomeDetailsEvent::Failed {
                generation,
                ids: page,
            },
        };
        let _ = tx.send(event);
    });
}

/// Fold a details event into the find browse. The generation guard runs here (the
/// browse is UI-thread state the task can't read post-await): a page whose
/// generation no longer matches the details walk is dropped. Returns a
/// follow-up `LoadEnrichment{Find}` when the landing queued new seeds, so the
/// runtime dispatches their osu-batch page — titles follow a details landing
/// without waiting for `m`.
pub fn handle_home_details_event(event: HomeDetailsEvent, app: &mut App) -> Option<AppCommand> {
    match event {
        HomeDetailsEvent::Loaded {
            generation,
            ids,
            rows,
        } => {
            let browse = &mut app.home.find.browse;
            // A page from a superseded run drops before it can fold or seed.
            if browse.details_walk_generation() != generation {
                return None;
            }
            browse.mark_details_settled();
            let mut queued = {
                let cache = &app.home.meta_cache;
                browse.queue_details_seeds(&rows, cache)
            };
            // A 200-subset response stranded the requested ids it didn't
            // return: they have no pairing to derive from, so they fall back
            // to raw seeding. Titles never depend on the details endpoint
            // returning every row.
            queued += browse.queue_raw_details_seeds(shortfall_ids(&ids, &rows));
            browse.record_details(rows);
            (queued > 0).then_some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Find,
            })
        }
        HomeDetailsEvent::Failed { generation, ids } => {
            let browse = &mut app.home.find.browse;
            // A stale failure drops silently — a superseding reseed already
            // invalidated the slice, so there is nothing to seed or report.
            if browse.details_walk_generation() != generation {
                return None;
            }
            browse.mark_details_settled();
            let queued = browse.queue_raw_details_seeds(ids);
            (queued > 0).then_some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Find,
            })
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/home_details.rs"]
mod tests;
