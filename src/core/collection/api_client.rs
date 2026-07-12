use super::Collection;
use crate::{
    config::constants::API_MAX_RETRIES,
    utils::{AppError, Result},
};
use osu_downloader::{Error, collection::CollectionClient};

pub trait CollectionService: Send + Sync {
    fn fetch_collection(
        &self,
        collection_id: u32,
    ) -> impl std::future::Future<Output = Result<Collection>> + Send;
}

pub struct HttpCollectionService {
    client: CollectionClient,
}

impl HttpCollectionService {
    pub fn new(client: CollectionClient) -> Self {
        Self { client }
    }
}

impl CollectionService for HttpCollectionService {
    async fn fetch_collection(&self, collection_id: u32) -> Result<Collection> {
        fetch_collection(&self.client, collection_id).await
    }
}

pub async fn fetch_collection(client: &CollectionClient, collection_id: u32) -> Result<Collection> {
    client
        .fetch_retrying(collection_id, API_MAX_RETRIES)
        .await
        .map_err(map_collection_error)
}

fn map_collection_error(err: Error) -> AppError {
    match err {
        Error::RateLimited { .. } => {
            AppError::api("rate-limited by osucollector.com (429), try again later")
        }
        Error::NotFound => AppError::api("collection not found (404)"),
        Error::HttpStatus(status) => AppError::api_dynamic(
            format!("failed to fetch collection: HTTP {status}").into_boxed_str(),
        ),
        Error::Timeout => AppError::api("request timed out"),
        Error::Network(msg) => AppError::api_dynamic(msg.into_boxed_str()),
        Error::Parse(msg) => AppError::api_dynamic(
            format!("failed to parse collection JSON: {msg}").into_boxed_str(),
        ),
        Error::InvalidUrl(msg) => AppError::api_dynamic(msg.into_boxed_str()),
        other => AppError::api_dynamic(other.to_string().into_boxed_str()),
    }
}
