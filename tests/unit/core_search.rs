use super::{GUEST_REFRESH_MARGIN_SECS, GuestToken};

fn token(expires_at: u64) -> GuestToken {
    GuestToken {
        access_token: "t".to_string(),
        expires_at,
    }
}

// Fixed `now` so the boundary is exact and jitter-free (the wall-clock wrapper
// is covered separately below).
const NOW: u64 = 1_000_000;

#[test]
fn token_past_the_refresh_margin_is_fresh() {
    // Expires one second beyond the margin -> still fresh.
    assert!(!token(NOW + GUEST_REFRESH_MARGIN_SECS + 1).is_stale_at(NOW));
}

#[test]
fn token_exactly_at_the_refresh_margin_is_stale() {
    // Boundary: expires_at == now + margin. `>=` re-mints early, so this is stale.
    assert!(token(NOW + GUEST_REFRESH_MARGIN_SECS).is_stale_at(NOW));
}

#[test]
fn already_expired_token_is_stale() {
    assert!(token(0).is_stale_at(NOW));
}

#[test]
fn is_stale_reads_the_wall_clock() {
    // Wrapper wiring, far from any boundary so it can't flake: a token that
    // never expires is fresh, an already-expired one is stale.
    assert!(!token(u64::MAX).is_stale());
    assert!(token(0).is_stale());
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
