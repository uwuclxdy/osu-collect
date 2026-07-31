use super::super::{App, AuthLoginState, Toast};
use crate::auth;
use crate::utils::AppError;
use tokio::sync::mpsc;
use tracing::debug;

// Each variant reports a finished async auth task; the shared `Complete` suffix
// is intentional and reads correctly at the match sites.
#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub(super) enum AuthEvent {
    /// Password (ROPC) grant finished.
    LazerLoginComplete(Result<auth::LazerLoginOutcome, String>),
    /// Session-verification code submission finished. Carries the resulting
    /// supporter status — the original login's `/me` probe 401'd before it
    /// could learn it, so it's re-probed once verification succeeds.
    VerificationComplete(Result<bool, String>),
    /// Verification-code reissue finished.
    ReissueComplete(Result<(), String>),
    LogoutComplete(Result<(), String>),
    /// A background `/me` re-probe of an already-stored token answered. Only
    /// ever sent for a CONFIRMED answer — an unreachable or unauthorized probe
    /// sends nothing, so a flaky network can never revoke supporter features.
    SupporterRefreshed(bool),
}

pub(super) fn handle_auth_event(event: AuthEvent, app: &mut App) {
    match event {
        // Discard a login / verify result if the user cancelled mid-flight (the
        // login state already left `InProgress`) or it otherwise settled.
        AuthEvent::LazerLoginComplete(_) | AuthEvent::VerificationComplete(_)
            if !matches!(app.config.login_state, AuthLoginState::InProgress(_)) =>
        {
            debug!("Discarding stale auth event (cancelled or already settled)");
        }
        AuthEvent::LazerLoginComplete(Ok(outcome)) => {
            if outcome.needs_verification {
                // The token is saved but device verification is pending, so it
                // can't download yet — treat as logged-out until verified.
                app.set_logged_out();
                // The open panel advances to the verification step and shows the
                // instruction inline; only toast when it's closed (a login can
                // finish after the panel was dismissed mid-flight).
                if let Some(login) = app.login.as_mut() {
                    login.enter_verification();
                } else {
                    app.push_toast(
                        Toast::warning("verification needed")
                            .with_detail("enter the code osu! emailed you"),
                    );
                }
            } else {
                app.set_login_complete(outcome.supporter);
                if let Some(login) = app.login.as_mut() {
                    login.enter_logged_in();
                } else {
                    app.toast_ok("login successful");
                }
            }
        }
        AuthEvent::LazerLoginComplete(Err(err)) => {
            app.set_login_failed();
            if let Some(login) = app.login.as_mut() {
                login.reset_credentials();
            }
            app.push_toast(Toast::danger("login failed").with_detail(err));
        }
        AuthEvent::VerificationComplete(Ok(supporter)) => {
            app.set_login_complete(supporter);
            // The open panel shows the logged-in state inline; only toast when
            // it's closed.
            if let Some(login) = app.login.as_mut() {
                login.enter_logged_in();
            } else {
                app.toast_ok("login successful");
            }
        }
        AuthEvent::VerificationComplete(Err(err)) => {
            // Stay on the verification step (phase unchanged) so the user can
            // re-enter the code; just drop the in-progress status.
            app.set_logged_out();
            app.push_toast(Toast::danger("verification failed").with_detail(err));
        }
        AuthEvent::ReissueComplete(Ok(())) => {
            app.toast_ok("verification code resent");
        }
        AuthEvent::ReissueComplete(Err(err)) => {
            app.push_toast(Toast::danger("could not resend code").with_detail(err));
        }
        AuthEvent::LogoutComplete(Ok(())) => {
            app.set_logged_out();
            if let Some(login) = app.login.as_mut() {
                login.reset_credentials();
            }
            app.toast_ok("logged out");
        }
        AuthEvent::LogoutComplete(Err(err)) => {
            app.set_login_failed();
            app.push_toast(Toast::danger("logout failed").with_detail(err));
        }
        AuthEvent::SupporterRefreshed(supporter) => {
            app.set_supporter(supporter);
        }
    }
}

pub(super) fn spawn_lazer_login_task(
    username: String,
    password: String,
    tx: mpsc::UnboundedSender<AuthEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let result = auth::lazer_login(&client, &username, &password)
            .await
            .map_err(|err| err.to_string());
        let _ = tx.send(AuthEvent::LazerLoginComplete(result));
    })
}

pub(super) fn spawn_verification_task(
    code: String,
    tx: mpsc::UnboundedSender<AuthEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let result = submit_verification(&code)
            .await
            .map_err(|err| err.to_string());
        let _ = tx.send(AuthEvent::VerificationComplete(result));
    })
}

/// Fire-and-forget like logout: the handle is not stored, so it never shares
/// the cancellable `tasks.login` slot with a login / verify request.
pub(super) fn spawn_reissue_task(tx: mpsc::UnboundedSender<AuthEvent>) {
    tokio::spawn(async move {
        let result = reissue_verification().await.map_err(|err| err.to_string());
        let _ = tx.send(AuthEvent::ReissueComplete(result));
    });
}

/// Re-probe `/me` for the supporter flag of a session that was ALREADY logged in
/// when the app started. Without this the flag is whatever the last completed
/// login wrote: a token stored before the field existed carries no answer at all
/// and reads as not-a-supporter forever, and an account that gained or lost
/// supporter since keeps the stale one — in both cases with no in-app way out
/// short of a full logout/login.
///
/// Fire-and-forget: no stored handle, no await on the startup path, and a probe
/// that answers nothing sends no event, so startup is unaffected by a failure.
pub(super) fn spawn_supporter_refresh_task(tx: mpsc::UnboundedSender<AuthEvent>) {
    tokio::spawn(async move {
        let Some(mut stored) = auth::load() else {
            return;
        };
        let client = reqwest::Client::new();
        // `/me` on an expired token 401s and carries no supporter data, so a
        // session idle past the token lifetime would learn nothing. A refresh
        // failure is not fatal — the probe below then just answers nothing.
        if let Err(err) = auth::ensure_valid(&client, &mut stored).await {
            debug!(error = %err, "token refresh before the supporter probe failed");
        }
        if let Some(supporter) = auth::refresh_supporter_status(&client, &mut stored).await {
            let _ = tx.send(AuthEvent::SupporterRefreshed(supporter));
        }
    });
}

pub(super) fn spawn_logout_task(tx: mpsc::UnboundedSender<AuthEvent>) {
    tokio::task::spawn_blocking(move || {
        let result = auth::delete().map_err(|err| err.to_string());
        let _ = tx.send(AuthEvent::LogoutComplete(result));
    });
}

/// Load the stored token and submit the verification code against it. On
/// success, re-probes `/me` for supporter status — unknown until now, since
/// the original login's probe 401'd — and returns the confirmed value.
async fn submit_verification(code: &str) -> crate::utils::Result<bool> {
    let mut stored =
        auth::load().ok_or_else(|| AppError::other_dynamic(Box::from("not logged in")))?;
    let client = reqwest::Client::new();
    auth::submit_session_verification(&client, stored.bearer_token(), code).await?;
    // An unanswered probe leaves the cached value standing rather than claiming
    // a status the server never confirmed.
    Ok(auth::refresh_supporter_status(&client, &mut stored)
        .await
        .unwrap_or_else(|| stored.is_supporter()))
}

/// Load the stored token and ask osu! to re-send the verification code.
async fn reissue_verification() -> crate::utils::Result<()> {
    let stored = auth::load().ok_or_else(|| AppError::other_dynamic(Box::from("not logged in")))?;
    let client = reqwest::Client::new();
    auth::reissue_session_verification(&client, stored.bearer_token()).await
}

#[cfg(test)]
#[path = "../../../tests/unit/runtime_auth.rs"]
mod tests;
