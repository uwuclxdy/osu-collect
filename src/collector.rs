use serde::{Deserialize, Serialize};
use crate::error::{AppError, Result};

const MAX_RETRIES: u8 = 3;
const COLLECTION_FETCH_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Deserialize, Serialize)]
pub struct Collection {
    pub id: u32,
    pub name: Box<str>,
    pub uploader: Uploader,
    pub beatmapsets: Vec<Beatmapset>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Uploader {
    pub id: u32,
    pub username: Box<str>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Beatmapset {
    pub id: u32,
    #[serde(default)]
    pub beatmaps: Vec<Beatmap>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Beatmap {
    pub id: u32,
    pub checksum: Box<str>,
}

/// Fetch collection from osucollector API with retry logic
pub async fn fetch_collection(
    client: &reqwest::Client,
    collection_id: u32,
) -> Result<Collection> {
    let url = format!("https://osucollector.com/api/collections/{}", collection_id);
    let mut last_error = None;

    for attempt in 1..=MAX_RETRIES {
        match try_fetch_collection(client, &url, collection_id).await {
            Ok(collection) => return Ok(collection),
            Err(e) => {
                let should_retry = matches!(e, AppError::Network(_));

                if should_retry && attempt < MAX_RETRIES {
                    eprintln!("Attempt {} failed, retrying... ({})", attempt, e);
                    let delay_secs = 2_u64.pow((attempt - 1) as u32);
                    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                    last_error = Some(e);
                } else {
                    return Err(e);
                }
            }
        }
    }

    Err(last_error.unwrap_or(
        AppError::api("All retry attempts failed")
    ))
}

/// Create HTTP client optimized for collection fetching
#[inline]
pub fn create_collection_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(COLLECTION_FETCH_TIMEOUT_SECS))
        .build()
        .map_err(AppError::Network)
}

/// Single attempt to fetch collection
async fn try_fetch_collection(
    client: &reqwest::Client,
    url: &str,
    collection_id: u32,
) -> Result<Collection> {
    let response = client.get(url).send().await
        .map_err(|e| {
            if e.is_timeout() {
                AppError::api("Request timed out after 30 seconds")
            } else if e.is_connect() {
                AppError::api("Failed to connect to osucollector.com")
            } else {
                AppError::from(e)
            }
        })?;

    let status = response.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::api_dynamic(
            format!("Collection {} not found (404)", collection_id).into_boxed_str()
        ));
    }

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(AppError::api(
            "Rate limited by osucollector.com (429). Please try again later."
        ));
    }

    if !status.is_success() {
        return Err(AppError::api_dynamic(
            format!("Failed to fetch collection: HTTP {}", status).into_boxed_str()
        ));
    }

    let collection: Collection = response.json().await
        .map_err(|e| AppError::api_dynamic(
            format!("Failed to parse collection JSON: {}", e).into_boxed_str()
        ))?;

    Ok(collection)
}

/// Display collection information
pub fn display_collection_info(collection: &Collection) {
    println!("\nCollection: \"{}\"", collection.name);
    println!("Uploader: {}", collection.uploader.username);
    println!("Total beatmaps: {}", collection.beatmapsets.len());
}
