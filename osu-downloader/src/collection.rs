//! osucollector.com client.
//!
//! Behind the `collection` feature.

use crate::{Error, Result, http};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

const COLLECTOR_API_BASE: &str = "https://osucollector.com/api/collections";

/// Collection metadata from osucollector.com.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Collection {
    /// Collection ID.
    pub id: u32,
    /// Collection name.
    pub name: String,
    /// Collection description.
    #[serde(default)]
    pub description: Option<String>,
    /// Uploader information.
    pub uploader: Uploader,
    /// Beatmapsets in this collection.
    pub beatmapsets: Vec<Beatmapset>,
    /// Number of favourites.
    #[serde(default)]
    pub favourites: u32,
}

/// Uploader information.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Uploader {
    /// Uploader user ID.
    pub id: u32,
    /// Uploader username.
    pub username: String,
}

/// Beatmapset in a collection.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Beatmapset {
    /// Beatmapset ID.
    pub id: u32,
    /// Individual beatmaps in this set.
    #[serde(default)]
    pub beatmaps: Vec<Beatmap>,
}

/// Individual beatmap.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Beatmap {
    /// Beatmap ID.
    pub id: u32,
    /// MD5 checksum.
    pub checksum: String,
}

impl Collection {
    /// Beatmapset IDs in this collection (preserving order, deduplicated).
    pub fn beatmapset_ids(&self) -> Vec<u32> {
        let mut seen = std::collections::HashSet::with_capacity(self.beatmapsets.len());
        self.beatmapsets
            .iter()
            .map(|b| b.id)
            .filter(|id| seen.insert(*id))
            .collect()
    }

    /// Total number of beatmaps (not beatmapsets).
    pub fn beatmap_count(&self) -> usize {
        self.beatmapsets.iter().map(|b| b.beatmaps.len()).sum()
    }

    /// Default folder name for this collection: `<sanitized-name>-<id>`.
    pub fn folder_name(&self) -> String {
        format!("{}-{}", sanitize_collection_name(&self.name), self.id)
    }
}

fn sanitize_collection_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        out.push(match c {
            '/' | '\\' | '\0' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        });
    }

    let trimmed = out.trim();
    if trimmed.is_empty() {
        return "collection".to_string();
    }

    // Windows resolves a path component to a DOS device when the component's
    // stem (text before the first dot, trailing spaces ignored, case-insensitive)
    // matches a reserved name such as CON, PRN, AUX, NUL, COM1-9 or LPT1-9,
    // extension included: `CON.txt` is the console. Prefixing `_` breaks that
    // resolution on Windows and is inert everywhere else.
    let stem = trimmed
        .split_once('.')
        .map_or(trimmed, |(stem, _)| stem)
        .trim_end();
    if is_reserved_device_name(stem) {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    }
}

fn is_reserved_device_name(stem: &str) -> bool {
    matches!(
        stem.to_ascii_lowercase().as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

/// Client for fetching collections from osucollector.com.
#[derive(Debug, Clone)]
pub struct CollectionClient {
    client: reqwest::Client,
}

impl CollectionClient {
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

    /// Fetch a collection by ID. Performs a single request — wrap in a retry
    /// loop on the caller side if you need to retry, or use [`fetch_retrying`](Self::fetch_retrying).
    pub async fn fetch(&self, collection_id: u32) -> Result<Collection> {
        let url = format!("{COLLECTOR_API_BASE}/{collection_id}");
        let response = self.client.get(&url).send().await?;

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

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::NotFound);
        }

        if !status.is_success() {
            return Err(Error::HttpStatus(status.as_u16()));
        }

        let bytes = response.bytes().await?;
        serde_json::from_slice(&bytes).map_err(Into::into)
    }

    /// Fetch with the library's built-in retry policy:
    /// - on [`Error::RateLimited`]: sleep for `retry_after` (capped at 60s, defaults to 30s)
    /// - on transient [`Error::Network`] / [`Error::Timeout`]: exponential backoff (2^attempt seconds)
    /// - on any other error: return immediately
    ///
    /// `attempts` is the maximum number of tries (so `attempts = 3` means up to 2 retries).
    pub async fn fetch_retrying(&self, collection_id: u32, attempts: u8) -> Result<Collection> {
        let attempts = attempts.max(1);
        let mut last_error: Option<Error> = None;

        for attempt in 1..=attempts {
            match self.fetch(collection_id).await {
                Ok(collection) => return Ok(collection),
                Err(Error::RateLimited { retry_after }) => {
                    let delay = retry_after
                        .unwrap_or(Duration::from_secs(30))
                        .min(Duration::from_secs(60));
                    warn!(
                        attempt,
                        delay_secs = delay.as_secs(),
                        "rate limited by osucollector.com; waiting before retry"
                    );
                    sleep(delay).await;
                    last_error = Some(Error::RateLimited { retry_after });
                }
                Err(err) if err.is_transient() && attempt < attempts => {
                    let delay_secs = 2_u64.pow((attempt - 1) as u32);
                    warn!(
                        attempt,
                        remaining = attempts - attempt,
                        delay_secs,
                        error = %err,
                        "fetch collection attempt failed; retrying"
                    );
                    sleep(Duration::from_secs(delay_secs)).await;
                    last_error = Some(err);
                }
                Err(err) => return Err(err),
            }
        }

        Err(last_error.unwrap_or_else(|| Error::network("all retry attempts failed")))
    }
}

impl Default for CollectionClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../tests/collection.rs"]
mod tests;
