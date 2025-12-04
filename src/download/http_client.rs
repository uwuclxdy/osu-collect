use super::constants::{COLLECTION_FETCH_TIMEOUT_SECS, DOWNLOAD_TIMEOUT_SECS};
use crate::utils::{AppError, Result};
use std::{sync::OnceLock, time::Duration};

static DOWNLOAD_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static API_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn build_download_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .pool_max_idle_per_host(10)
        .build()
        .map_err(AppError::from)
}

fn build_api_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(COLLECTION_FETCH_TIMEOUT_SECS))
        .pool_max_idle_per_host(5)
        .build()
        .map_err(AppError::from)
}

pub fn download_client() -> Result<reqwest::Client> {
    if let Some(client) = DOWNLOAD_CLIENT.get() {
        return Ok(client.clone());
    }

    let client = build_download_client()?;
    Ok(DOWNLOAD_CLIENT.get_or_init(|| client).clone())
}

pub fn api_client() -> Result<reqwest::Client> {
    if let Some(client) = API_CLIENT.get() {
        return Ok(client.clone());
    }

    let client = build_api_client()?;
    Ok(API_CLIENT.get_or_init(|| client).clone())
}
