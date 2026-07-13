//! Shared osu-batch enrichment: lazily backfills id-only browse rows with
//! set-level metadata via `GET /api/v2/beatmaps?ids[]=`. Replaces the nzbasic
//! `beatmapDetails` pager for every id-only browse — nzbasic-routed find results
//! and collection browse&pick. One page in flight per target browse (find and
//! collection each own a task slot); the first page auto-fetches
//! when id-only rows land / the browse descends, `m` loads more. Fail-soft: a
//! failed page rewinds so `m` retries, and a failure never becomes a hard
//! dependency (a toast, not a stall). Token resolution reuses the session-wide
//! [`HttpSearchService`](crate::core::search::HttpSearchService) — the same guest
//! or user bearer as search, never a second token path.

use crate::app::{App, EnrichSink, EnrichTarget};
use crate::core::search::{BeatmapRow, BeatmapSetMeta, shared_service};
use std::collections::HashMap;
use tokio::{sync::mpsc, task::JoinHandle};

/// An enrichment page result, sent back to the main loop.
#[derive(Debug)]
pub enum EnrichEvent {
    /// A batch page arrived; fold its set-level metadata into the target browse.
    Enriched {
        target: EnrichTarget,
        generation: u64,
        rows: Vec<BeatmapRow>,
    },
    /// A page failed; rewind the pager so `m` retries it.
    Failed {
        target: EnrichTarget,
        generation: u64,
        rewind_to: usize,
        reason: String,
    },
}

/// Fetch one enrichment page for `target`. Aborts the target's prior page task
/// (one page in flight per target — `enrich_handle` is that target's own slot);
/// `generation` tags the request so a page returning after a superseding reseed
/// is dropped rather than folded into the new rows.
pub fn schedule_enrichment(
    target: EnrichTarget,
    generation: u64,
    page: Vec<u32>,
    rewind_to: usize,
    enrich_handle: &mut Option<JoinHandle<()>>,
    enrich_tx: &mpsc::UnboundedSender<EnrichEvent>,
) {
    if let Some(handle) = enrich_handle.take() {
        handle.abort();
    }

    let tx = enrich_tx.clone();
    let handle = tokio::spawn(async move {
        let event = match shared_service().beatmaps(&page).await {
            Ok(rows) => EnrichEvent::Enriched {
                target,
                generation,
                rows,
            },
            Err(err) => EnrichEvent::Failed {
                target,
                generation,
                rewind_to,
                reason: err.to_string(),
            },
        };
        let _ = tx.send(event);
    });
    *enrich_handle = Some(handle);
}

/// Fold an enrichment event into the app. The generation guard runs here (the
/// pager is UI-thread state the task can't read post-await): a page whose
/// generation no longer matches the target browse is dropped.
pub fn handle_enrich_event(event: EnrichEvent, app: &mut App) {
    match event {
        EnrichEvent::Enriched {
            target,
            generation,
            rows,
        } => {
            let sink = enrich_sink_mut(app, target);
            if sink.enrich_generation() != generation {
                return;
            }
            // This generation's in-flight page landed — clear the loading cue.
            sink.set_enriching(false);
            // Dedupe to set-level metadata; the first diff of a set wins (title /
            // artist / creator / status are set-level, and the batch nests each
            // row's full set). Holes (ids the server omitted) simply contribute no
            // row, so their sets stay id-only.
            let mut meta_by_set: HashMap<u32, BeatmapSetMeta> = HashMap::new();
            for row in rows {
                meta_by_set
                    .entry(row.beatmapset_id)
                    .or_insert(row.beatmapset);
            }
            sink.fold_meta(meta_by_set);
        }
        EnrichEvent::Failed {
            target,
            generation,
            rewind_to,
            reason,
        } => {
            let sink = enrich_sink_mut(app, target);
            // A stale page's failure drops silently — a superseding reseed
            // already invalidated it, so there is nothing to retry or report.
            if sink.enrich_generation() != generation {
                return;
            }
            sink.set_enriching(false);
            sink.rewind_enrichment(rewind_to);
            app.toast_warn(format!("map details unavailable: {reason}"));
        }
    }
}

/// The [`EnrichSink`] an enrichment page targets.
pub fn enrich_sink_mut(app: &mut App, target: EnrichTarget) -> &mut dyn EnrichSink {
    match target {
        EnrichTarget::Find => &mut app.home.find.browse,
        EnrichTarget::Collection => &mut app.home.collection_browse,
        EnrichTarget::Update => &mut app.home.update,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/home_enrich.rs"]
mod tests;
