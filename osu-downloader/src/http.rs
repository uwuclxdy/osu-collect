//! HTTP client creation and configuration (internal).

use crate::Result;
use std::time::Duration;

pub(crate) const DEFAULT_DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(any(feature = "collection", feature = "size-fetch", feature = "search"))]
pub(crate) const DEFAULT_API_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn create_download_client(user_agent: Option<String>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(DEFAULT_DOWNLOAD_CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .pool_max_idle_per_host(10);

    if let Some(ua) = user_agent {
        builder = builder.user_agent(ua);
    }

    builder.build().map_err(Into::into)
}

#[cfg(any(feature = "collection", feature = "size-fetch", feature = "search"))]
pub(crate) fn create_api_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(DEFAULT_API_TIMEOUT)
        .pool_max_idle_per_host(20)
        .build()?)
}
