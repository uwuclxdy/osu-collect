use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Network(#[from] NetworkError),
    #[error(transparent)]
    FileSystem(#[from] FileSystemError),
    #[error(transparent)]
    Parsing(#[from] ParsingError),
    #[error(transparent)]
    Domain(#[from] DomainError),
}

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("{0}")]
    Validation(&'static str),
    #[error("{0}")]
    ValidationDynamic(Box<str>),
}

#[derive(Error, Debug)]
pub enum NetworkError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    Api(&'static str),
    #[error("{0}")]
    ApiDynamic(Box<str>),
}

#[derive(Error, Debug)]
pub enum FileSystemError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("{message}")]
    Context {
        #[source]
        source: Option<io::Error>,
        message: Box<str>,
    },
}

impl FileSystemError {
    pub fn contextual(message: impl Into<Box<str>>) -> Self {
        Self::Context {
            source: None,
            message: message.into(),
        }
    }

    pub fn contextual_with_source(source: io::Error, message: impl Into<Box<str>>) -> Self {
        Self::Context {
            source: Some(source),
            message: message.into(),
        }
    }
}

#[derive(Error, Debug)]
pub enum ParsingError {
    #[error("Invalid URL format: {0}")]
    InvalidUrl(&'static str),
    #[error("{0}")]
    InvalidUrlDynamic(Box<str>),
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Error, Debug)]
pub enum DomainError {
    #[error("{0}")]
    General(&'static str),
    #[error("{0}")]
    GeneralDynamic(Box<str>),
}

impl AppError {
    #[inline]
    pub const fn invalid_url(msg: &'static str) -> Self {
        AppError::Parsing(ParsingError::InvalidUrl(msg))
    }

    #[inline]
    pub fn invalid_url_dynamic(msg: impl Into<Box<str>>) -> Self {
        AppError::Parsing(ParsingError::InvalidUrlDynamic(msg.into()))
    }

    #[inline]
    pub const fn api(msg: &'static str) -> Self {
        AppError::Network(NetworkError::Api(msg))
    }

    #[inline]
    pub fn api_dynamic(msg: impl Into<Box<str>>) -> Self {
        AppError::Network(NetworkError::ApiDynamic(msg.into()))
    }

    #[inline]
    pub const fn other(msg: &'static str) -> Self {
        AppError::Domain(DomainError::General(msg))
    }

    #[inline]
    pub fn other_dynamic(msg: impl Into<Box<str>>) -> Self {
        AppError::Domain(DomainError::GeneralDynamic(msg.into()))
    }

    #[inline]
    pub const fn config(msg: &'static str) -> Self {
        AppError::Config(ConfigError::Validation(msg))
    }

    #[inline]
    pub fn config_dynamic(msg: impl Into<Box<str>>) -> Self {
        AppError::Config(ConfigError::ValidationDynamic(msg.into()))
    }

    #[inline]
    pub fn filesystem_context(message: impl Into<Box<str>>) -> Self {
        AppError::FileSystem(FileSystemError::contextual(message))
    }

    #[inline]
    pub fn filesystem_context_with_source(source: io::Error, message: impl Into<Box<str>>) -> Self {
        AppError::FileSystem(FileSystemError::contextual_with_source(source, message))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::Network(NetworkError::from(err))
    }
}

impl From<io::Error> for AppError {
    fn from(err: io::Error) -> Self {
        AppError::FileSystem(FileSystemError::from(err))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Parsing(ParsingError::from(err))
    }
}
