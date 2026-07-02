//! Self-update lifecycle surfaced to the UI.
//!
//! At startup a detached task runs once. With auto-update on it downloads and
//! installs a newer release, reporting progress as toasts (downloading →
//! restart). With auto-update off it only reports that an update is available
//! (`Available`); the user then opens the `u` modal and confirms, which spawns
//! the same download+apply flow. A cargo/dev build is forced into notify-only
//! regardless of the setting, since self-replacing a `target/` binary is moot.

use super::super::{App, Toast, ToastTag};
use crate::auto_update::{AvailableUpdate, check_and_apply, check_for_update, is_cargo_build};
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Debug)]
pub(super) enum UpdateEvent {
    /// Notify-only mode found a newer release (auto-update disabled).
    Available(AvailableUpdate),
    /// A newer release was found; the download has started.
    Downloading,
    /// The new binary was installed; the app must be restarted to apply it.
    Installed,
    /// The check or download failed. Best-effort; carries the reason.
    Failed(String),
}

/// Spawn the one-shot background update check. `auto` selects the mode: set =
/// download + apply automatically; clear = only surface availability.
/// `prereleases` opts the check into the prerelease channel.
pub(super) fn spawn_update_check(
    tx: mpsc::UnboundedSender<UpdateEvent>,
    auto: bool,
    prereleases: bool,
) {
    // A cargo/dev build can't meaningfully self-replace its `target/` binary, so
    // fall back to notify-only there — the update still surfaces without clobbering
    // the compiled artifact.
    let auto = auto && !is_cargo_build();
    tokio::spawn(async move {
        if auto {
            report_apply(&tx, prereleases).await;
        } else {
            match check_for_update(prereleases).await {
                Ok(Some(info)) => {
                    let _ = tx.send(UpdateEvent::Available(info));
                }
                Ok(None) => {} // up to date — stay silent
                Err(err) => warn!(error = %err, "Update check failed"),
            }
        }
    });
}

/// Spawn the download+apply flow the user confirmed from the update modal.
/// `prereleases` must match the channel that surfaced the update so the apply
/// re-finds the same release.
pub(super) fn spawn_apply_update(tx: mpsc::UnboundedSender<UpdateEvent>, prereleases: bool) {
    tokio::spawn(async move {
        report_apply(&tx, prereleases).await;
    });
}

/// Run `check_and_apply`, reporting the download → install/fail outcome as
/// `UpdateEvent`s. Silent when nothing newer exists.
async fn report_apply(tx: &mpsc::UnboundedSender<UpdateEvent>, prereleases: bool) {
    let found_tx = tx.clone();
    let result = check_and_apply(
        move || {
            let _ = found_tx.send(UpdateEvent::Downloading);
        },
        prereleases,
    )
    .await;
    let outcome = match result {
        Ok(Some(_message)) => UpdateEvent::Installed,
        Ok(None) => return,
        Err(err) => UpdateEvent::Failed(err.to_string()),
    };
    let _ = tx.send(outcome);
}

pub(super) fn handle_update_event(event: UpdateEvent, app: &mut App) {
    match event {
        UpdateEvent::Available(info) => app.set_available_update(info),
        UpdateEvent::Downloading => {
            app.push_toast(
                Toast::info("downloading update")
                    .until_resolved()
                    .tagged(ToastTag::Update),
            );
        }
        UpdateEvent::Installed => {
            app.toasts.replace_tagged(
                ToastTag::Update,
                Toast::success("update installed")
                    .with_detail("restart osu!collect to finish update")
                    .until_dismissed(),
            );
        }
        UpdateEvent::Failed(err) => {
            warn!(error = %err, "Auto-update failed; a new version may be available");
            app.toasts.replace_tagged(
                ToastTag::Update,
                Toast::danger("update failed")
                    .with_detail(err)
                    .tagged(ToastTag::Update),
            );
        }
    }
}
