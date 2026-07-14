//! Lazy nekoha size backfill for osu-routed find results. Probes the download
//! size of CHECKED sets only, at most once per set id for the app session, so the
//! find form's download button can show `· ~X`. The nzbasic route gets its sizes
//! free from its own response's `SizeMap` (folded into the same cache by
//! `runtime/filter.rs`) and never probes; the collection browse gets no sizes.
//! Fire-and-forget and capped at [`SIZE_CONCURRENCY`] in flight, so a select-all
//! over hundreds of results trickles instead of bursting the community-hosted
//! mirror.
//!
//! Fail-soft, and the two failures stay apart: the mirror answering "no size for
//! this set" settles the id for the session, while a probe that never reached the
//! mirror releases its claim so the next selection change retries. Neither ever
//! raises a toast — the figure is a progressive enhancement, so its absence is
//! the whole error path.

use crate::app::FindSource;
use futures_util::{StreamExt, stream};
use osu_downloader::size::SizeFetcher;
use std::sync::LazyLock;
use tokio::sync::mpsc;
use tracing::debug;

/// In-flight probe cap: a select-all over hundreds of results trickles at this
/// width rather than bursting the free mirror.
const SIZE_CONCURRENCY: usize = 4;

/// One shared fetcher (connection reuse) for the whole session.
static SIZE_FETCHER: LazyLock<SizeFetcher> = LazyLock::new(SizeFetcher::new);

/// A settled size probe, folded back into the find source's session cache.
#[derive(Debug)]
pub enum HomeSizeEvent {
    /// The mirror answered: `size` is its byte count, or `None` when it has no
    /// size for this set. Settles the id — it is not probed again this session.
    Probed { id: u32, size: Option<u64> },
    /// The mirror could not be reached, which says nothing about the set. Releases
    /// the id's claim so the next selection change retries it.
    Failed { id: u32 },
}

/// Spawn a fire-and-forget probe of `ids` (already claimed `Pending` by the
/// caller), reporting each result over the channel, bounded by
/// [`SIZE_CONCURRENCY`]. No stored handle: the sizes are optional, so a probe
/// left running at quit just finds the receiver dropped and ends.
pub fn schedule_size_probe(ids: Vec<u32>, tx: &mpsc::UnboundedSender<HomeSizeEvent>) {
    if ids.is_empty() {
        return;
    }
    let tx = tx.clone();
    tokio::spawn(async move {
        let mut probes = stream::iter(ids)
            .map(|id| async move { (id, SIZE_FETCHER.fetch_size(id).await) })
            .buffer_unordered(SIZE_CONCURRENCY);
        while let Some((id, result)) = probes.next().await {
            let _ = tx.send(match result {
                Ok(size) => HomeSizeEvent::Probed { id, size },
                Err(err) => {
                    debug!(id, error = %err, "size probe failed; will retry on reselect");
                    HomeSizeEvent::Failed { id }
                }
            });
        }
    });
}

/// Fold a settled probe into the find source's size cache.
pub fn handle_home_size_event(event: HomeSizeEvent, find: &mut FindSource) {
    match event {
        HomeSizeEvent::Probed { id, size } => find.record_size(id, size),
        HomeSizeEvent::Failed { id } => find.release_size_probe(id),
    }
}
