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
            // Generation guard first: a page from a superseded run drops before it
            // can fold or write the cache.
            if enrich_sink_mut(app, target).enrich_generation() != generation {
                return;
            }
            // Collapse to set-level metadata: the first diff of a set wins the
            // set's title/artist/creator/status (identical across a set's diffs),
            // and every row folds its difficulty into the set's `beatmaps[]` so an
            // id-only browse gains the full difficulty spread. Holes (ids the
            // server omitted) contribute no row, so their sets stay id-only.
            let mut meta_by_set: HashMap<u32, BeatmapSetMeta> = HashMap::new();
            for row in rows {
                let set_id = row.beatmap.beatmapset_id;
                let meta = meta_by_set.entry(set_id).or_insert(row.beatmapset);
                meta.beatmaps.push(row.beatmap);
            }
            // Every landed page feeds the session cache (cache-miss only, so a
            // title never clobbers; osu search rows feed it separately in
            // `runtime/search.rs`), so a later reopen / rescan / re-resolve of
            // any browse skips the refetch entirely.
            for (set, meta) in &meta_by_set {
                app.home
                    .meta_cache
                    .entry(*set)
                    .or_insert_with(|| meta.clone());
            }
            let sink = enrich_sink_mut(app, target);
            // This page settled — one fewer outstanding (a counter, so a newer
            // dispatch's cue survives a stale page's late event).
            sink.mark_enrichment_settled();
            sink.fold_meta(meta_by_set);
        }
        EnrichEvent::Failed {
            target,
            generation,
            rewind_to,
            reason,
        } => {
            // A stale page's failure drops silently — a superseding reseed
            // already invalidated it, so there is nothing to retry or report.
            if enrich_sink_mut(app, target).enrich_generation() != generation {
                return;
            }
            {
                let sink = enrich_sink_mut(app, target);
                sink.mark_enrichment_settled();
                sink.rewind_enrichment(rewind_to);
            }
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
