//! Beatmapset size and availability checks via the Nekoha mirror.
//!
//! Behind the `size-fetch` feature.

use crate::{Error, Mirror, Result, http};
use futures_util::{StreamExt, TryStreamExt, stream, stream::FuturesUnordered};
use reqwest::Client;
use serde::Deserialize;
use std::{collections::HashSet, time::Duration};
use tracing::{debug, trace, warn};

const NEKOHA_API_BASE: &str = "https://mirror.nekoha.moe/api";
const DEFAULT_CONCURRENT_REQUESTS: usize = 50;
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_REDIRECT_LIMIT: usize = 3;
const PROBE_TRANSIENT_RETRIES: usize = 2;
const ZIP_MAGIC: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];

/// Aggregated result for a batch size fetch.
#[derive(Debug, Clone)]
pub struct SizeFetchResult {
    /// Estimated total bytes (known bytes + averaged estimate for unknowns).
    pub total_bytes: u64,
    /// Number of beatmapsets the mirror had no size record for.
    pub missing_count: u32,
}

/// Aggregated result for a multi-mirror availability check.
#[derive(Debug, Clone)]
pub struct AvailabilityResult {
    /// Beatmapsets available on at least one mirror.
    pub available: HashSet<u32>,
    /// Beatmapsets unavailable on every mirror.
    pub unavailable: HashSet<u32>,
}

/// Builder-free fetcher; pass in a shared client.
pub struct SizeFetcher {
    client: Client,
    concurrency: usize,
}

impl Default for SizeFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl SizeFetcher {
    /// New fetcher backed by the library's default API client.
    ///
    /// # Panics
    ///
    /// Panics if the underlying reqwest client builder fails — which only
    /// happens if the system's TLS backend cannot initialise.
    pub fn new() -> Self {
        Self {
            client: http::create_api_client().expect("failed to build default reqwest client"),
            concurrency: DEFAULT_CONCURRENT_REQUESTS,
        }
    }

    /// Override the maximum number of concurrent requests.
    #[must_use]
    pub fn concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency.max(1);
        self
    }

    /// Fetch one beatmapset's archive size from the Nekoha mirror.
    ///
    /// Separates the two failures [`fetch_sizes`](Self::fetch_sizes) folds
    /// together, so a caller can cache a real answer and retry a transient one:
    ///
    /// - `Ok(Some(bytes))` — the mirror has a size record.
    /// - `Ok(None)` — the mirror answered, and this set has no size: either it
    ///   knows no such set (HTTP 404) or its record carries a null `file_size`.
    ///   A stable answer; re-asking gets the same one.
    /// - `Err(_)` — the mirror could not be reached or did not answer usefully
    ///   (network, timeout, 5xx, unparseable body). Says nothing about the set.
    ///
    /// Performs a single request — retry on the caller side if needed.
    ///
    /// # Errors
    ///
    /// [`Error::Network`] / [`Error::Timeout`] on transport failure,
    /// [`Error::HttpStatus`] on a non-404 unsuccessful status, [`Error::Parse`]
    /// on a body that is not the expected JSON.
    pub async fn fetch_size(&self, beatmapset_id: u32) -> Result<Option<u64>> {
        fetch_single_size(&self.client, beatmapset_id).await
    }

    /// Fetch sizes for `beatmapset_ids` from the Nekoha mirror.
    ///
    /// Aggregate-only: a set the mirror has no record for and a set whose probe
    /// failed both land in `missing_count`. Use [`fetch_size`](Self::fetch_size)
    /// when that difference matters.
    pub async fn fetch_sizes(&self, beatmapset_ids: &[u32]) -> SizeFetchResult {
        let results: Vec<Option<u64>> = stream::iter(beatmapset_ids.iter().copied())
            .map(|id| {
                let client = self.client.clone();
                // A failed probe counts as missing here — same aggregate estimate
                // this returned before the per-id path split the two apart.
                async move { fetch_single_size(&client, id).await.ok().flatten() }
            })
            .buffer_unordered(self.concurrency)
            .collect()
            .await;

        estimate_from_probes(&results)
    }

    /// Probe each beatmapset against every supplied mirror.
    ///
    /// `report_progress(checked, total)` fires after each completed beatmapset.
    pub async fn check_availability(
        &self,
        beatmapset_ids: &[u32],
        mirrors: &[Mirror],
        mut report_progress: impl FnMut(usize, usize),
    ) -> AvailabilityResult {
        let total = beatmapset_ids.len();
        let templates: Vec<&str> = mirrors.iter().map(Mirror::template).collect();
        let mut available = HashSet::new();
        let mut unavailable = HashSet::new();
        let mut checked = 0usize;

        let mut results = stream::iter(beatmapset_ids.iter().copied())
            .map(|id| {
                let client = self.client.clone();
                let templates = templates.clone();
                async move { (id, check_on_any_mirror(&client, id, &templates).await) }
            })
            .buffer_unordered(self.concurrency);

        while let Some((id, ok)) = results.next().await {
            if ok {
                available.insert(id);
            } else {
                unavailable.insert(id);
            }
            checked += 1;
            report_progress(checked, total);
        }

        AvailabilityResult {
            available,
            unavailable,
        }
    }
}

#[derive(Deserialize)]
struct BeatmapsetResponse {
    #[serde(default, deserialize_with = "deserialize_string_or_number")]
    file_size: Option<u64>,
}

fn deserialize_string_or_number<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        String(String),
        Number(u64),
        Null,
    }

    match StringOrNumber::deserialize(deserializer)? {
        StringOrNumber::String(s) => s
            .parse::<u64>()
            .map(Some)
            .map_err(|_| D::Error::custom("invalid u64 string")),
        StringOrNumber::Number(n) => Ok(Some(n)),
        StringOrNumber::Null => Ok(None),
    }
}

/// Total the landed probes and bill each unknown at the average of what landed,
/// so a partial sample still yields a whole-batch estimate. With nothing known,
/// there is no average to bill at and the total is just the known bytes (0).
fn estimate_from_probes(results: &[Option<u64>]) -> SizeFetchResult {
    let mut known_bytes: u64 = 0;
    let mut fetched_count: usize = 0;
    let mut missing_count: u32 = 0;

    for size_opt in results {
        match size_opt {
            Some(size) => {
                known_bytes = known_bytes.saturating_add(*size);
                fetched_count += 1;
            }
            None => missing_count += 1,
        }
    }

    let total_bytes = if missing_count > 0 && fetched_count > 0 {
        let average = known_bytes / fetched_count as u64;
        known_bytes.saturating_add(average.saturating_mul(u64::from(missing_count)))
    } else {
        known_bytes
    };

    debug!(
        total_bytes,
        known_bytes,
        fetched = fetched_count,
        missing = missing_count,
        "fetched beatmapset sizes from nekoha"
    );

    SizeFetchResult {
        total_bytes,
        missing_count,
    }
}

/// One size lookup. `Ok(None)` is the mirror's own answer that the set has no
/// size — a 404 (no such set) or a null `file_size`; every other failure is an
/// `Err`, so a caller can tell a stable answer from an unreachable mirror.
async fn fetch_single_size(client: &Client, beatmapset_id: u32) -> Result<Option<u64>> {
    let url = format!("{NEKOHA_API_BASE}/beatmapset/{beatmapset_id}");

    let response = client.get(&url).send().await.inspect_err(|err| {
        warn!(beatmapset_id, error = %err, "failed to fetch beatmapset size");
    })?;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        trace!(beatmapset_id, "mirror has no record for beatmapset");
        return Ok(None);
    }
    if !status.is_success() {
        warn!(beatmapset_id, %status, "beatmapset size lookup failed");
        return Err(Error::HttpStatus(status.as_u16()));
    }

    let bytes = response.bytes().await?;
    let data: BeatmapsetResponse = serde_json::from_slice(&bytes).inspect_err(|err| {
        warn!(beatmapset_id, error = %err, "failed to parse beatmapset response");
    })?;
    Ok(data.file_size)
}

async fn check_on_any_mirror(client: &Client, beatmapset_id: u32, templates: &[&str]) -> bool {
    let mut probes: FuturesUnordered<_> = templates
        .iter()
        .map(|template| probe_template(client, beatmapset_id, template))
        .collect();

    while let Some(available) = probes.next().await {
        if available {
            return true;
        }
    }
    false
}

async fn probe_template(client: &Client, beatmapset_id: u32, template: &str) -> bool {
    let mut url = template.replace("{id}", &beatmapset_id.to_string());
    let mut redirects_remaining = PROBE_REDIRECT_LIMIT;
    let mut retries_remaining = PROBE_TRANSIENT_RETRIES;

    loop {
        match probe_once(client, beatmapset_id, &url).await {
            ProbeOutcome::Available => return true,
            ProbeOutcome::Rejected => return false,
            ProbeOutcome::RetryRedirect(next) => {
                if redirects_remaining == 0 {
                    return false;
                }
                redirects_remaining -= 1;
                url = next;
            }
            ProbeOutcome::RetryTransient => {
                if retries_remaining == 0 {
                    return false;
                }
                retries_remaining -= 1;
            }
        }
    }
}

enum ProbeOutcome {
    Available,
    Rejected,
    RetryRedirect(String),
    RetryTransient,
}

async fn probe_once(client: &Client, beatmapset_id: u32, url: &str) -> ProbeOutcome {
    let result = client
        .get(url)
        .header("Range", "bytes=0-3")
        .header("Connection", "close")
        .timeout(PROBE_TIMEOUT)
        .send()
        .await;

    let resp = match result {
        Ok(resp) => resp,
        Err(err) => {
            if err.is_timeout() || err.is_connect() || err.is_request() {
                trace!(beatmapset_id, mirror = %url, error = %err, "transient probe error");
                return ProbeOutcome::RetryTransient;
            }
            return ProbeOutcome::Rejected;
        }
    };

    let status = resp.status();
    if status.is_redirection() {
        if let Some(location) = resp.headers().get(reqwest::header::LOCATION)
            && let Ok(location_str) = location.to_str()
        {
            let next = resp
                .url()
                .join(location_str)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| location_str.to_string());
            return ProbeOutcome::RetryRedirect(next);
        }
        return ProbeOutcome::RetryTransient;
    }

    if status.is_server_error() {
        return ProbeOutcome::RetryTransient;
    }

    if !status.is_success() {
        return ProbeOutcome::Rejected;
    }

    if let Some(content_type) = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
    {
        let ct = content_type.to_ascii_lowercase();
        if ct.contains("text/html") || ct.contains("application/json") {
            return ProbeOutcome::Rejected;
        }
    }

    match read_probe_prefix(resp).await {
        Ok(bytes) if bytes == ZIP_MAGIC => ProbeOutcome::Available,
        Ok(_) => ProbeOutcome::Rejected,
        Err(_) => ProbeOutcome::RetryTransient,
    }
}

async fn read_probe_prefix(resp: reqwest::Response) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(ZIP_MAGIC.len());
    let mut stream = resp.bytes_stream();

    while bytes.len() < ZIP_MAGIC.len() {
        let Some(chunk) = stream.try_next().await? else {
            break;
        };
        let remaining = ZIP_MAGIC.len() - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    Ok(bytes)
}

#[cfg(test)]
#[path = "../tests/size.rs"]
mod tests;
