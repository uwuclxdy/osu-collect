use super::{SearchQuery, SearchResults};
use crate::utils::{AppError, Result};
use osu_downloader::{Error, search::SearchClient};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// osu-collect's own registered app credentials, injected by `build.rs` from
/// `.env` / CI env. `None` in a fork build with no creds — search then degrades
/// to login-required.
const OSU_CLIENT_ID: Option<&str> = option_env!("OSU_CLIENT_ID");
const OSU_CLIENT_SECRET: Option<&str> = option_env!("OSU_CLIENT_SECRET");

/// Re-mint the guest token this many seconds before it actually expires.
const GUEST_REFRESH_MARGIN_SECS: u64 = 60;

pub trait SearchService: Send + Sync {
    fn search(
        &self,
        query: &SearchQuery,
    ) -> impl std::future::Future<Output = Result<SearchResults>> + Send;
}

/// A cached guest `client_credentials` token.
struct GuestToken {
    access_token: String,
    expires_at: u64,
}

impl GuestToken {
    fn is_stale(&self) -> bool {
        self.is_stale_at(unix_now())
    }

    fn is_stale_at(&self, now: u64) -> bool {
        now + GUEST_REFRESH_MARGIN_SECS >= self.expires_at
    }
}

/// osu! API v2 search wrapper. Resolves the bearer token per call: a stored user
/// token (full features) when logged in, else a cached guest `client_credentials`
/// token minted from the build-injected app creds, else a login-required error.
pub struct HttpSearchService {
    client: SearchClient,
    token_client: reqwest::Client,
    guest: Mutex<Option<GuestToken>>,
}

impl HttpSearchService {
    pub fn new(client: SearchClient) -> Self {
        Self {
            client,
            token_client: reqwest::Client::new(),
            guest: Mutex::new(None),
        }
    }

    async fn resolve_token(&self) -> Result<String> {
        // Logged in: use the stored user token (unlocks supporter r/played).
        if let Some(mut auth) = crate::auth::load() {
            crate::auth::ensure_valid(&self.token_client, &mut auth).await?;
            return Ok(auth.bearer_token().to_string());
        }

        // Logged out: guest client_credentials from the app's own creds.
        let (Some(client_id), Some(client_secret)) = (OSU_CLIENT_ID, OSU_CLIENT_SECRET) else {
            return Err(AppError::api(
                "log in to search (no search credentials in this build)",
            ));
        };

        // Single-flight the guest mint: hold the lock across the grant so
        // concurrent logged-out searches share one token instead of racing two.
        // (The user-token branch above delegates freshness to `ensure_valid`.)
        let mut guard = self.guest.lock().await;
        if let Some(token) = guard.as_ref()
            && !token.is_stale()
        {
            return Ok(token.access_token.clone());
        }

        let resp =
            crate::auth::client_credentials(&self.token_client, client_id, client_secret).await?;
        let access_token = resp.access_token.clone();
        *guard = Some(GuestToken {
            access_token: resp.access_token,
            expires_at: unix_now() + resp.expires_in,
        });
        Ok(access_token)
    }
}

impl SearchService for HttpSearchService {
    async fn search(&self, query: &SearchQuery) -> Result<SearchResults> {
        let token = self.resolve_token().await?;
        self.client
            .search(&token, query)
            .await
            .map_err(map_search_error)
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn map_search_error(err: Error) -> AppError {
    match err {
        Error::RateLimited { .. } => {
            AppError::api("rate limited by osu! (429). please try again later.")
        }
        Error::HttpStatus(401) => AppError::api("search requires login (401)"),
        Error::HttpStatus(status) => {
            AppError::api_dynamic(format!("search failed: HTTP {status}").into_boxed_str())
        }
        Error::Timeout => AppError::api("search request timed out"),
        Error::Network(msg) => AppError::api_dynamic(msg.into_boxed_str()),
        Error::Parse(msg) => AppError::api_dynamic(
            format!("failed to parse search response: {msg}").into_boxed_str(),
        ),
        other => AppError::api_dynamic(other.to_string().into_boxed_str()),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/core_search.rs"]
mod tests;
