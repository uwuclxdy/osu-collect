//! # osu-downloader
//!
//! library for downloading osu! beatmapsets from multiple mirrors.
//!
//! ## quick start
//!
//! ```rust,no_run
//! use osu_downloader::{Downloader, Mirror};
//! use futures_util::StreamExt;
//!
//! # async fn example() -> Result<(), osu_downloader::Error> {
//! let downloader = Downloader::builder()
//!     .mirror(Mirror::nerinyan())
//!     .mirror(Mirror::osu_direct())
//!     .concurrent_downloads(8)
//!     .build()?;
//!
//! let mut session = downloader.download_many([123456u32, 789012], "./downloads");
//! let mut events = session.events().expect("first events() call");
//! while let Some(_event) = events.next().await {
//!     // handle event
//! }
//! let summary = session.wait().await?;
//! println!("downloaded {} beatmapsets", summary.downloaded.len());
//! # Ok(())
//! # }
//! ```
//!
//! ## errors
//!
//! Every fallible call returns the same [`Error`] enum. `Error::is_transient()` is
//! the shortcut for "should I retry?" — true for network, timeout, and rate-limit
//! variants. Collection and size-fetch operations surface the same type.
//!
//! ## features
//!
//! - `collection` — osucollector.com client and `collection.db` writer
//! - `size-fetch` — Nekoha-backed beatmapset size and availability probes
//! - `instrument` — per-attempt observer callback for scheduler tuning
#![deny(missing_docs)]

pub(crate) mod batch;
pub(crate) mod config;
pub(crate) mod download;
mod downloader;
mod error;
mod event;
pub(crate) mod http;
#[cfg(feature = "instrument")]
pub mod instrument;
mod mirrors;
mod output_entry;
pub(crate) mod validation;
pub(crate) mod worker;

#[cfg(feature = "collection")]
pub mod collection;

#[cfg(feature = "size-fetch")]
pub mod size;

pub use download::sanitize_filename;
pub use downloader::{Downloader, DownloaderBuilder, OnExists, Session};
pub use error::{Error, Result};
pub use event::{Event, Skip, Status, Summary};
pub use mirrors::{Mirror, MirrorKind, MirrorRef};
pub use output_entry::{OutputEntry, classify_output_entry};
pub use validation::{
    ArchiveValidation, ArchiveValidationResult, validate_and_remove, validate_archive,
};

mod url_parse;
pub use url_parse::parse_collection_id;
