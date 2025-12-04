use super::constants::CONCURRENT_REQUESTS;
use futures_util::{StreamExt, stream};
use reqwest::Client;
use serde::Deserialize;
use std::{collections::HashSet, time::Duration};
use tracing::{debug, info, trace, warn};

const NEKOHA_API_BASE: &str = "https://mirror.nekoha.moe/api4";

/// Mirror URLs for availability checking (using HEAD requests)
const MIRROR_CHECK_URLS: &[&str] = &[
    "https://catboy.best/d/{id}",
    "https://api.nerinyan.moe/d/{id}",
    "https://osu.direct/api/d/{id}",
    "https://dl.sayobot.cn/beatmaps/download/full/{id}",
    "https://mirror.nekoha.moe/api4/download/{id}",
];

fn deserialize_string_to_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
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

#[derive(Debug, Deserialize)]
struct BeatmapsetResponse {
    #[serde(default, deserialize_with = "deserialize_string_to_u64")]
    file_size: Option<u64>,
}

pub struct SizeFetchResult {
    pub total_bytes: u64,
    pub missing_count: u32,
}

/// Results from checking beatmapset availability on mirrors
pub struct MirrorAvailabilityResult {
    /// IDs of beatmapsets that are available on at least one mirror
    pub available: HashSet<u32>,
    /// IDs of beatmapsets that are unavailable on all mirrors
    pub unavailable: HashSet<u32>,
}

/// Check beatmapsets availability across mirrors.
/// Uses ZIP magic byte verification to ensure actual archives are available.
pub async fn check_mirror_availability(
    client: &Client,
    beatmapset_ids: &[u32],
) -> MirrorAvailabilityResult {
    let results: Vec<(u32, bool)> = stream::iter(beatmapset_ids.iter().copied())
        .map(|id| {
            let client = client.clone();
            async move {
                let available = check_availability_on_any_mirror(&client, id).await;
                (id, available)
            }
        })
        .buffer_unordered(CONCURRENT_REQUESTS)
        .collect()
        .await;

    let mut available = HashSet::new();
    let mut unavailable = HashSet::new();

    for (id, is_available) in results {
        if is_available {
            available.insert(id);
        } else {
            unavailable.insert(id);
        }
    }

    info!(
        total = beatmapset_ids.len(),
        available = available.len(),
        unavailable = unavailable.len(),
        "Checked beatmapset availability on mirrors"
    );

    MirrorAvailabilityResult {
        available,
        unavailable,
    }
}

pub async fn fetch_beatmapset_sizes(client: &Client, beatmapset_ids: &[u32]) -> SizeFetchResult {
    let results: Vec<(u32, Option<u64>)> = stream::iter(beatmapset_ids.iter().copied())
        .map(|id| {
            let client = client.clone();
            async move {
                let size = fetch_single_size(&client, id).await;
                (id, size)
            }
        })
        .buffer_unordered(CONCURRENT_REQUESTS)
        .collect()
        .await;

    let mut total_bytes: u64 = 0;
    let mut fetched_count: usize = 0;
    let mut missing_count: u32 = 0;

    for (_id, size_opt) in results {
        match size_opt {
            Some(size) => {
                total_bytes = total_bytes.saturating_add(size);
                fetched_count += 1;
            }
            None => {
                missing_count += 1;
            }
        }
    }

    debug!(
        total_bytes,
        fetched = fetched_count,
        missing = missing_count,
        "Fetched beatmapset sizes from nekoha"
    );

    SizeFetchResult {
        total_bytes,
        missing_count,
    }
}

async fn fetch_single_size(client: &Client, beatmapset_id: u32) -> Option<u64> {
    let url = format!("{}/beatmapset/{}", NEKOHA_API_BASE, beatmapset_id);

    let response = match client.get(&url).send().await {
        Ok(resp) => resp,
        Err(err) => {
            warn!(beatmapset_id, error = %err, "Failed to fetch beatmapset size");
            return None;
        }
    };

    if !response.status().is_success() {
        return None;
    }

    match response.json::<BeatmapsetResponse>().await {
        Ok(data) => data.file_size,
        Err(err) => {
            warn!(beatmapset_id, error = %err, "Failed to parse beatmapset response");
            None
        }
    }
}

/// Check if a beatmapset is available on any mirror by verifying ZIP magic bytes.
/// HEAD requests alone are unreliable as some mirrors return 200 for error pages.
async fn check_availability_on_any_mirror(client: &Client, beatmapset_id: u32) -> bool {
    for template in MIRROR_CHECK_URLS {
        let url = template.replace("{id}", &beatmapset_id.to_string());

        // Use a range request to fetch only the first 4 bytes (ZIP magic)
        let result = client
            .get(&url)
            .header("Range", "bytes=0-3")
            .timeout(Duration::from_secs(10))
            .send()
            .await;

        match result {
            Ok(resp) => {
                let status = resp.status();
                // Accept 2xx responses (includes 200 and 206 Partial Content)
                if status.is_success() {
                    // Check content-type first
                    if let Some(content_type) = resp.headers().get("content-type")
                        && let Ok(ct) = content_type.to_str()
                    {
                        let ct_lower = ct.to_ascii_lowercase();
                        // Reject if content-type indicates HTML or JSON (error pages)
                        if ct_lower.contains("text/html") || ct_lower.contains("application/json") {
                            trace!(beatmapset_id, mirror = %url, content_type = %ct, "Rejected: error page content type");
                            continue;
                        }
                    }

                    // Verify ZIP magic bytes
                    if let Ok(bytes) = resp.bytes().await {
                        if bytes.len() >= 4 && bytes[0..4] == [0x50, 0x4B, 0x03, 0x04] {
                            trace!(beatmapset_id, mirror = %url, "Verified available with ZIP magic");
                            return true;
                        } else {
                            trace!(beatmapset_id, mirror = %url, "Rejected: invalid ZIP magic bytes");
                        }
                    }
                } else if status.is_redirection() {
                    // For redirects, we can't easily verify - skip this mirror
                    trace!(beatmapset_id, mirror = %url, "Skipped: redirect without verification");
                }
            }
            Err(_) => continue,
        }
    }
    false
}
