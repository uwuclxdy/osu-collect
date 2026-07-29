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
    /// Approved. Expressed through `q` as `status=approved` with the category
    /// param forced to `any`; the `s` param alone can't pin it (probed
    /// 2026-07-11, unlike every other status here).
    Approved,
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
            // `Approved` rides the `any` category and is pinned by a
            // `status=approved` term in `q` (see [`build_q`]).
            Self::Approved => "any",
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

/// One bound of a [`QueryRange`], carrying whether it is inclusive. Inclusive
/// emits `>=`/`<=`; strict emits `>`/`<` (osu q-DSL supports both).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeBound<T> {
    /// The bound value.
    pub value: T,
    /// `true` → inclusive (`>=`/`<=`); `false` → strict (`>`/`<`).
    pub inclusive: bool,
}

impl<T> RangeBound<T> {
    /// An inclusive bound (`>=`/`<=`).
    pub fn inclusive(value: T) -> Self {
        Self {
            value,
            inclusive: true,
        }
    }

    /// A strict bound (`>`/`<`).
    pub fn strict(value: T) -> Self {
        Self {
            value,
            inclusive: false,
        }
    }
}

/// One filter criterion serialized into the `q` string for a single key.
///
/// [`Exact`](Self::Exact) emits `key=value` — for numeric keys the server widens
/// that to its tolerance band (e.g. `ar=9` ≈ `8.95..9.05`). [`Range`](Self::Range)
/// emits a `>`/`>=` and/or `<`/`<=` term per set bound (strict vs inclusive per
/// [`RangeBound::inclusive`]); an all-`None` range emits nothing. Reused for
/// float (stars/ar/…), integer (length/keys/…), and date-string (`ranked`) keys.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryRange<T> {
    /// Single value: emits `key=value`.
    Exact(T),
    /// Bounded range: emits a lower and/or upper comparison term.
    Range {
        /// Lower bound (`key>min` / `key>=min`), if set.
        min: Option<RangeBound<T>>,
        /// Upper bound (`key<max` / `key<=max`), if set.
        max: Option<RangeBound<T>>,
    },
}

impl<T> QueryRange<T> {
    /// A `None`-collapsing inclusive pair: `None` when both bounds are absent, so
    /// "no criterion" has exactly one representation (the outer `Option` on the
    /// [`SearchQuery`] field). Both bounds inclusive — used for the `ranked` key,
    /// whose `..` date grammar has no strict form.
    pub fn from_bounds(min: Option<T>, max: Option<T>) -> Option<Self> {
        if min.is_none() && max.is_none() {
            None
        } else {
            Some(Self::Range {
                min: min.map(RangeBound::inclusive),
                max: max.map(RangeBound::inclusive),
            })
        }
    }

    /// Inclusive lower bound (`key>=value`).
    pub fn at_least(value: T) -> Self {
        Self::Range {
            min: Some(RangeBound::inclusive(value)),
            max: None,
        }
    }

    /// Strict lower bound (`key>value`).
    pub fn greater_than(value: T) -> Self {
        Self::Range {
            min: Some(RangeBound::strict(value)),
            max: None,
        }
    }

    /// Inclusive upper bound (`key<=value`).
    pub fn at_most(value: T) -> Self {
        Self::Range {
            min: None,
            max: Some(RangeBound::inclusive(value)),
        }
    }

    /// Strict upper bound (`key<value`).
    pub fn less_than(value: T) -> Self {
        Self::Range {
            min: None,
            max: Some(RangeBound::strict(value)),
        }
    }

    /// Inclusive two-sided range (`key>=min key<=max`).
    pub fn between(min: T, max: T) -> Self {
        Self::Range {
            min: Some(RangeBound::inclusive(min)),
            max: Some(RangeBound::inclusive(max)),
        }
    }
}

impl<T: std::fmt::Display> QueryRange<T> {
    /// Append this criterion's q-DSL term(s) for `key` onto `out`.
    fn emit_terms(&self, key: &str, out: &mut Vec<String>) {
        match self {
            Self::Exact(value) => out.push(format!("{key}={value}")),
            Self::Range { min, max } => {
                if let Some(min) = min {
                    let op = if min.inclusive { ">=" } else { ">" };
                    out.push(format!("{key}{op}{}", min.value));
                }
                if let Some(max) = max {
                    let op = if max.inclusive { "<=" } else { "<" };
                    out.push(format!("{key}{op}{}", max.value));
                }
            }
        }
    }
}

/// An exact-text term `key="value"`, inner `"` escaped as `\"` (the web parser's
/// `makeTextOption` un-escapes it back). Text keys accept the `=` operator only.
fn text_term(key: &str, value: &str) -> String {
    format!("{key}=\"{}\"", value.replace('"', "\\\""))
}

/// A search query. Serialized to url parameters by [`SearchClient::search`];
/// [`cursor`](Self::cursor) threads cursor-based pagination.
///
/// The typed criteria below are folded into the `q` param in a fixed, documented
/// order (see [`build_q`]) so the emitted string is byte-stable — tests and the
/// app's staleness snapshot pin it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchQuery {
    /// Free text (`q`). Empty is allowed; the server applies its default sort.
    /// Emitted first in the `q` string, ahead of the typed criteria.
    pub text: String,
    /// Game mode (`m`). `None` searches every mode.
    pub mode: Option<SearchMode>,
    /// Category / rank status (`s`). `None` uses the server default (has-leaderboard).
    /// [`SearchStatus::Approved`] additionally emits a `status=approved` term in `q`.
    pub status: Option<SearchStatus>,
    /// Sort field + order. `None` uses the server default (`relevance` with a
    /// query, else `ranked`; `desc`).
    pub sort: Option<(SortField, SortOrder)>,
    /// Pagination cursor (`cursor_string`). `None` requests the first page;
    /// resend the previous response's [`SearchResults::cursor_string`] for the next.
    pub cursor: Option<String>,
    /// Star-rating criterion (`stars`).
    pub stars: Option<QueryRange<f64>>,
    /// Approach-rate criterion (`ar`).
    pub ar: Option<QueryRange<f64>>,
    /// Circle-size criterion (`cs`).
    pub cs: Option<QueryRange<f64>>,
    /// Overall-difficulty criterion (`od`).
    pub od: Option<QueryRange<f64>>,
    /// HP-drain criterion — emitted under the canonical key `dr`.
    pub hp: Option<QueryRange<f64>>,
    /// BPM criterion (`bpm`).
    pub bpm: Option<QueryRange<f64>>,
    /// Length criterion (`length`), in raw seconds.
    pub length: Option<QueryRange<u32>>,
    /// Mania key-count criterion (`keys`).
    pub keys: Option<QueryRange<u32>>,
    /// Ranked-date criterion (`ranked`); values are `yyyy`, `yyyy-mm`, or
    /// `yyyy-mm-dd`.
    pub ranked: Option<QueryRange<String>>,
    /// Favourite-count criterion (`favourites`).
    pub favourites: Option<QueryRange<u32>>,
    /// Exact artist (`artist="…"`).
    pub artist: Option<String>,
    /// Exact mapper (`creator="…"`).
    pub creator: Option<String>,
    /// Exact title (`title="…"`).
    pub title: Option<String>,
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
    /// Per-difficulty beatmaps from the search response's nested `beatmaps[]`
    /// array. Empty when the carrier response omits it.
    #[serde(default)]
    pub beatmaps: Vec<Beatmap>,
}

/// One difficulty (beatmap) nested under a search result's `beatmaps[]` array.
/// The wire keys mirror the osu! API v2 `beatmaps` element shape; note in
/// particular that HP drain is carried under `drain` and overall difficulty
/// under `accuracy`. (The q-DSL *query* parameter for HP is the separate key
/// `dr` — see [`build_q`]; these are two different names for the same
/// attribute across the request/response split.)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Beatmap {
    /// Beatmap (difficulty) id.
    #[serde(default)]
    pub id: u32,
    /// Parent beatmapset id.
    #[serde(default)]
    pub beatmapset_id: u32,
    /// Game mode as an integer (`0` osu … `3` mania).
    #[serde(default)]
    pub mode_int: u32,
    /// Difficulty name string (`"Expert"`, `"Insane"`, …).
    #[serde(default)]
    pub version: String,
    /// Star rating.
    #[serde(default)]
    pub difficulty_rating: f64,
    /// BPM.
    #[serde(default)]
    pub bpm: f64,
    /// Approach rate.
    #[serde(default)]
    pub ar: f64,
    /// Circle size.
    #[serde(default)]
    pub cs: f64,
    /// HP drain — the osu API v2 wire key is `drain`.
    #[serde(default, rename = "drain")]
    pub hp: f64,
    /// Overall difficulty — the osu API v2 wire key is `accuracy`.
    #[serde(default, rename = "accuracy")]
    pub od: f64,
    /// Total length in seconds.
    #[serde(default)]
    pub total_length: u32,
    /// Drain time in seconds.
    #[serde(default)]
    pub hit_length: u32,
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

/// Assemble the `q` parameter: free text first, then the typed criteria in a
/// fixed order so the output is byte-stable. `SearchStatus::Approved` prepends a
/// `status=approved` term ahead of the numeric/text criteria (right after free
/// text) — the `s` category param alone can't pin `approved` (probed 2026-07-11).
fn build_q(query: &SearchQuery) -> String {
    let mut terms: Vec<String> = Vec::new();
    if !query.text.is_empty() {
        terms.push(query.text.clone());
    }
    if query.status == Some(SearchStatus::Approved) {
        terms.push("status=approved".to_string());
    }
    if let Some(range) = &query.stars {
        range.emit_terms("stars", &mut terms);
    }
    if let Some(range) = &query.ar {
        range.emit_terms("ar", &mut terms);
    }
    if let Some(range) = &query.cs {
        range.emit_terms("cs", &mut terms);
    }
    if let Some(range) = &query.od {
        range.emit_terms("od", &mut terms);
    }
    if let Some(range) = &query.hp {
        range.emit_terms("dr", &mut terms);
    }
    if let Some(range) = &query.bpm {
        range.emit_terms("bpm", &mut terms);
    }
    if let Some(range) = &query.length {
        range.emit_terms("length", &mut terms);
    }
    if let Some(range) = &query.keys {
        range.emit_terms("keys", &mut terms);
    }
    if let Some(range) = &query.ranked {
        range.emit_terms("ranked", &mut terms);
    }
    if let Some(range) = &query.favourites {
        range.emit_terms("favourites", &mut terms);
    }
    if let Some(value) = &query.creator {
        terms.push(text_term("creator", value));
    }
    if let Some(value) = &query.artist {
        terms.push(text_term("artist", value));
    }
    if let Some(value) = &query.title {
        terms.push(text_term("title", value));
    }
    terms.join(" ")
}

/// Maximum ids accepted by a single [`SearchClient::beatmaps`] call. The osu!
/// endpoint caps a batch at this many; page larger id lists caller-side.
pub const MAX_BATCH_IDS: usize = 50;

/// One row from [`SearchClient::beatmaps`]: a single beatmap (difficulty) plus
/// its parent set's metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct BeatmapRow {
    /// Beatmap (difficulty) id.
    pub id: u32,
    /// Parent beatmapset id — the id the download pipeline consumes.
    pub beatmapset_id: u32,
    /// Game mode as an integer (`0` osu … `3` mania); defensively optional.
    #[serde(default)]
    pub mode_int: Option<u8>,
    /// The parent set's metadata (title, artist, creator, status, counts, flags).
    pub beatmapset: BeatmapSetMeta,
}

/// Envelope for `GET /api/v2/beatmaps` — a single `beatmaps` array.
#[derive(Debug, Clone, Deserialize)]
struct BeatmapsResponse {
    #[serde(default)]
    beatmaps: Vec<BeatmapRow>,
}

/// Serialize a query to `(key, value)` url parameters. `q` is always sent (empty
/// allowed); the rest are omitted when unset so the server applies its defaults.
fn build_query_params(query: &SearchQuery) -> Vec<(&'static str, String)> {
    let mut params: Vec<(&'static str, String)> = vec![("q", build_q(query))];
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
        self.send_get(self.client.get(&url), token).await
    }

    /// Look up beatmaps by id via `GET /api/v2/beatmaps?ids[]=…`. At most
    /// [`MAX_BATCH_IDS`] ids per call — page larger id lists caller-side. `token`
    /// is a caller-supplied bearer (guest or user), attached with the mandatory
    /// `x-api-version` header like [`search`](Self::search).
    ///
    /// The server silently omits unknown, deleted, or restricted ids, so the
    /// result may be shorter than `ids` and in any order: callers must tolerate
    /// holes and key rows by [`BeatmapRow::beatmapset_id`], never by position.
    /// An empty `ids` slice short-circuits to an empty vec without a request.
    pub async fn beatmaps(&self, token: &str, ids: &[u32]) -> Result<Vec<BeatmapRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        if ids.len() > MAX_BATCH_IDS {
            return Err(Error::config(format!(
                "beatmaps() accepts at most {MAX_BATCH_IDS} ids per call, got {} — chunk caller-side",
                ids.len()
            )));
        }
        // `ids[]` repeated per id; the brackets are percent-encoded (`ids%5B%5D`)
        // so the raw URL parses cleanly — PHP decodes it back into an array.
        let query_string = ids
            .iter()
            .map(|id| format!("ids%5B%5D={id}"))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!("{OSU_API_BASE}/beatmaps?{query_string}");
        let response: BeatmapsResponse = self.send_get(self.client.get(&url), token).await?;
        Ok(response.beatmaps)
    }

    /// Attach the bearer + `x-api-version` header, send, map 429 / non-success
    /// statuses to the library error variants, and deserialize the JSON body.
    async fn send_get<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        token: &str,
    ) -> Result<T> {
        let response = request
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
