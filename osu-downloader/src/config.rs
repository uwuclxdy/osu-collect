//! Configuration types for the downloader

use crate::downloader::OnExists;
use crate::validation::ArchiveValidation;
use std::time::Duration;

pub(crate) const TRANSIENT_RETRY_ATTEMPTS: u32 = 3;
pub(crate) const TRANSIENT_RETRY_BASE_DELAY: Duration = Duration::from_millis(500);
/// Delay between full network-retry passes when every mirror has exhausted transient errors.
pub(crate) const NETWORK_RETRY_BACKOFF: Duration = Duration::from_secs(5);

/// Longest rate-limit cooldown a map waits out inline before it defers instead.
/// A `WaitUntil` shorter than this is slept through in `download_beatmapset`; a
/// longer one returns the map to the batch queue for later retry.
pub(crate) const INLINE_WAIT_MAX: Duration = Duration::from_secs(2);

/// Deferral pass cap: a map gets up to this many processing passes (the
/// initial pass plus 2 requeues); the deferral ending the last pass is dropped
/// as [`Skip::RateLimitSkipped`](crate::Skip::RateLimitSkipped) instead of
/// requeued. Guarantees termination for a set that is rate-limited on every
/// mirror indefinitely.
pub(crate) const DEFERRAL_PASS_CAP: u32 = 3;

#[derive(Debug, Clone)]
pub(crate) struct DownloadConfig {
    pub(crate) concurrent_downloads: usize,
    pub(crate) archive_validation: ArchiveValidation,
    pub(crate) progress_timeout: Duration,
    pub(crate) user_agent: String,
    pub(crate) network_retry_attempts: usize,
    pub(crate) sanitize_filenames: bool,
    pub(crate) on_exists: OnExists,
    /// Per-pass cumulative inline rate-limit wait after which a map defers itself
    /// (reset each pass, so up to ~3x this across the pass cap); `None` waits forever.
    pub(crate) rate_limit_skip_after: Option<Duration>,
    /// Per-attempt observer for the tuning harness.
    #[cfg(feature = "instrument")]
    pub(crate) attempt_observer: Option<crate::instrument::AttemptObserver>,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            concurrent_downloads: 4,
            archive_validation: ArchiveValidation::Magic,
            progress_timeout: Duration::from_secs(30),
            user_agent: format!("osu-downloader/{}", env!("CARGO_PKG_VERSION")),
            network_retry_attempts: 0,
            sanitize_filenames: true,
            on_exists: OnExists::Skip,
            rate_limit_skip_after: None,
            #[cfg(feature = "instrument")]
            attempt_observer: None,
        }
    }
}
