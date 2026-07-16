//! Fire-and-forget cover-image fetch for the flat set-browse preview. Mirrors
//! the nekoha size backfill (`runtime/size.rs`): a `tokio::spawn`ed task GETs
//! the beatmapset's card cover, decodes the bytes to a [`DynamicImage`] on the
//! task, and hands THAT back over a channel. The UI thread turns it into a
//! render protocol via the [`Picker`](ratatui_image::picker::Picker) it owns —
//! the picker never leaves the UI thread, and `DynamicImage` is `Send`, so the
//! task carries nothing terminal-bound.
//!
//! Fail-soft: a 404, a network error, or a decode failure all resolve to
//! `Missing`, which settles the id text-only. Never panics, never toasts — a
//! cover is a progressive enhancement, so its absence is the whole error path.

use super::App;
use image::DynamicImage;
use std::sync::LazyLock;
use tokio::sync::mpsc;
use tracing::debug;

/// One shared client (connection reuse) for the whole session. No auth or
/// headers — the cover CDN is public.
static COVER_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// The public osu! CDN card cover for a beatmapset (a ~2.9:1 wide JPEG).
fn cover_url(set_id: u32) -> String {
    format!("https://assets.ppy.sh/beatmaps/{set_id}/covers/card.jpg")
}

/// A settled cover fetch, folded back into [`crate::app::covers::Covers`].
#[derive(Debug)]
pub enum HomeCoverEvent {
    /// The cover fetched and decoded; the UI thread builds its render protocol.
    Loaded { set_id: u32, image: DynamicImage },
    /// No cover (404 / fetch error / decode failure). Settles the id text-only.
    Missing { set_id: u32 },
}

/// Spawn a fire-and-forget fetch of `set_id`'s cover (already claimed `Pending`
/// by the caller), reporting the result over the channel. No stored handle: a
/// probe left running at quit just finds the receiver dropped and ends.
pub fn schedule_cover_fetch(set_id: u32, tx: &mpsc::UnboundedSender<HomeCoverEvent>) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let event = match load_cover(set_id).await {
            Some(image) => HomeCoverEvent::Loaded { set_id, image },
            None => HomeCoverEvent::Missing { set_id },
        };
        let _ = tx.send(event);
    });
}

/// GET + decode the cover, or `None` on any failure (logged at `debug`).
async fn load_cover(set_id: u32) -> Option<DynamicImage> {
    let url = cover_url(set_id);
    let response = match COVER_CLIENT.get(&url).send().await {
        Ok(response) => response,
        Err(error) => {
            debug!(set_id, %error, "cover fetch failed");
            return None;
        }
    };
    if !response.status().is_success() {
        debug!(set_id, status = %response.status(), "cover unavailable");
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    match image::load_from_memory(&bytes) {
        Ok(image) => Some(image),
        Err(error) => {
            debug!(set_id, %error, "cover decode failed");
            None
        }
    }
}

/// Fold a settled cover fetch into the store. A `Loaded` builds the render
/// protocol with the UI-thread picker (read then insert — separate borrows).
pub fn handle_home_cover_event(event: HomeCoverEvent, app: &mut App) {
    match event {
        HomeCoverEvent::Loaded { set_id, image } => {
            let protocol = app.covers.picker.new_resize_protocol(image);
            app.covers.record_ready(set_id, protocol);
        }
        HomeCoverEvent::Missing { set_id } => app.covers.record_missing(set_id),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/runtime_cover.rs"]
mod tests;
