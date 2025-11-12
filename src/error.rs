use std::borrow::Cow;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Invalid URL format: {0}")]
    InvalidUrl(Cow<'static, str>),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("API error: {0}")]
    Api(Cow<'static, str>),

    #[error("File system error: {0}")]
    FileSystem(#[from] std::io::Error),

    #[error("JSON parsing error: {0}")]
    JsonParsing(#[from] serde_json::Error),

    #[error("{0}")]
    Other(Cow<'static, str>),
}

impl AppError {
    pub fn invalid_url_static(msg: &'static str) -> Self {
        AppError::InvalidUrl(Cow::Borrowed(msg))
    }

    pub fn invalid_url_owned(msg: String) -> Self {
        AppError::InvalidUrl(Cow::Owned(msg))
    }

    pub fn api_static(msg: &'static str) -> Self {
        AppError::Api(Cow::Borrowed(msg))
    }

    pub fn api_owned(msg: String) -> Self {
        AppError::Api(Cow::Owned(msg))
    }

    pub fn other_static(msg: &'static str) -> Self {
        AppError::Other(Cow::Borrowed(msg))
    }

    pub fn other_owned(msg: String) -> Self {
        AppError::Other(Cow::Owned(msg))
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
