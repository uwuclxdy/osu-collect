use std::io;
use thiserror::Error;

use crate::utils::AppError;

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("no mirrors available")]
    NoMirrors,

    #[error("no mapsets selected")]
    NoBeatmapsets,

    #[error("collection is empty")]
    EmptyCollection,

    #[error("concurrent download in progress for: {0}")]
    ConcurrentDownload(String),

    #[error("internal error: {0}")]
    Internal(Box<str>),
}

impl DownloadError {
    #[inline]
    pub fn internal(msg: impl Into<Box<str>>) -> Self {
        Self::Internal(msg.into())
    }
}

impl From<AppError> for DownloadError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::Network(e) => Self::internal(e.to_string()),
            AppError::FileSystem(e) => Self::internal(e.to_string()),
            AppError::Parsing(e) => Self::internal(e.to_string()),
            AppError::Config(e) => Self::internal(e.to_string()),
            AppError::Domain(e) => Self::internal(e.to_string()),
        }
    }
}
