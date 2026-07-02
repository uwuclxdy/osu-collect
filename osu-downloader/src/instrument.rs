//! Per-attempt instrumentation hooks (`instrument` feature).
//!
//! Opt-in observability for fitting the mirror scheduler's timing constants.
//! Enable the `instrument` feature and attach an observer with
//! [`DownloaderBuilder::attempt_observer`](crate::DownloaderBuilder::attempt_observer);
//! it receives one [`AttemptRecord`] per HTTP attempt.

use crate::MirrorKind;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// Outcome of a single mirror attempt, as seen by an [`AttemptObserver`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// 2xx response accepted for download.
    Success,
    /// 404: the set is absent on this mirror.
    NotFound,
    /// 429: the mirror rate-limited the request.
    RateLimited,
    /// A transient failure (timeout, connect error, 5xx) eligible for retry.
    Transient,
    /// A non-retryable failure (unexpected status or body).
    Definitive,
}

/// One record per mirror attempt, delivered to an [`AttemptObserver`].
#[derive(Debug, Clone)]
pub struct AttemptRecord {
    /// Mirror host contacted (e.g. `"osu.direct"`).
    pub host: Box<str>,
    /// Mirror kind.
    pub kind: MirrorKind,
    /// Time since the session started.
    pub elapsed: Duration,
    /// Attempt outcome.
    pub outcome: AttemptOutcome,
    /// HTTP status, if a response was received.
    pub http_status: Option<u16>,
    /// `Retry-After` parsed from a 429 response, if present.
    pub retry_after: Option<Duration>,
    /// Scheduler request spacing for this slot when the request was sent.
    pub interval: Duration,
    /// Wall-clock latency of the request.
    pub latency: Duration,
}

/// Observer invoked once per mirror attempt. Cheap to clone (wraps an [`Arc`]).
#[derive(Clone)]
pub struct AttemptObserver(Arc<dyn Fn(AttemptRecord) + Send + Sync>);

impl AttemptObserver {
    /// Wrap a callback as an observer.
    pub fn new(callback: impl Fn(AttemptRecord) + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }

    pub(crate) fn emit(&self, record: AttemptRecord) {
        (self.0)(record);
    }
}

impl fmt::Debug for AttemptObserver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AttemptObserver")
    }
}
