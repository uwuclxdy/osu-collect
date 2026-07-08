//! nzbasic batch-beatmap-downloader filter client.
//!
//! Behind the `filter` feature. Queries the hosted BBD instance at
//! `v2.nzbasic.com` — an indexed beatmap database filterable by per-diff
//! attributes (stars/ar/cs/od/hp/bpm/length), status, mode, text, and the
//! BBD-computed farm/stream/ranked-mapper flags that osu! API v2 cannot serve.
//!
//! The wire format is BBD's recursive node tree; this module keeps it private
//! and exposes only the flat AND query the app needs. Quirks encoded here
//! (verified live 2026-07-08): the server reads `rule.type` only as
//! group-vs-rule marker, `like` values are `%`-wrapped server-side, the
//! `Special` pseudo-field must be rewritten to the `Farm`/`Stream`/
//! `RankedMapper` flag columns, and `limit` caps diff rows before set-dedupe.
//!
//! The hosted instance is a solo-maintained free service — callers must treat
//! every error as survivable (fail soft) and never hard-depend on it.

use crate::{Error, Result, http};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

/// Base URL of the hosted BBD API.
const NZBASIC_BASE: &str = "https://v2.nzbasic.com";

/// Static client identifier sent with every filter request. BBD uses it for
/// anonymous metrics; no per-install id is generated.
const CLIENT_ID: &str = "osu-collect";

/// Game mode filter (the `Mode` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    /// osu!standard.
    Osu,
    /// osu!taiko.
    Taiko,
    /// osu!catch.
    Catch,
    /// osu!mania.
    Mania,
}

impl FilterMode {
    /// The `Mode` column value (BBD stores display labels, not ints).
    fn as_value(self) -> &'static str {
        match self {
            Self::Osu => "osu!",
            Self::Taiko => "Taiko",
            Self::Catch => "Catch the Beat",
            Self::Mania => "osu!mania",
        }
    }
}

/// Rank-status filter (the `Approved` column). `Leaderboard` and `Unranked`
/// are server-side macros expanding to multiple statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterStatus {
    /// Has a leaderboard (ranked + loved + approved) — server macro.
    Leaderboard,
    /// Ranked.
    Ranked,
    /// Loved.
    Loved,
    /// Approved.
    Approved,
    /// Pending.
    Pending,
    /// Work-in-progress.
    Wip,
    /// Graveyard.
    Graveyard,
    /// Not on a leaderboard (WIP + graveyard + pending) — server macro.
    Unranked,
}

impl FilterStatus {
    /// The `Approved` column value or server macro keyword.
    fn as_value(self) -> &'static str {
        match self {
            Self::Leaderboard => "HasLeaderboard",
            Self::Ranked => "ranked",
            Self::Loved => "loved",
            Self::Approved => "approved",
            Self::Pending => "pending",
            Self::Wip => "WIP",
            Self::Graveyard => "graveyard",
            Self::Unranked => "unranked",
        }
    }
}

/// BBD-computed special tag. These flags exist only in nzbasic's indexed
/// database; osu! API v2 has no equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterSpecial {
    /// Farm maps (high pp-per-effort, BBD heuristic).
    Farm,
    /// Stream maps (BBD heuristic).
    Stream,
    /// Mapped by a ranked mapper.
    RankedMapper,
}

impl FilterSpecial {
    /// The flag column the rule targets (value is always `"1"`).
    fn as_field(self) -> &'static str {
        match self {
            Self::Farm => "Farm",
            Self::Stream => "Stream",
            Self::RankedMapper => "RankedMapper",
        }
    }
}

/// Sort column (the request's `by` parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterSort {
    /// Star difficulty.
    Stars,
    /// Beats per minute.
    Bpm,
    /// Ranked/approved date.
    ApprovedDate,
    /// Last-updated date.
    LastUpdate,
    /// Play count.
    PlayCount,
    /// Favourite count.
    FavouriteCount,
    /// Total length in seconds.
    TotalLength,
    /// Archive size in bytes.
    Size,
}

impl FilterSort {
    /// The database column name.
    fn as_column(self) -> &'static str {
        match self {
            Self::Stars => "stars",
            Self::Bpm => "bpm",
            Self::ApprovedDate => "approvedDate",
            Self::LastUpdate => "lastUpdate",
            Self::PlayCount => "playCount",
            Self::FavouriteCount => "favouriteCount",
            Self::TotalLength => "totalLength",
            Self::Size => "size",
        }
    }
}

/// Sort direction (the request's `direction` parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterDirection {
    /// Ascending.
    Asc,
    /// Descending.
    Desc,
}

impl FilterDirection {
    /// The `direction` parameter value.
    fn as_param(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

/// An inclusive numeric bound pair. `None` on a side leaves that side open;
/// a fully-`None` range emits no rule.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FilterRange {
    /// Inclusive lower bound (`>=`).
    pub min: Option<f64>,
    /// Inclusive upper bound (`<=`).
    pub max: Option<f64>,
}

/// A flat AND filter query. Every set field becomes one rule; unset fields
/// (`None` / empty strings / empty ranges) are omitted so the server applies
/// no constraint. Serialized to BBD's node tree by [`FilterClient::fetch`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterQuery {
    /// Game mode. `None` matches every mode.
    pub mode: Option<FilterMode>,
    /// Rank status. `None` matches every status.
    pub status: Option<FilterStatus>,
    /// BBD special tag (farm / stream / ranked mapper).
    pub special: Option<FilterSpecial>,
    /// Star difficulty range.
    pub stars: FilterRange,
    /// Approach rate range.
    pub ar: FilterRange,
    /// Circle size range.
    pub cs: FilterRange,
    /// Overall difficulty range.
    pub od: FilterRange,
    /// HP drain range.
    pub hp: FilterRange,
    /// BPM range.
    pub bpm: FilterRange,
    /// Total length range, in seconds (the `TotalLength` column).
    pub length: FilterRange,
    /// Artist substring match. Empty = unset.
    pub artist: String,
    /// Mapper substring match. Empty = unset.
    pub creator: String,
    /// Title substring match. Empty = unset.
    pub title: String,
    /// Sort column + direction. `None` uses the server's natural order.
    pub sort: Option<(FilterSort, FilterDirection)>,
    /// Maximum number of DIFF rows returned (the server dedupes to sets
    /// after the limit, so the resulting set count is smaller).
    pub limit: Option<u32>,
}

/// A deserialized `/v2/filter` response. The server's metrics uuid (`Id`) is
/// deliberately not modelled.
#[derive(Debug, Clone, Deserialize)]
pub struct FilterResults {
    /// Matching diff-level beatmap ids — the input for [`FilterClient::details`].
    #[serde(rename = "Ids")]
    pub ids: Vec<u32>,
    /// Deduplicated beatmapset ids — what the download pipeline consumes.
    #[serde(rename = "SetIds")]
    pub set_ids: Vec<u32>,
    /// Archive size in bytes, keyed by set id.
    #[serde(rename = "SizeMap")]
    pub size_map: HashMap<u32, u64>,
    /// Per-diff MD5 checksums.
    #[serde(rename = "Hashes")]
    pub hashes: Vec<String>,
}

/// One `/beatmapDetails` row — per-diff metadata for rich previews. The
/// response carries more columns than modelled here; unknown fields are
/// ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct BeatmapDetails {
    /// Diff-level beatmap id.
    #[serde(rename = "Id")]
    pub id: u32,
    /// Owning beatmapset id.
    #[serde(rename = "SetId")]
    pub set_id: u32,
    /// Song title.
    #[serde(rename = "Title")]
    pub title: String,
    /// Artist.
    #[serde(rename = "Artist")]
    pub artist: String,
    /// Mapper username.
    #[serde(rename = "Creator")]
    pub creator: String,
    /// Difficulty name.
    #[serde(rename = "Version")]
    pub version: String,
    /// Star difficulty.
    #[serde(rename = "Stars")]
    pub stars: f64,
    /// Beats per minute.
    #[serde(rename = "Bpm")]
    pub bpm: f64,
    /// Approach rate.
    #[serde(rename = "Ar")]
    pub ar: f64,
    /// Circle size.
    #[serde(rename = "Cs")]
    pub cs: f64,
    /// Overall difficulty.
    #[serde(rename = "Od")]
    pub od: f64,
    /// HP drain.
    #[serde(rename = "Hp")]
    pub hp: f64,
    /// Rank status string (`"ranked"`, `"loved"`, ...).
    #[serde(rename = "Approved")]
    pub approved: String,
    /// Game mode string (lowercase in this endpoint, e.g. `"osu"`).
    #[serde(rename = "Mode")]
    pub mode: String,
    /// Total length in seconds.
    #[serde(rename = "TotalLength")]
    pub total_length: u32,
    /// Favourite count.
    #[serde(rename = "FavouriteCount", default)]
    pub favourite_count: u32,
    /// Play count.
    #[serde(rename = "PlayCount", default)]
    pub play_count: u32,
    /// Set archive size in bytes.
    #[serde(rename = "Size", default)]
    pub size: u64,
}

/// Serialize a query to BBD's request envelope: the node tree plus
/// `clientId` and the optional `limit`/`by`/`direction`.
fn build_request(query: &FilterQuery) -> Value {
    let mut body = json!({
        "node": build_node(query),
        "clientId": CLIENT_ID,
    });
    if let Some(limit) = query.limit {
        body["limit"] = limit.into();
    }
    if let Some((sort, direction)) = query.sort {
        body["by"] = sort.as_column().into();
        body["direction"] = direction.as_param().into();
    }
    body
}

/// Serialize the flat query to a single AND group of rules. Rule order is
/// fixed so the same query always yields byte-identical JSON (callers may
/// hash it).
fn build_node(query: &FilterQuery) -> Value {
    let mut rules: Vec<Value> = Vec::new();
    let mut push = |kind: &str, field: &str, operator: &str, value: String| {
        rules.push(json!({
            "id": (rules.len() + 1).to_string(),
            "rule": { "type": kind, "field": field, "operator": operator, "value": value },
        }));
    };

    if let Some(mode) = query.mode {
        push("Text", "Mode", "=", mode.as_value().to_string());
    }
    if let Some(status) = query.status {
        push("Text", "Approved", "=", status.as_value().to_string());
    }
    if let Some(special) = query.special {
        // No `Special` column server-side; the flag columns hold 0/1.
        push("Numeric", special.as_field(), "=", "1".to_string());
    }
    for (field, range) in [
        ("Stars", query.stars),
        ("Ar", query.ar),
        ("Cs", query.cs),
        ("Od", query.od),
        ("Hp", query.hp),
        ("Bpm", query.bpm),
        ("TotalLength", query.length),
    ] {
        if let Some(min) = range.min {
            push("Numeric", field, ">=", format!("{min}"));
        }
        if let Some(max) = range.max {
            push("Numeric", field, "<=", format!("{max}"));
        }
    }
    for (field, text) in [
        ("Artist", &query.artist),
        ("Creator", &query.creator),
        ("Title", &query.title),
    ] {
        if !text.is_empty() {
            // Raw value: the server wraps `like` operands in %...% itself.
            push("Text", field, "like", text.clone());
        }
    }

    json!({
        "id": "root",
        "group": { "connector": { "type": "AND", "not": [] }, "children": rules },
    })
}

/// Client for the hosted BBD filter API.
#[derive(Debug, Clone)]
pub struct FilterClient {
    client: reqwest::Client,
}

impl FilterClient {
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

    /// Run a filter query. Performs a single request — retry on the caller
    /// side if needed.
    pub async fn fetch(&self, query: &FilterQuery) -> Result<FilterResults> {
        self.post_json(&format!("{NZBASIC_BASE}/v2/filter"), &build_request(query))
            .await
    }

    /// Fetch per-diff metadata for a slice of diff ids (from
    /// [`FilterResults::ids`]). Page the ids caller-side; the endpoint takes
    /// the whole body as one query.
    pub async fn details(&self, diff_ids: &[u32]) -> Result<Vec<BeatmapDetails>> {
        self.post_json(&format!("{NZBASIC_BASE}/beatmapDetails"), &json!(diff_ids))
            .await
    }

    /// POST a JSON body and deserialize the JSON response, mapping 429 and
    /// non-success statuses to the library error variants.
    async fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        body: &Value,
    ) -> Result<T> {
        let response = self.client.post(url).json(body).send().await?;
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

impl Default for FilterClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../tests/filter.rs"]
mod tests;
