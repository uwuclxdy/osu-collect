use super::{
    BeatmapStage, DownloadEvent, DownloadId, DownloadStage, DownloadSummary, Emit, FailedMap,
};
use crate::app::collection::FailureReason;
use crate::config::constants::status::{
    self, CHECKING_PREFIX, DOWNLOADED_FROM_PREFIX, FROM_SUFFIX, RATE_LIMITED_SUFFIX,
    RETRYING_AFTER, RETRYING_ATTEMPT_PREFIX, RETRYING_PREFIX, VERIFYING_PREFIX,
};
use osu_downloader::{Error as LibError, Event as LibEvent, Skip, Status};
use std::collections::HashSet;
use std::time::Instant;
use tracing::warn;

/// Running counters for a pipeline run. Consumed by `translate_event` and converted
/// into the app-facing `DownloadSummary` at the end of a run.
#[derive(Default)]
pub struct Tally {
    pub downloaded: u32,
    pub skipped: u32,
    pub failed: u32,
    pub unverified: u32,
    /// Per-beatmapset failure detail, surfaced in the UI failed-maps panel and
    /// persisted by `failed_maps`.
    pub failures: Vec<FailedMap>,
    /// Beatmapset IDs that this run downloaded successfully.
    pub successful: HashSet<u32>,
}

impl Tally {
    pub fn to_summary(&self) -> DownloadSummary {
        DownloadSummary {
            downloaded: self.downloaded,
            skipped: self.skipped,
            failed: self.failed,
            unverified: self.unverified,
        }
    }

    fn record_completed(&mut self, beatmapset_id: u32) {
        self.downloaded = self.downloaded.saturating_add(1);
        self.successful.insert(beatmapset_id);
        if self.unverified > 0 {
            self.unverified = self.unverified.saturating_sub(1);
        }
    }

    fn record_skipped(&mut self) {
        self.skipped = self.skipped.saturating_add(1);
    }

    fn record_failed(&mut self, beatmapset_id: u32, reason: FailureReason) {
        self.failed = self.failed.saturating_add(1);
        self.failures.push(FailedMap {
            beatmapset_id,
            title: None,
            reason,
        });
    }
}

pub fn translate_event(id: DownloadId, event: LibEvent, tally: &mut Tally, emit: Emit<'_>) {
    match event {
        LibEvent::SessionStarted { .. } | LibEvent::SessionCompleted { .. } => {}
        LibEvent::BeatmapsetStatus {
            beatmapset_id,
            status,
        } => emit_status(id, beatmapset_id, status, emit),
        LibEvent::Progress {
            beatmapset_id,
            downloaded_bytes,
            total_bytes,
            ..
        } => emit(DownloadEvent::BeatmapProgress {
            id,
            beatmapset_id,
            downloaded: downloaded_bytes,
            total: total_bytes.unwrap_or(0),
        }),
        LibEvent::BeatmapsetCompleted {
            beatmapset_id,
            mirror_used,
            verify_duration_us,
            ..
        } => {
            tally.record_completed(beatmapset_id);
            let label = mirror_used.label();
            let mut msg = String::with_capacity(DOWNLOADED_FROM_PREFIX.len() + label.len());
            msg.push_str(DOWNLOADED_FROM_PREFIX);
            msg.push_str(label);
            emit_terminal_status(id, beatmapset_id, BeatmapStage::Success, msg, emit);
            emit(DownloadEvent::BeatmapVerified {
                id,
                duration_us: verify_duration_us,
            });
            emit_overall_progress(id, tally, emit);
        }
        LibEvent::BeatmapsetSkipped {
            beatmapset_id,
            reason,
        } => match reason {
            Skip::AlreadyExists => {
                tally.record_skipped();
                emit_terminal_status(
                    id,
                    beatmapset_id,
                    BeatmapStage::Skipped,
                    "skipped: already exists".to_string(),
                    emit,
                );
                emit_overall_progress(id, tally, emit);
            }
            Skip::UnavailableOnMirrors => {
                record_and_emit_failed(
                    id,
                    beatmapset_id,
                    "unavailable on all mirrors".to_string(),
                    FailureReason::NotFound,
                    tally,
                    emit,
                );
            }
            // User skipped this map while it sat on a rate-limit cooldown.
            // Counted as skipped (not failed) and the active slot is freed.
            Skip::RateLimitSkipped => {
                tally.record_skipped();
                emit_terminal_status(
                    id,
                    beatmapset_id,
                    BeatmapStage::Skipped,
                    "skipped: rate-limited".to_string(),
                    emit,
                );
                emit_overall_progress(id, tally, emit);
            }
        },
        // Soft requeue: the map is not counted (stays "queued"); the app frees
        // its active slot and tracks it as deferred-pending. No toast.
        LibEvent::BeatmapsetDeferred {
            beatmapset_id,
            pass,
            retry_in,
        } => emit(DownloadEvent::BeatmapDeferred {
            id,
            beatmapset_id,
            pass,
            retry_in,
        }),
        LibEvent::BeatmapsetFailed {
            beatmapset_id,
            error,
            ..
        } => {
            let (msg, failure_reason) = classify_error(&error);
            if error.is_transient() {
                warn!(beatmapset_id, reason = %msg, "network error, all mirrors exhausted");
            }
            record_and_emit_failed(id, beatmapset_id, msg, failure_reason, tally, emit);
        }
    }
}

fn record_and_emit_failed(
    id: DownloadId,
    beatmapset_id: u32,
    message: String,
    failure_reason: FailureReason,
    tally: &mut Tally,
    emit: Emit<'_>,
) {
    tally.record_failed(beatmapset_id, failure_reason);
    emit_terminal_status(id, beatmapset_id, BeatmapStage::Failed, message, emit);
    emit_overall_progress(id, tally, emit);
}

/// Map a library `Error` to a `(display_string, FailureReason)` pair.
fn classify_error(error: &LibError) -> (String, FailureReason) {
    match error {
        LibError::NotFound => ("not found".to_string(), FailureReason::NotFound),
        LibError::RateLimited { .. } => (
            "rate-limited, all mirrors exhausted".to_string(),
            FailureReason::RateLimited,
        ),
        LibError::Network(msg) => (format!("network error: {msg}"), FailureReason::NetworkError),
        LibError::Timeout => (
            "operation timed out".to_string(),
            FailureReason::NetworkError,
        ),
        LibError::Validation(msg) => (
            format!("archive validation failed: {msg}"),
            FailureReason::ValidationFailed,
        ),
        // The lib usually returns typed `NotFound` / `RateLimited`, but a few
        // call sites bubble up a raw `HttpStatus(...)` — bucket the well-known
        // codes so the UI still picks the right category.
        LibError::HttpStatus(404) => ("not found".to_string(), FailureReason::NotFound),
        LibError::HttpStatus(429) => (
            "rate-limited (HTTP 429)".to_string(),
            FailureReason::RateLimited,
        ),
        LibError::HttpStatus(code) if (500..600).contains(code) => {
            (format!("HTTP {code}"), FailureReason::NetworkError)
        }
        LibError::Io(msg) => (format!("I/O error: {msg}"), FailureReason::ValidationFailed),
        other => (other.to_string(), FailureReason::Unknown),
    }
}

fn emit_terminal_status(
    id: DownloadId,
    beatmapset_id: u32,
    stage: BeatmapStage,
    message: String,
    emit: Emit<'_>,
) {
    emit(DownloadEvent::BeatmapStatus {
        id,
        beatmapset_id,
        stage,
        message,
        rate_limited: false,
        cooldown_until: None,
    });
}

fn status_msg(prefix: &str, label: &str) -> String {
    let mut s = String::with_capacity(prefix.len() + label.len());
    s.push_str(prefix);
    s.push_str(label);
    s
}

fn emit_status(id: DownloadId, beatmapset_id: u32, event: Status, emit: Emit<'_>) {
    let (message, stage, rate_limited, cooldown_until) = match event {
        // dont remove this
        Status::Contacting { mirror } => (
            status_msg(CHECKING_PREFIX, mirror.label()),
            BeatmapStage::Downloading,
            false,
            None,
        ),
        Status::Downloading { mirror } => {
            let label = mirror.label();
            let mut s =
                String::with_capacity(status::DOWNLOADING.len() + FROM_SUFFIX.len() + label.len());
            s.push_str(status::DOWNLOADING);
            s.push_str(FROM_SUFFIX);
            s.push_str(label);
            (s, BeatmapStage::Downloading, false, None)
        }
        Status::Verifying { mirror } => (
            status_msg(VERIFYING_PREFIX, mirror.label()),
            BeatmapStage::Verifying,
            false,
            None,
        ),
        Status::RateLimited { cooldown } => {
            // Base message ends at "...waiting" with NO number — the live countdown
            // is appended once by `download::rate_limited_item` from `cooldown_until`,
            // so the seconds shown always update (a baked-in number would freeze).
            let mut s =
                String::with_capacity(status::RATE_LIMITED.len() + RATE_LIMITED_SUFFIX.len());
            s.push_str(status::RATE_LIMITED);
            s.push_str(RATE_LIMITED_SUFFIX);
            (
                s,
                BeatmapStage::Downloading,
                true,
                Some(Instant::now() + cooldown),
            )
        }
        Status::RetryingTransient {
            mirror,
            attempt,
            max_attempts,
            reason,
        } => {
            let label = mirror.label();
            let attempt_s = attempt.to_string();
            let max_s = max_attempts.to_string();
            let mut s = String::with_capacity(
                RETRYING_PREFIX.len()
                    + label.len()
                    + RETRYING_AFTER.len()
                    + reason.len()
                    + RETRYING_ATTEMPT_PREFIX.len()
                    + attempt_s.len()
                    + 1
                    + max_s.len()
                    + 1,
            );
            s.push_str(RETRYING_PREFIX);
            s.push_str(label);
            s.push_str(RETRYING_AFTER);
            s.push_str(&reason);
            s.push_str(RETRYING_ATTEMPT_PREFIX);
            s.push_str(&attempt_s);
            s.push('/');
            s.push_str(&max_s);
            s.push(')');
            (s, BeatmapStage::Downloading, false, None)
        }
    };
    emit(DownloadEvent::BeatmapStatus {
        id,
        beatmapset_id,
        stage,
        message,
        rate_limited,
        cooldown_until,
    });
}

pub fn emit_overall_progress(id: DownloadId, tally: &Tally, emit: Emit<'_>) {
    emit(DownloadEvent::OverallProgress {
        id,
        downloaded: tally.downloaded,
        skipped: tally.skipped,
        failed: tally.failed,
        unverified: tally.unverified,
    });
}

pub fn emit_finish(id: DownloadId, emit: Emit<'_>, summary: DownloadSummary) {
    emit(DownloadEvent::Finished { id, summary });
    emit(DownloadEvent::StageChanged {
        id,
        stage: DownloadStage::Completed,
    });
}
