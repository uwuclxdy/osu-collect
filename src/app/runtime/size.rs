//! Lazy nekoha size backfill for osu-routed find results. Probes the download
//! size of CHECKED sets only, at most once per set id for the app session, so the
//! find form's download button can show `· ~X`. The nzbasic route keeps its own
//! `SizeMap`; the collection browse gets no sizes. Fire-and-forget and capped at
//! [`SIZE_CONCURRENCY`] in flight, so a select-all over hundreds of results
//! trickles instead of bursting the community-hosted mirror. Fail-soft: a probe
//! that can't reach the mirror records the set as sizeless (the figure just stays
//! absent — a progressive enhancement), never a toast.

use crate::app::FindSource;
use futures_util::{StreamExt, stream};
use osu_downloader::size::SizeFetcher;
use std::sync::LazyLock;
use tokio::sync::mpsc;

/// In-flight probe cap: a select-all over hundreds of results trickles at this
/// width rather than bursting the free mirror.
const SIZE_CONCURRENCY: usize = 4;

/// One shared fetcher (connection reuse) for the whole session.
static SIZE_FETCHER: LazyLock<SizeFetcher> = LazyLock::new(SizeFetcher::new);

/// A landed size probe, folded back into the find source's session cache.
#[derive(Debug)]
pub enum HomeSizeEvent {
    /// `size` is the mirror's byte count, or `None` when it has no record.
    Probed { id: u32, size: Option<u64> },
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
            .map(|id| async move { (id, probe_one(id).await) })
            .buffer_unordered(SIZE_CONCURRENCY);
        while let Some((id, size)) = probes.next().await {
            let _ = tx.send(HomeSizeEvent::Probed { id, size });
        }
    });
}

/// Probe one set's size. The lib's aggregate `fetch_sizes` over a single id is
/// the only public per-id path: `missing_count == 0` means the mirror had a
/// record, so `total_bytes` is that set's size; otherwise it has none (or the
/// request failed — indistinguishable through this API, and either way the set
/// stays sizeless for the session).
async fn probe_one(id: u32) -> Option<u64> {
    let result = SIZE_FETCHER.fetch_sizes(&[id]).await;
    (result.missing_count == 0).then_some(result.total_bytes)
}

/// Fold a landed probe into the find source's size cache.
pub fn handle_home_size_event(event: HomeSizeEvent, find: &mut FindSource) {
    match event {
        HomeSizeEvent::Probed { id, size } => find.record_size(id, size),
    }
}
