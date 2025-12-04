use std::io;
use thiserror::Error;

use crate::utils::AppError;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum DownloadError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Rate limited")]
    RateLimited,

    #[error("Not found: {0}")]
    NotFound(Box<str>),

    #[error("Invalid archive: {0}")]
    InvalidArchive(Box<str>),

    #[error("Disk full: {0}")]
    DiskFull(Box<str>),

    #[error("Download aborted")]
    Aborted,

    #[error("Timeout: {0}")]
    Timeout(Box<str>),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("No mirrors available")]
    NoMirrors,

    #[error("No beatmapsets selected")]
    NoBeatmapsets,

    #[error("Collection is empty")]
    EmptyCollection,

    #[error("Directory not empty")]
    DirectoryNotEmpty,

    #[error("Concurrent download in progress for: {0}")]
    ConcurrentDownload(String),

    #[error("{0}")]
    Other(Box<str>),
}

#[allow(dead_code)]
impl DownloadError {
    #[inline]
    pub fn not_found(msg: impl Into<Box<str>>) -> Self {
        Self::NotFound(msg.into())
    }

    #[inline]
    pub fn invalid_archive(msg: impl Into<Box<str>>) -> Self {
        Self::InvalidArchive(msg.into())
    }

    #[inline]
    pub fn disk_full(msg: impl Into<Box<str>>) -> Self {
        Self::DiskFull(msg.into())
    }

    #[inline]
    pub fn timeout(msg: impl Into<Box<str>>) -> Self {
        Self::Timeout(msg.into())
    }

    #[inline]
    pub fn other(msg: impl Into<Box<str>>) -> Self {
        Self::Other(msg.into())
    }
}

impl From<AppError> for DownloadError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::Network(e) => Self::other(e.to_string()),
            AppError::FileSystem(e) => Self::other(e.to_string()),
            AppError::Parsing(e) => Self::other(e.to_string()),
            AppError::Config(e) => Self::other(e.to_string()),
            AppError::Domain(e) => Self::other(e.to_string()),
        }
    }
}
