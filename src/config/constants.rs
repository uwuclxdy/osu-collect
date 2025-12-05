use super::model::{LogFormat, LogLevel};
use std::time::Duration;

pub const KB: f64 = 1024.0;
pub const MB: f64 = KB * 1024.0;
pub const GB: f64 = MB * 1024.0;

pub const MAX_TRUNCATED_CHARS: usize = 80;

pub mod status {
    pub const RATE_LIMITED: &str = "Rate limited";
    pub const CONTACTING_PREFIX: &str = "Contacting";
    pub const ABORTED: &str = "Aborted";
    pub const RECHECKING_PREFIX: &str = "Rechecking";
    pub const STARTING_DOWNLOAD: &str = "Starting download";
    pub const DOWNLOADING: &str = "Downloading";
    pub const FETCHING: &str = "Fetching";
    pub const VERIFYING_PREFIX: &str = "Verifying integrity for";
}

pub const MAX_EOCD_SEARCH_BYTES: u64 = 65_558;
pub const CONCURRENT_REQUESTS: usize = 50;
pub const DOWNLOAD_TIMEOUT_SECS: u64 = 60;
pub const DEFAULT_PROGRESS_WATCHDOG_SECS: u64 = 120;
pub const COLLECTION_FETCH_TIMEOUT_SECS: u64 = 30;

pub const DEFAULT_THREADS: u8 = 3;
pub const DEFAULT_RETRIES: u8 = 1;

pub const CONFIG_SUBDIR: &str = "osu-collect";
pub const CONFIG_FILE: &str = "config.toml";
pub const CONFIG_ENV_PATH: &str = "OSU_COLLECT_CONFIG";

pub const EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
pub const MIN_PROGRESS_DELTA: u64 = 256 * 1024;
pub const MIN_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

pub const RELEASES_URL: &str = "https://api.github.com/repos/uwuclxdy/osu-collect/releases/latest";
pub const AUTO_UPDATE_TIMEOUT: Duration = Duration::from_secs(60);

pub const SPEED_STALE_AFTER: Duration = Duration::from_secs(1);
pub const COMPLETION_PREFIXES: [&str; 4] = ["Done", "Skipped", "Failed", "Accepted"];
pub const MAX_LOG_LINES: usize = 5;

pub const VALIDATION_CACHE_LIMIT: usize = 4096;

pub const LOG_LEVELS: [LogLevel; 5] = [
    LogLevel::Error,
    LogLevel::Warn,
    LogLevel::Info,
    LogLevel::Debug,
    LogLevel::Trace,
];

pub const LOG_FORMATS: [LogFormat; 2] = [LogFormat::Compact, LogFormat::Pretty];

pub const HOME_TAB_INDEX: usize = 0;
pub const UPDATES_TAB_INDEX: usize = 1;
pub const CONFIG_TAB_INDEX: usize = 2;
pub const STATIC_TABS: usize = 3;

pub const NEKOHA_API_BASE: &str = "https://mirror.nekoha.moe/api4";
pub const MIRROR_CHECK_URLS: &[&str] = &[
    "https://catboy.best/d/{id}",
    "https://api.nerinyan.moe/d/{id}",
    "https://osu.direct/api/d/{id}",
    "https://dl.sayobot.cn/beatmaps/download/full/{id}",
    "https://mirror.nekoha.moe/api4/download/{id}",
];

pub const LOW_SPACE_THRESHOLD_BYTES: u64 = 1024 * 1024 * 1024;

pub const DIRECTORY_LOCK_FILE: &str = ".osu-collect.lock";

pub const OSU_DB_VERSION: u32 = 20150203;

pub const API_MAX_RETRIES: u8 = 3;
