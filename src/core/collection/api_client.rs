use super::model::Collection;
use crate::{
    config::constants::API_MAX_RETRIES,
    download::http_client,
    utils::{AppError, Result},
};
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

pub trait CollectionService: Send + Sync {
    fn fetch_collection(
        &self,
        collection_id: u32,
    ) -> impl std::future::Future<Output = Result<Collection>> + Send;
}

pub struct HttpCollectionService {
    client: reqwest::Client,
}

impl HttpCollectionService {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub fn builder() -> HttpCollectionServiceBuilder {
        HttpCollectionServiceBuilder::new()
    }
}

impl CollectionService for HttpCollectionService {
    async fn fetch_collection(&self, collection_id: u32) -> Result<Collection> {
        fetch_collection(&self.client, collection_id).await
    }
}

pub struct HttpCollectionServiceBuilder;

impl HttpCollectionServiceBuilder {
    pub fn new() -> Self {
        Self
    }

    pub fn build(self) -> Result<HttpCollectionService> {
        let client = http_client::api_client()?;
        Ok(HttpCollectionService::new(client))
    }
}

pub async fn fetch_collection(client: &reqwest::Client, collection_id: u32) -> Result<Collection> {
    let url = format!("https://osucollector.com/api/collections/{collection_id}");
    let mut last_error = None;

    for attempt in 1..=API_MAX_RETRIES {
        match try_fetch_collection(client, &url, collection_id).await {
            Ok(collection) => return Ok(collection),
            Err(err) => {
                let should_retry = matches!(err, AppError::Network(_));

                if should_retry && attempt < API_MAX_RETRIES {
                    let delay_secs = 2_u64.pow((attempt - 1) as u32);
                    warn!(
                        attempt,
                        remaining_attempts = API_MAX_RETRIES - attempt,
                        delay_secs,
                        error = %err,
                        "Fetch collection attempt failed; retrying"
                    );
                    sleep(Duration::from_secs(delay_secs)).await;
                    last_error = Some(err);
                } else {
                    return Err(err);
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| AppError::api("All retry attempts failed")))
}

async fn try_fetch_collection(
    client: &reqwest::Client,
    url: &str,
    collection_id: u32,
) -> Result<Collection> {
    let response = client.get(url).send().await.map_err(|err| {
        if err.is_timeout() {
            AppError::api("Request timed out after 30 seconds")
        } else if err.is_connect() {
            AppError::api("Failed to connect to osucollector.com")
        } else {
            AppError::from(err)
        }
    })?;

    let status = response.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::api_dynamic(
            format!("Collection {collection_id} not found (404)").into_boxed_str(),
        ));
    }

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(AppError::api(
            "Rate limited by osucollector.com (429). Please try again later.",
        ));
    }

    if !status.is_success() {
        return Err(AppError::api_dynamic(
            format!("Failed to fetch collection: HTTP {status}").into_boxed_str(),
        ));
    }

    let collection: Collection = response.json().await.map_err(|err| {
        AppError::api_dynamic(format!("Failed to parse collection JSON: {err}").into_boxed_str())
    })?;

    Ok(collection)
}
