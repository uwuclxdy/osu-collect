use super::StoredAuth;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn make_auth(expires_at: u64) -> StoredAuth {
    StoredAuth {
        client_id: "1".into(),
        client_secret: "s".into(),
        redirect_uri: "http://localhost:7273/oauth/callback".into(),
        access_token: "tok".into(),
        refresh_token: Some("rtok".into()),
        expires_at,
        scopes: vec!["public".into(), "identify".into()],
        supporter: None,
    }
}

#[test]
fn token_not_expired_when_far_future() {
    let auth = make_auth(now_secs() + 3600);
    assert!(!auth.is_expired());
}

#[test]
fn token_expired_when_in_past() {
    let auth = make_auth(now_secs() - 1);
    assert!(auth.is_expired());
}

#[test]
fn token_expired_within_margin() {
    // expires in 30s — within the 60s refresh margin
    let auth = make_auth(now_secs() + 30);
    assert!(auth.is_expired());
}
