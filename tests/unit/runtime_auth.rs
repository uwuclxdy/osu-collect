//! How a finished auth task lands on the app.
//!
//! The startup supporter re-probe is the reason this file exists: the flag it
//! moves gates six find-form rows, and before the re-probe existed the only way
//! to change it was a full logout/login.

use super::*;
use crate::app::AuthLoginState;
use crate::config::Config;

/// An app with a known auth state. `ConfigTab::new` seeds `supporter` from the
/// stored token on the machine running this, so both are pinned explicitly —
/// otherwise these pass or fail on whether the developer is logged in.
fn app_logged_in(supporter: bool) -> App {
    let mut app = App::new(Config::default());
    app.config.login_state = AuthLoginState::LoggedIn;
    app.config.supporter = supporter;
    app
}

#[test]
fn a_confirmed_probe_reaches_the_supporter_gate() {
    let mut app = app_logged_in(false);
    handle_auth_event(AuthEvent::SupporterRefreshed(true), &mut app);
    assert!(
        app.config.supporter,
        "the startup re-probe is the only thing that opens the gate for a \
         session logged in before the flag was ever stored"
    );
}

#[test]
fn a_confirmed_lapse_closes_the_supporter_gate() {
    let mut app = app_logged_in(true);
    handle_auth_event(AuthEvent::SupporterRefreshed(false), &mut app);
    assert!(!app.config.supporter, "supporter expires; the gate follows");
}

/// The probe answer is scoped to the token it was made against. A logout that
/// landed first already zeroed the flag, and the stale answer must not undo it.
#[test]
fn a_probe_landing_after_a_logout_is_ignored() {
    let mut app = app_logged_in(true);
    handle_auth_event(AuthEvent::LogoutComplete(Ok(())), &mut app);
    assert!(!app.config.supporter);
    handle_auth_event(AuthEvent::SupporterRefreshed(true), &mut app);
    assert!(
        !app.config.supporter,
        "a logged-out session must not be re-granted by an in-flight probe"
    );
}
