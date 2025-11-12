use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Invalid URL format: {0}")]
    InvalidUrl(&'static str),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("API error: {0}")]
    Api(&'static str),

    #[error("File system error: {0}")]
    FileSystem(#[from] std::io::Error),

    #[error("JSON parsing error: {0}")]
    JsonParsing(#[from] serde_json::Error),

    #[error("{0}")]
    Other(&'static str),

    #[error("{0}")]
    Dynamic(Box<str>),
}

impl AppError {
    #[inline]
    pub const fn invalid_url(msg: &'static str) -> Self {
        AppError::InvalidUrl(msg)
    }

    #[inline]
    pub fn invalid_url_dynamic(msg: impl Into<Box<str>>) -> Self {
        AppError::Dynamic(msg.into())
    }

    #[inline]
    pub const fn api(msg: &'static str) -> Self {
        AppError::Api(msg)
    }

    #[inline]
    pub fn api_dynamic(msg: impl Into<Box<str>>) -> Self {
        AppError::Dynamic(msg.into())
    }

    #[inline]
    pub const fn other(msg: &'static str) -> Self {
        AppError::Other(msg)
    }

    #[inline]
    pub fn other_dynamic(msg: impl Into<Box<str>>) -> Self {
        AppError::Dynamic(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
