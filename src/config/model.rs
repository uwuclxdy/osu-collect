use crate::{
    download::ArchiveValidation,
    mirrors,
    osu_db::OsuClient,
    utils::{AppError, Result},
};
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Config {
    #[serde(default)]
    pub mirror: MirrorConfig,
    #[serde(default)]
    pub download: DownloadConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub recent: RecentConfig,
    #[serde(default)]
    pub update: UpdateConfig,
}

/// Last-used home-tab inputs, persisted across runs so the collection field and
/// download directory pre-fill with whatever the user downloaded last.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct RecentConfig {
    /// Last collection URL or ID typed into the home form.
    pub collection: Option<String>,
    /// Last download directory typed into the home form.
    pub directory: Option<String>,
    /// Last osu! client kind selected for the app-global library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub osu_client: Option<OsuClient>,
    /// Last osu! installation path typed for the app-global library. Persisted verbatim
    /// even when it no longer exists on disk — a stale path surfaces a "no db"
    /// scan rather than silently reverting to auto-detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub osu_path: Option<String>,
}

/// Self-update behavior.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct UpdateConfig {
    /// When true (default), a newer release is downloaded and applied
    /// automatically at startup. When false, the app only surfaces that an
    /// update exists (header indicator + `u` modal) and applies it on demand.
    pub auto_update: bool,
    /// Opt-in prerelease channel. When true the update check also considers
    /// GitHub prereleases (highest semver wins); default false = stable only.
    pub prereleases: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            auto_update: true,
            prereleases: false,
        }
    }
}

/// Theme selection for the TUI.
///
/// The palette defaults to [`ThemeMode::Full`]. When `display.theme` is absent
/// from config entirely (first run, or a config that failed to parse), the full
/// truecolor palette is used — there is no terminal auto-detection. See
/// `tui::theme::apply_theme`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    /// Force the full Catppuccin Mocha truecolor (RGB) palette.
    #[default]
    Full,
    /// Force the xterm-256 compatible palette.
    Compatible,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct DisplayConfig {
    /// Explicit palette choice. `None` (key absent) selects the full truecolor
    /// palette at startup. Any value the user picks in the config tab pins the
    /// choice from then on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeMode>,
    /// Vim-style navigation keymap. When on, `hjkl` move, `gg`/`G` jump to
    /// top/bottom, `Ctrl+d`/`Ctrl+u` page-scroll, and `i`/`a` enter edit mode on
    /// a focused text field. Off by default; toggled from the config tab. A
    /// text field in edit mode bypasses the layer so typing stays literal.
    #[serde(default)]
    pub vim_keys: bool,
    /// On download launch, switch to the Downloads tab (landing on the run
    /// list). Off by default: launch stays on the current tab, signalled by
    /// the queued toast.
    #[serde(default)]
    pub jump_to_downloads: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MirrorConfig {
    #[serde(default = "default_enabled")]
    pub nerinyan: bool,
    #[serde(default = "default_enabled")]
    pub osu_direct: bool,
    #[serde(default = "default_enabled")]
    pub sayobot: bool,
    #[serde(default = "default_enabled")]
    pub nekoha: bool,
    /// Anonymous beatconnect.io CDN download. On by default.
    #[serde(default = "default_enabled")]
    pub beatconnect: bool,
    /// Anonymous osu!dl (Cloudflare R2) download. On by default: fast and
    /// auth-free, though coverage is ranked/approved/loved only — rotation
    /// backfills the sets it cannot serve.
    #[serde(default = "default_enabled")]
    pub osudl: bool,
    /// Anonymous catboy.best direct download. On by default: fast and auth-free.
    #[serde(default = "default_enabled")]
    pub catboy: bool,
    /// Hinamizawa cascade. Off by default: it races server-side through the
    /// other mirrors, so enabling it alongside them is redundant.
    #[serde(default)]
    pub hinamizawa: bool,
    /// Official osu! API download. Off by default: needs an interactive
    /// `lazer`-scope login and is rate-limited to 10–20 downloads/hour.
    #[serde(default)]
    pub osu_official: bool,
    /// User-defined custom mirror URL templates, each containing `{id}`. Tried
    /// after the built-ins, in list order.
    #[serde(default)]
    pub urls: Vec<Box<str>>,
    /// Legacy single custom mirror URL (pre-multi-mirror configs). Read on load
    /// and folded into [`custom_templates`](MirrorConfig::custom_templates);
    /// never serialized back, so saving migrates it into `urls`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<Box<str>>,
    /// User-defined try-order for the built-in mirrors, as host keys
    /// ([`MirrorKind::host`](mirrors::MirrorKind::host)). Empty (the default)
    /// means the canonical [`MirrorKind::BUILTINS`](mirrors::MirrorKind::BUILTINS)
    /// order. Reconstructed into a ranked list by
    /// [`ordered_builtins`](MirrorConfig::ordered_builtins), which drops unknown
    /// host keys and appends any built-in missing from the list, so the field
    /// can never hide or duplicate a mirror.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<Box<str>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DownloadConfig {
    pub concurrent: Option<u8>,
    /// Whether beatmap videos are included in downloads. `true` (the default)
    /// uses each mirror's full template; `false` switches to its no-video one.
    pub video: bool,
    pub archive_validation: ArchiveValidation,
    pub retry_failed_on_download: RetryFailedOnDownload,
    /// When true, maps parked on a rate-limit cooldown are automatically
    /// deferred (requeued to retry once a mirror frees) after
    /// `rate_limit_skip_secs` seconds instead of waiting indefinitely. The key
    /// name is kept for config compatibility.
    pub auto_skip_rate_limited: bool,
    /// Per-pass seconds of inline cooldown wait before auto-deferring a
    /// rate-limited map (the budget resets each processing pass, so a map may wait
    /// up to ~3x this across the deferral pass cap). Only meaningful when
    /// `auto_skip_rate_limited` is true. Floored at 1 s in the pipeline.
    pub rate_limit_skip_secs: u32,
    /// When true, a Get-Maps download pre-skips beatmapsets already present in
    /// the configured osu! client's library (still written to `collection.db`).
    pub skip_already_imported: bool,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            concurrent: None,
            video: true,
            archive_validation: ArchiveValidation::default(),
            retry_failed_on_download: RetryFailedOnDownload::default(),
            auto_skip_rate_limited: true,
            rate_limit_skip_secs: 60,
            skip_already_imported: true,
        }
    }
}

/// Policy for retrying beatmaps that failed in a previous run when the user
/// kicks off a new download for the same collection.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RetryFailedOnDownload {
    /// Prompt the user before each download when failures intersect.
    #[default]
    Ask,
    /// Always retry — include previously failed beatmaps in the download.
    Yes,
    /// Never retry — skip previously failed beatmaps silently.
    No,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub enabled: bool,
    pub level: LogLevel,
    pub format: LogFormat,
    pub file_dir: Option<Box<str>>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum LogFormat {
    #[default]
    Compact,
    Pretty,
}

fn default_enabled() -> bool {
    true
}

impl Default for MirrorConfig {
    fn default() -> Self {
        Self {
            nerinyan: true,
            osu_direct: true,
            sayobot: true,
            nekoha: true,
            beatconnect: true,
            osudl: true,
            catboy: true,
            hinamizawa: false,
            osu_official: false,
            urls: Vec::new(),
            url: None,
            order: Vec::new(),
        }
    }
}

impl MirrorConfig {
    /// Trimmed, non-empty custom mirror URL templates, in tried order. Folds the
    /// legacy single `url` (if any) ahead of the `urls` list so old configs keep
    /// working until the next save migrates them.
    pub fn custom_templates(&self) -> Vec<&str> {
        self.url
            .as_deref()
            .into_iter()
            .chain(self.urls.iter().map(Box::as_ref))
            .map(str::trim)
            .filter(|template| !template.is_empty())
            .collect()
    }

    /// The built-in mirror kinds in the user's configured try-order.
    ///
    /// Known host keys from [`order`](Self::order) come first in their saved
    /// order (duplicates collapsed, unknown keys dropped), then any built-in
    /// absent from `order` is appended in
    /// [`MirrorKind::BUILTINS`](mirrors::MirrorKind::BUILTINS) order. The result
    /// always holds every built-in exactly once, so it is the single source of
    /// truth for both the pipeline try-order and the TUI mirror-row order.
    pub fn ordered_builtins(&self) -> Vec<mirrors::MirrorKind> {
        use mirrors::MirrorKind;
        let mut ordered: Vec<MirrorKind> = Vec::with_capacity(MirrorKind::BUILTINS.len());
        for key in &self.order {
            if let Some(kind) = MirrorKind::BUILTINS
                .iter()
                .copied()
                .find(|kind| kind.host() == key.as_ref())
                && !ordered.contains(&kind)
            {
                ordered.push(kind);
            }
        }
        for &kind in MirrorKind::BUILTINS {
            if !ordered.contains(&kind) {
                ordered.push(kind);
            }
        }
        ordered
    }

    fn any_enabled(&self) -> bool {
        self.nerinyan
            || self.osu_direct
            || self.sayobot
            || self.nekoha
            || self.beatconnect
            || self.osudl
            || self.catboy
            || self.hinamizawa
            || self.osu_official
            || !self.custom_templates().is_empty()
    }
}

impl DownloadConfig {
    pub fn resolved_concurrent(&self) -> u8 {
        self.concurrent
            .unwrap_or_else(super::constants::default_threads)
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            level: LogLevel::Info,
            format: LogFormat::Compact,
            file_dir: None,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if !self.mirror.any_enabled() {
            return Err(AppError::config("Enable at least one mirror"));
        }

        for template in self.mirror.custom_templates() {
            mirrors::Mirror::custom(template)
                .map_err(|e| AppError::config_dynamic(e.to_string()))?;
        }

        if let Some(concurrent) = self.download.concurrent {
            if concurrent == 0 {
                return Err(AppError::config("Thread count must be at least 1"));
            }

            if concurrent > 100 {
                warn!(
                    concurrent,
                    "Thread count is unusually high; consider lowering to avoid rate limiting"
                );
                warn!("Recommended maximum is 20 to avoid rate limiting");
            }
        }

        if self.logging.enabled
            && let Some(dir) = self.logging.file_dir.as_deref()
            && dir.trim().is_empty()
        {
            return Err(AppError::config(
                "logging.file_dir cannot be empty when logging is enabled",
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/config_theme.rs"]
mod tests;
