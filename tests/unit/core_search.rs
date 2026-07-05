use super::{GUEST_REFRESH_MARGIN_SECS, GuestToken, unix_now};

fn token_expiring_in(secs: u64) -> GuestToken {
    GuestToken {
        access_token: "t".to_string(),
        expires_at: unix_now() + secs,
    }
}

#[test]
fn fresh_token_is_not_stale() {
    assert!(!token_expiring_in(3600).is_stale());
}

#[test]
fn token_inside_refresh_margin_is_stale() {
    // Half the margin from expiry: still valid to the server, but we re-mint
    // early so an in-flight search never carries a token that expires en route.
    assert!(token_expiring_in(GUEST_REFRESH_MARGIN_SECS / 2).is_stale());
}

#[test]
fn already_expired_token_is_stale() {
    assert!(
        GuestToken {
            access_token: "t".to_string(),
            expires_at: 0,
        }
        .is_stale()
    );
}

// Live confirmation of the guest client_credentials grant (client 53382) + the
// search call end-to-end. Bypasses the stored-user-token branch so it exercises
// exactly the unproven guest path. Skips when no creds were compiled in.
#[tokio::test]
#[ignore = "hits the live osu! API; run with --ignored"]
async fn guest_grant_then_live_search() {
    let (Some(id), Some(secret)) = (super::OSU_CLIENT_ID, super::OSU_CLIENT_SECRET) else {
        eprintln!("no OSU_CLIENT_ID/SECRET compiled in; skipping live search");
        return;
    };

    let http = reqwest::Client::new();
    let token = crate::auth::client_credentials(&http, id, secret)
        .await
        .expect("guest client_credentials grant should succeed for client 53382");
    assert!(!token.access_token.is_empty());

    let client = osu_downloader::search::SearchClient::new();
    let query = super::SearchQuery {
        text: "tekno".to_string(),
        ..Default::default()
    };
    let results = client
        .search(&token.access_token, &query)
        .await
        .expect("guest search should succeed");

    assert!(results.total > 0, "expected matches for a common query");
    eprintln!(
        "guest search OK: total={}, page1={} sets",
        results.total,
        results.beatmapsets.len()
    );
}
