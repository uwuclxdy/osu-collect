//! Fire-and-forget cover-image fetch for the flat set-browse preview. Mirrors
//! the nekoha size backfill (`runtime/size.rs`): a `tokio::spawn`ed task GETs
//! the beatmapset's covers, decodes the bytes to [`DynamicImage`]s on the task,
//! and hands THOSE back over a channel. The UI thread turns them into render
//! protocols via the [`Picker`](ratatui_image::picker::Picker) it owns — the
//! picker never leaves the UI thread, and `DynamicImage` is `Send`, so the task
//! carries nothing terminal-bound.
//!
//! Two variants are fetched per set: the square `list@2x` backs the right-hand
//! column, the wide `card@2x` backs the full-width banner. The preview picks per
//! pane geometry, so a terminal resize re-lays out with no refetch.
//!
//! Fail-soft: a 404, a network error, or a decode failure resolve that variant
//! to `None`; both failing settles the id text-only via `Missing`. Never panics,
//! never toasts — a cover is a progressive enhancement, so its absence is the
//! whole error path.

use super::App;
use image::DynamicImage;
use std::sync::LazyLock;
use tokio::sync::mpsc;
use tracing::debug;

/// One shared client (connection reuse) for the whole session. No auth or
/// headers — the cover CDN is public.
static COVER_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// The square `list@2x` cover (300x300): the only square asset in the set, so it
/// fills the right-hand column without shrinking to a sliver.
fn square_cover_url(set_id: u32) -> String {
    format!("https://assets.ppy.sh/beatmaps/{set_id}/covers/list@2x.jpg")
}

/// The wide `card@2x` cover (800x280, 2.9:1): the widest asset that still stands
/// a few rows tall, so a full-width banner uses the blank space above the text.
fn wide_cover_url(set_id: u32) -> String {
    format!("https://assets.ppy.sh/beatmaps/{set_id}/covers/card@2x.jpg")
}

/// A settled cover fetch, folded back into [`crate::app::covers::Covers`].
#[derive(Debug)]
pub enum HomeCoverEvent {
    /// At least one variant fetched and decoded; the UI thread builds the render
    /// protocols. A variant that failed rides as `None`.
    Loaded {
        set_id: u32,
        square: Option<DynamicImage>,
        wide: Option<DynamicImage>,
    },
    /// Both variants failed (404 / fetch error / decode failure). Text-only.
    Missing { set_id: u32 },
}

/// Spawn a fire-and-forget fetch of `set_id`'s covers (already claimed `Pending`
/// by the caller), reporting the result over the channel. No stored handle: a
/// probe left running at quit just finds the receiver dropped and ends.
pub fn schedule_cover_fetch(set_id: u32, tx: &mpsc::UnboundedSender<HomeCoverEvent>) {
    let tx = tx.clone();
    tokio::spawn(async move {
        // Both GETs concurrently; either may fail independently.
        let (square, wide) = tokio::join!(
            load_variant(set_id, square_cover_url(set_id)),
            load_variant(set_id, wide_cover_url(set_id)),
        );
        let event = if square.is_none() && wide.is_none() {
            HomeCoverEvent::Missing { set_id }
        } else {
            HomeCoverEvent::Loaded {
                set_id,
                square,
                wide,
            }
        };
        let _ = tx.send(event);
    });
}

/// GET + decode one cover variant, or `None` on any failure (logged at `debug`).
async fn load_variant(set_id: u32, url: String) -> Option<DynamicImage> {
    let response = match COVER_CLIENT.get(&url).send().await {
        Ok(response) => response,
        Err(error) => {
            debug!(set_id, %url, %error, "cover fetch failed");
            return None;
        }
    };
    if !response.status().is_success() {
        debug!(set_id, %url, status = %response.status(), "cover unavailable");
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    match image::load_from_memory(&bytes) {
        Ok(image) => Some(image),
        Err(error) => {
            debug!(set_id, %url, %error, "cover decode failed");
            None
        }
    }
}

/// Fold a settled cover fetch into the store. A `Loaded` builds each present
/// variant's render protocol with the UI-thread picker (read then insert —
/// separate borrows).
pub fn handle_home_cover_event(event: HomeCoverEvent, app: &mut App) {
    match event {
        HomeCoverEvent::Loaded {
            set_id,
            square,
            wide,
        } => {
            let square = square.map(|image| app.covers.picker.new_resize_protocol(image));
            let wide = wide.map(|image| app.covers.picker.new_resize_protocol(image));
            app.covers.record_ready(set_id, square, wide);
        }
        HomeCoverEvent::Missing { set_id } => app.covers.record_missing(set_id),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/runtime_cover.rs"]
mod tests;
