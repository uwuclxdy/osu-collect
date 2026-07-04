use super::model::{LogFormat, LogLevel, RetryFailedOnDownload, ThemeMode};
use osu_downloader::ArchiveValidation;
use std::time::Duration;

pub const KB: f64 = 1024.0;
pub const MB: f64 = KB * 1024.0;
pub const GB: f64 = MB * 1024.0;

pub mod status {
    pub const RATE_LIMITED: &str = "rate limited";
    pub const DOWNLOADING: &str = "downloading";
    pub const CHECKING_PREFIX: &str = "checking ";
    pub const FROM_SUFFIX: &str = " from ";
    pub const VERIFYING_PREFIX: &str = "verifying from ";
    pub const DOWNLOADED_FROM_PREFIX: &str = "downloaded from ";
    pub const RETRYING_PREFIX: &str = "retrying ";
    pub const RETRYING_AFTER: &str = " after ";
    pub const RETRYING_ATTEMPT_PREFIX: &str = " (attempt ";
    pub const RATE_LIMITED_SUFFIX: &str = " on all mirrors, waiting";
}

pub const CONCURRENT_REQUESTS: usize = 100;
pub const DEFAULT_PROGRESS_WATCHDOG_SECS: u64 = 120;

pub fn default_threads() -> u8 {
    std::thread::available_parallelism()
        .map(|n| (n.get() as u8).min(100))
        .unwrap_or(1)
}

/// Maximum number of additional passes through the mirror pool after every mirror has
/// exhausted its transient retries. The library waits 5 seconds between passes
/// (cancellable). Beyond this cap the beatmapset is reported as `BeatmapsetFailed`
/// carrying a transient `Error` (e.g. `Error::Network`).
pub const NETWORK_RETRY_CAP: u32 = 1000;

pub const CONFIG_SUBDIR: &str = "osu-collect";
pub const CONFIG_FILE: &str = "config.toml";
pub const CONFIG_ENV_PATH: &str = "OSU_COLLECT_CONFIG";

pub const RELEASES_URL: &str = "https://api.github.com/repos/uwuclxdy/osu-collect/releases/latest";
/// Full release list (newest first); used when the prerelease channel is opted
/// in, since `/releases/latest` excludes prereleases and drafts.
pub const RELEASES_LIST_URL: &str = "https://api.github.com/repos/uwuclxdy/osu-collect/releases";
pub const AUTO_UPDATE_TIMEOUT: Duration = Duration::from_secs(60);

pub const SPEED_UPDATE_INTERVAL: Duration = Duration::from_millis(250);
pub const SPEED_STALE_AFTER: Duration = Duration::from_secs(1);
pub const DISK_CACHE_TTL: Duration = Duration::from_secs(10);
pub const VALIDATION_CACHE_LIMIT: usize = 4096;

pub const THEME_MODES: [ThemeMode; 2] = [ThemeMode::Full, ThemeMode::Compatible];

pub const LOG_LEVELS: [LogLevel; 5] = [
    LogLevel::Error,
    LogLevel::Warn,
    LogLevel::Info,
    LogLevel::Debug,
    LogLevel::Trace,
];

pub const LOG_FORMATS: [LogFormat; 2] = [LogFormat::Compact, LogFormat::Pretty];

pub const ARCHIVE_VALIDATIONS: [ArchiveValidation; 3] = [
    ArchiveValidation::Off,
    ArchiveValidation::Magic,
    ArchiveValidation::Eocd,
];

pub const RETRY_FAILED_ON_DOWNLOAD_MODES: [RetryFailedOnDownload; 3] = [
    RetryFailedOnDownload::Ask,
    RetryFailedOnDownload::Yes,
    RetryFailedOnDownload::No,
];

pub const HOME_TAB_INDEX: usize = 0;
pub const UPDATES_TAB_INDEX: usize = 1;
pub const CONFIG_TAB_INDEX: usize = 2;
pub const STATIC_TABS: usize = 3;

pub const TAB_HOME_LOWER: &str = "home";
pub const TAB_UPDATES_LOWER: &str = "updates";
pub const TAB_CONFIG_LOWER: &str = "config";

/// Free-space threshold below which a disk-low banner and warning pill appear (1 GiB).
pub const DISK_WARN_BYTES: u64 = 1024 * 1024 * 1024;
/// Free-space threshold below which a disk-full banner and danger pill appear (100 MiB).
pub const DISK_DANGER_BYTES: u64 = 100 * 1024 * 1024;

/// Alias kept for any code that references the old name.
pub const LOW_SPACE_THRESHOLD_BYTES: u64 = DISK_WARN_BYTES;

pub const DIRECTORY_LOCK_FILE: &str = ".osu-collect.lock";

pub const API_MAX_RETRIES: u8 = 3;
