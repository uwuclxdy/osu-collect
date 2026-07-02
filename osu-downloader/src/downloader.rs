//! Public download API.
//!
//! [`Downloader`] is the only entrypoint. Build one with [`DownloaderBuilder`], then call
//! [`Downloader::download_many`] to start a session and consume its event stream.

use crate::{
    Error, Event, Result, Summary,
    batch::{self, BatchConfig},
    config::{DownloadConfig, INLINE_WAIT_MAX},
    http,
    mirrors::{Mirror, MirrorPool},
    validation::ArchiveValidation,
};
use futures_util::Stream;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::{Notify, mpsc, watch};
use tokio_stream::wrappers::UnboundedReceiverStream;

/// How to handle a beatmapset whose target archive already exists on disk.
///
/// Applies to every item in a [`Downloader::download_many`] call. Set on the
/// builder via [`DownloaderBuilder::on_exists`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnExists {
    /// Skip if any matching archive is already present (default).
    #[default]
    Skip,
    /// Overwrite the exact target filename.
    Overwrite,
}

/// Builder for a [`Downloader`].
pub struct DownloaderBuilder {
    mirrors: Vec<Mirror>,
    concurrent_downloads: Option<usize>,
    archive_validation: Option<ArchiveValidation>,
    progress_timeout: Option<Duration>,
    user_agent: Option<String>,
    network_retry_attempts: usize,
    sanitize_filenames: bool,
    on_exists: OnExists,
    rate_limit_skip_after: Option<Duration>,
    #[cfg(feature = "instrument")]
    attempt_observer: Option<crate::instrument::AttemptObserver>,
    #[cfg(any(test, feature = "test-helpers"))]
    http_client_override: Option<reqwest::Client>,
}

impl DownloaderBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            mirrors: Vec::new(),
            concurrent_downloads: None,
            archive_validation: None,
            progress_timeout: None,
            user_agent: None,
            network_retry_attempts: 0,
            sanitize_filenames: true,
            on_exists: OnExists::Skip,
            rate_limit_skip_after: None,
            #[cfg(feature = "instrument")]
            attempt_observer: None,
            #[cfg(any(test, feature = "test-helpers"))]
            http_client_override: None,
        }
    }

    /// Add a mirror.
    #[must_use]
    pub fn mirror(mut self, mirror: Mirror) -> Self {
        self.mirrors.push(mirror);
        self
    }

    /// Add several mirrors.
    #[must_use]
    pub fn mirrors(mut self, mirrors: impl IntoIterator<Item = Mirror>) -> Self {
        self.mirrors.extend(mirrors);
        self
    }

    /// Add every built-in mirror.
    #[must_use]
    pub fn builtins(mut self) -> Self {
        self.mirrors.extend(Mirror::builtins());
        self
    }

    /// Set max concurrent downloads (default 4).
    #[must_use]
    pub fn concurrent_downloads(mut self, count: usize) -> Self {
        self.concurrent_downloads = Some(count);
        self
    }

    /// Archive validation strictness (default [`ArchiveValidation::Magic`]).
    /// `Off` skips validation; `Magic` checks the ZIP local-file-header
    /// signature; `Eocd` additionally checks the end-of-central-directory footer.
    #[must_use]
    pub fn archive_validation(mut self, mode: ArchiveValidation) -> Self {
        self.archive_validation = Some(mode);
        self
    }

    /// Stall-watchdog for the body stream (default 30s).
    #[must_use]
    pub fn progress_timeout(mut self, timeout: Duration) -> Self {
        self.progress_timeout = Some(timeout);
        self
    }

    /// Custom User-Agent.
    #[must_use]
    pub fn user_agent(mut self, agent: impl Into<String>) -> Self {
        self.user_agent = Some(agent.into());
        self
    }

    /// Maximum number of additional passes through the mirror pool after every
    /// mirror has exhausted its transient retries. Each retry waits 5 seconds
    /// (cancellable) before the next pass. Default `0` (no retry).
    #[must_use]
    pub fn network_retry_attempts(mut self, attempts: usize) -> Self {
        self.network_retry_attempts = attempts;
        self
    }

    /// Whether to sanitize archive filenames extracted from `Content-Disposition`
    /// headers. Default `true`. Disabling allows mirrors to dictate the final
    /// on-disk filename verbatim — only do this if every mirror is trusted, as
    /// it removes the path-traversal guard.
    ///
    /// The default sanitizer is exposed as [`crate::sanitize_filename`] if you
    /// want to call it yourself.
    #[must_use]
    pub fn sanitize_filenames(mut self, enabled: bool) -> Self {
        self.sanitize_filenames = enabled;
        self
    }

    /// What to do when a beatmapset archive already exists in the output
    /// directory. Default [`OnExists::Skip`].
    #[must_use]
    pub fn on_exists(mut self, policy: OnExists) -> Self {
        self.on_exists = policy;
        self
    }

    /// Auto-defer a beatmapset once the time it has spent on inline rate-limit
    /// cooldown waits reaches `budget`. The budget is PER PROCESSING PASS, not per
    /// map lifetime: the accumulator resets each pass, so across the deferral pass
    /// cap a map may wait up to ~3x `budget` inline in total before it terminates.
    /// The map is returned to the queue for a later pass rather than dropped, so it
    /// is not lost; it is dropped only after the deferral pass cap is reached.
    /// `None` (default) waits each cooldown out inline indefinitely.
    #[must_use]
    pub fn rate_limit_skip_after(mut self, budget: Option<Duration>) -> Self {
        self.rate_limit_skip_after = budget;
        self
    }

    /// Attach a per-attempt observer (`instrument` feature). It receives one
    /// [`AttemptRecord`](crate::instrument::AttemptRecord) per HTTP attempt,
    /// for fitting the scheduler's timing constants.
    #[cfg(feature = "instrument")]
    #[must_use]
    pub fn attempt_observer(mut self, observer: crate::instrument::AttemptObserver) -> Self {
        self.attempt_observer = Some(observer);
        self
    }

    /// Override the HTTP client (test helper).
    #[cfg(any(test, feature = "test-helpers"))]
    #[must_use]
    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.http_client_override = Some(client);
        self
    }

    /// Build the [`Downloader`].
    pub fn build(self) -> Result<Downloader> {
        if self.mirrors.is_empty() {
            return Err(Error::config(
                "at least one mirror must be configured (use .builtins() or .mirror())",
            ));
        }

        let concurrent_downloads = self.concurrent_downloads.unwrap_or(4);
        if concurrent_downloads == 0 {
            return Err(Error::config(
                "concurrent downloads must be greater than zero",
            ));
        }

        let config = DownloadConfig {
            concurrent_downloads,
            archive_validation: self.archive_validation.unwrap_or(ArchiveValidation::Magic),
            progress_timeout: self.progress_timeout.unwrap_or(Duration::from_secs(30)),
            user_agent: self
                .user_agent
                .unwrap_or_else(|| format!("osu-downloader/{}", env!("CARGO_PKG_VERSION"))),
            network_retry_attempts: self.network_retry_attempts,
            sanitize_filenames: self.sanitize_filenames,
            on_exists: self.on_exists,
            rate_limit_skip_after: self.rate_limit_skip_after,
            #[cfg(feature = "instrument")]
            attempt_observer: self.attempt_observer,
        };

        #[cfg(any(test, feature = "test-helpers"))]
        let http_client = if let Some(client) = self.http_client_override {
            client
        } else {
            http::create_download_client(Some(config.user_agent.clone()))?
        };
        #[cfg(not(any(test, feature = "test-helpers")))]
        let http_client = http::create_download_client(Some(config.user_agent.clone()))?;

        Ok(Downloader {
            config: Arc::new(config),
            http_client,
            mirror_pool: Arc::new(MirrorPool::new(self.mirrors)),
        })
    }
}

impl Default for DownloaderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// The downloader.
pub struct Downloader {
    config: Arc<DownloadConfig>,
    http_client: reqwest::Client,
    mirror_pool: Arc<MirrorPool>,
}

impl Downloader {
    /// New builder.
    pub fn builder() -> DownloaderBuilder {
        DownloaderBuilder::new()
    }

    #[cfg(test)]
    pub(crate) fn mirrors(&self) -> &[Mirror] {
        self.mirror_pool.mirrors()
    }

    /// Start a batch download for the given beatmapset IDs into `output_dir`.
    /// Returns a [`Session`] for events + cancel.
    pub fn download_many(
        &self,
        ids: impl IntoIterator<Item = u32>,
        output_dir: impl AsRef<Path>,
    ) -> Session {
        let ids: Vec<u32> = ids.into_iter().collect();
        let output_dir = output_dir.as_ref().to_path_buf();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let defer = Arc::new(Notify::new());
        let drop_signal = Arc::new(Notify::new());

        let client = self.http_client.clone();
        let mirror_pool = self.mirror_pool.clone();
        let config = self.config.clone();
        let batch_defer = defer.clone();
        let batch_drop = drop_signal.clone();

        let task = tokio::spawn(async move {
            let batch_config = BatchConfig {
                concurrent_downloads: config.concurrent_downloads,
                archive_validation: config.archive_validation,
                progress_timeout: config.progress_timeout,
                network_retry_attempts: config.network_retry_attempts,
                sanitize_filenames: config.sanitize_filenames,
                on_exists: config.on_exists,
                rate_limit_skip_after: config.rate_limit_skip_after,
                inline_wait_max: INLINE_WAIT_MAX,
                #[cfg(feature = "instrument")]
                attempt_observer: config.attempt_observer.clone(),
                #[cfg(feature = "instrument")]
                session_start: std::time::Instant::now(),
            };
            batch::download_batch(
                ids,
                &output_dir,
                client,
                mirror_pool,
                batch_config,
                event_tx,
                cancel_rx,
                batch_defer,
                batch_drop,
            )
            .await
        });

        Session {
            events: Some(event_rx),
            cancel: cancel_tx,
            defer,
            drop_signal,
            task,
        }
    }
}

/// Handle to a running batch session.
pub struct Session {
    events: Option<mpsc::UnboundedReceiver<Event>>,
    cancel: watch::Sender<bool>,
    defer: Arc<Notify>,
    drop_signal: Arc<Notify>,
    task: tokio::task::JoinHandle<Summary>,
}

impl Session {
    /// Consume the event stream. Returns `None` if already taken.
    pub fn events(&mut self) -> Option<impl Stream<Item = Event> + Unpin + Send + 'static> {
        self.events.take().map(UnboundedReceiverStream::new)
    }

    /// Signal cancellation. Running downloads abort at the next checkpoint.
    pub fn cancel(&self) {
        let _ = self.cancel.send(true);
    }

    /// Defer every map *currently* parked on an inline rate-limit **cooldown**
    /// wait (inline-cooling waiters only). Each such map is returned to the
    /// queue tail (its pass counter incremented) and retried once a mirror
    /// frees, rather than dropped. Items already deferred to the queue are
    /// untouched (they are already deferred), as are maps in plain
    /// request-spacing waits and maps that hit a rate limit later. A no-op
    /// when nothing is parked.
    pub fn defer_rate_limited(&self) {
        self.defer.notify_waiters();
    }

    /// Hard-drop every map rate-limit-blocked at the instant of the call:
    /// inline-cooling waiters **and** deferred-pending items sitting in the
    /// batch queue. Each ends as
    /// [`Skip::RateLimitSkipped`](crate::Skip::RateLimitSkipped) immediately,
    /// even while every worker is busy streaming other maps. Untouched: fresh
    /// queue items that merely have not started yet, maps in plain
    /// request-spacing waits, and maps that hit a rate limit later. Unlike
    /// [`defer_rate_limited`](Self::defer_rate_limited) this discards the maps.
    pub fn skip_rate_limited(&self) {
        self.drop_signal.notify_waiters();
    }

    /// Wait for the task to finish and return the [`Summary`]. Drops any remaining events.
    pub async fn wait(mut self) -> Result<Summary> {
        if let Some(mut rx) = self.events.take() {
            while rx.recv().await.is_some() {}
        }
        self.task
            .await
            .map_err(|err| Error::Network(format!("download task panicked: {err}")))
    }
}

#[cfg(test)]
#[path = "../tests/downloader.rs"]
mod tests;
