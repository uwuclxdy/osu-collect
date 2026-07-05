//! osu! API v2 beatmapset search client.
//!
//! Behind the `search` feature. The client is auth-agnostic: the caller supplies
//! a bearer token (a guest `client_credentials` token or a user token) and this
//! module never imports [`crate::auth`]. Token resolution lives on the app side.

use crate::{Error, Result, http};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Base URL for osu! API v2. Matches the app-side `OSU_API_BASE`; kept local so
/// the library stays decoupled from the binary's auth module.
const OSU_API_BASE: &str = "https://osu.ppy.sh/api/v2";

/// `x-api-version` header value. Mandatory on every api v2 call — osu! rejects
/// requests that omit it. Any recent `YYYYMMDD` integer works.
const X_API_VERSION: &str = "20250115";

/// Game mode filter (osu! `m` parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// osu!standard (`m=0`).
    Osu,
    /// osu!taiko (`m=1`).
    Taiko,
    /// osu!catch (`m=2`).
    Fruits,
    /// osu!mania (`m=3`).
    Mania,
}

impl SearchMode {
    /// The `m` parameter value.
    fn as_param(self) -> &'static str {
        match self {
            Self::Osu => "0",
            Self::Taiko => "1",
            Self::Fruits => "2",
            Self::Mania => "3",
        }
    }
}

/// Category / rank-status filter (osu! `s` parameter). Only the guest-safe subset
/// is modelled; `favourites`/`mine` need a user token and are omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchStatus {
    /// Every category.
    Any,
    /// Has a leaderboard (ranked + approved + loved).
    Leaderboard,
    /// Ranked.
    Ranked,
    /// Qualified.
    Qualified,
    /// Loved.
    Loved,
    /// Pending.
    Pending,
    /// Work-in-progress.
    Wip,
    /// Graveyard.
    Graveyard,
}

impl SearchStatus {
    /// The `s` parameter value.
    fn as_param(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Leaderboard => "leaderboard",
            Self::Ranked => "ranked",
            Self::Qualified => "qualified",
            Self::Loved => "loved",
            Self::Pending => "pending",
            Self::Wip => "wip",
            Self::Graveyard => "graveyard",
        }
    }
}

/// Sort field (the `{field}` half of the `sort` parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    /// Text-match relevance (only meaningful with a non-empty query).
    Relevance,
    /// Title.
    Title,
    /// Artist.
    Artist,
    /// Mapper.
    Creator,
    /// Star difficulty.
    Difficulty,
    /// Favourite count.
    Favourites,
    /// Nomination count.
    Nominations,
    /// Play count.
    Plays,
    /// Ranked date.
    Ranked,
    /// User rating.
    Rating,
    /// Last-updated date.
    Updated,
}

impl SortField {
    /// The `{field}` half of the `sort` parameter value.
    fn as_param(self) -> &'static str {
        match self {
            Self::Relevance => "relevance",
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Creator => "creator",
            Self::Difficulty => "difficulty",
            Self::Favourites => "favourites",
            Self::Nominations => "nominations",
            Self::Plays => "plays",
            Self::Ranked => "ranked",
            Self::Rating => "rating",
            Self::Updated => "updated",
        }
    }
}

/// Sort order (the `{asc|desc}` half of the `sort` parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    /// Ascending.
    Asc,
    /// Descending.
    Desc,
}

impl SortOrder {
    /// The `{asc|desc}` half of the `sort` parameter value.
    fn as_param(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

/// A search query. Serialized to url parameters by [`SearchClient::search`];
/// [`cursor`](Self::cursor) threads cursor-based pagination.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Free text (`q`). Empty is allowed; the server applies its default sort.
    pub text: String,
    /// Game mode (`m`). `None` searches every mode.
    pub mode: Option<SearchMode>,
    /// Category / rank status (`s`). `None` uses the server default (has-leaderboard).
    pub status: Option<SearchStatus>,
    /// Sort field + order. `None` uses the server default (`relevance` with a
    /// query, else `ranked`; `desc`).
    pub sort: Option<(SortField, SortOrder)>,
    /// Pagination cursor (`cursor_string`). `None` requests the first page;
    /// resend the previous response's [`SearchResults::cursor_string`] for the next.
    pub cursor: Option<String>,
}

/// One normalized result row. A compact subset of the osu! `beatmapsets[]`
/// element — the set-level fields the download pipeline and preview need.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BeatmapSetMeta {
    /// Beatmapset id — the id the download pipeline consumes.
    pub id: u32,
    /// Romanised title.
    pub title: String,
    /// Romanised artist.
    pub artist: String,
    /// Mapper username.
    pub creator: String,
    /// Rank status string (`"ranked"`, `"loved"`, `"graveyard"`, …).
    pub status: String,
    /// Number of favourites.
    #[serde(default)]
    pub favourite_count: u32,
    /// Number of plays.
    #[serde(default)]
    pub play_count: u32,
    /// Whether the set is flagged NSFW.
    #[serde(default)]
    pub nsfw: bool,
    /// Whether any difficulty carries a video.
    #[serde(default)]
    pub video: bool,
}

/// A deserialized search response. `total` feeds the result count;
/// `cursor_string` threads the next page (`None` = last page).
#[derive(Debug, Clone, Deserialize)]
pub struct SearchResults {
    /// The result rows for this page.
    pub beatmapsets: Vec<BeatmapSetMeta>,
    /// Total matches across all pages.
    #[serde(default)]
    pub total: u64,
    /// Cursor for the next page; `None`/`null` marks the last page.
    #[serde(default)]
    pub cursor_string: Option<String>,
}

/// Serialize a query to `(key, value)` url parameters. `q` is always sent (empty
/// allowed); the rest are omitted when unset so the server applies its defaults.
fn build_query_params(query: &SearchQuery) -> Vec<(&'static str, String)> {
    let mut params: Vec<(&'static str, String)> = vec![("q", query.text.clone())];
    if let Some(mode) = query.mode {
        params.push(("m", mode.as_param().to_string()));
    }
    if let Some(status) = query.status {
        params.push(("s", status.as_param().to_string()));
    }
    if let Some((field, order)) = query.sort {
        params.push(("sort", format!("{}_{}", field.as_param(), order.as_param())));
    }
    if let Some(cursor) = &query.cursor {
        params.push(("cursor_string", cursor.clone()));
    }
    params
}

/// Percent-encode the query parameters into a `k=v&k=v` string.
fn encode_query_string(params: &[(&'static str, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Client for the osu! API v2 `beatmapsets/search` endpoint.
#[derive(Debug, Clone)]
pub struct SearchClient {
    client: reqwest::Client,
}

impl SearchClient {
    /// New client backed by the library's default reqwest client.
    ///
    /// # Panics
    ///
    /// Panics if the underlying reqwest client builder fails — which only
    /// happens if the system's TLS backend cannot initialise.
    pub fn new() -> Self {
        Self {
            client: http::create_api_client().expect("failed to build default reqwest client"),
        }
    }

    /// Run a search. `token` is a caller-supplied bearer (guest or user); this
    /// method attaches it plus the mandatory `x-api-version` header and never
    /// touches [`crate::auth`]. Performs a single request — retry on the caller
    /// side if needed.
    pub async fn search(&self, token: &str, query: &SearchQuery) -> Result<SearchResults> {
        let query_string = encode_query_string(&build_query_params(query));
        let url = format!("{OSU_API_BASE}/beatmapsets/search?{query_string}");
        let response = self
            .client
            .get(&url)
            .bearer_auth(token)
            .header("x-api-version", X_API_VERSION)
            .send()
            .await?;

        let status = response.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs);
            return Err(Error::RateLimited { retry_after });
        }

        if !status.is_success() {
            return Err(Error::HttpStatus(status.as_u16()));
        }

        let bytes = response.bytes().await?;
        serde_json::from_slice(&bytes).map_err(Into::into)
    }
}

impl Default for SearchClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../tests/search.rs"]
mod tests;
