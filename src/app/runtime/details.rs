//! nzbasic-only details enrichment: fetches per-diff `beatmapDetails` for the
//! loaded nzbasic find results and folds each set's extra columns (tags, source,
//! genre, language, dates, plus one representative diff's combo/drain/passes/hash)
//! into the find browse's preview. Piggybacks on the osu-batch enrichment cadence
//! — the same page of diff ids, the same `m`/results-land trigger, the same
//! generation — so titles and details fill in lockstep. nzbasic-route only: the
//! osu route never fetches details (its rows carry full metadata already), and
//! collection browse&pick never does either.
//!
//! Stale-id contract: the diff ids come only from the same session's
//! [`FilterResults::ids`](osu_downloader::filter::FilterResults) via the enrichment
//! page, so the whole-batch-500s-on-one-unknown-id hazard cannot fire. Never pass
//! an id from another route.
//!
//! Fail-soft: a failed page drops silently. The osu-batch titles still render, and
//! the shared enrichment pager owns the cursor + retry — the details path only
//! misses this page's extra columns, never the whole preview, and never toasts a
//! dependency on the free instance.
//!
//! Fire-and-forget (like the size probe, no stored handle): a page left in flight
//! at reseed or quit just lands to a generation-mismatched browse (or a dropped
//! receiver) and is discarded. Not aborting means a fast-paged browse never loses
//! an in-flight page to a supersede — every dispatched page runs to completion and
//! folds if it is still current.

use crate::app::{App, EnrichSink};
use osu_downloader::filter::{BeatmapDetails, FilterClient};
use std::sync::LazyLock;
use tokio::sync::mpsc;

/// One reqwest client for every details page — its own pool to the BBD host,
/// separate from the filter task's client but configured identically.
static DETAILS_CLIENT: LazyLock<FilterClient> = LazyLock::new(FilterClient::new);

/// A details page result, sent back to the main loop.
#[derive(Debug)]
pub enum HomeDetailsEvent {
    /// A page arrived; fold its per-set columns into the find browse.
    Loaded {
        generation: u64,
        rows: Vec<BeatmapDetails>,
    },
    /// A page failed. No generation, no rewind — a failure folds nothing, the
    /// enrichment pager owns the cursor, and these sets simply keep their
    /// osu-batch metadata without the extra columns.
    Failed,
}

/// Fetch the details for one diff-id page. `generation` tags the request so a page
/// returning after a superseding reseed is dropped by [`handle_home_details_event`]
/// rather than folded into new rows.
pub fn schedule_details(
    generation: u64,
    page: Vec<u32>,
    tx: &mpsc::UnboundedSender<HomeDetailsEvent>,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let event = match DETAILS_CLIENT.details(&page).await {
            Ok(rows) => HomeDetailsEvent::Loaded { generation, rows },
            Err(_) => HomeDetailsEvent::Failed,
        };
        let _ = tx.send(event);
    });
}

/// Fold a details event into the find browse. The generation guard runs here (the
/// browse is UI-thread state the task can't read post-await): a page whose
/// generation no longer matches the browse is dropped.
pub fn handle_home_details_event(event: HomeDetailsEvent, app: &mut App) {
    match event {
        HomeDetailsEvent::Loaded { generation, rows } => {
            // A page from a superseded run drops before it can fold.
            if app.home.find.browse.enrich_generation() == generation {
                app.home.find.browse.record_details(rows);
            }
        }
        HomeDetailsEvent::Failed => {}
    }
}
