use super::{
    custom_mirrors::CustomMirrorList,
    first_field,
    home::InputField,
    last_field,
    messages::{AppMessage, clear_app_message, set_loading_message},
    next_field, prev_field,
};
use crate::{
    config::{
        Config, DisplayConfig, DownloadConfig, LogFormat, LogLevel, LoggingConfig, MirrorConfig,
        RetryFailedOnDownload, ThemeMode, UpdateConfig,
        constants::{
            ARCHIVE_VALIDATIONS, LOG_FORMATS, LOG_LEVELS, RETRY_FAILED_ON_DOWNLOAD_MODES,
            THEME_MODES, default_threads,
        },
    },
    download::ArchiveValidation,
    utils::expand_tilde,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthLoginState {
    LoggedOut,
    InProgress(String),
    LoggedIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    AuthChip,
    Theme,
    VimKeys,
    MirrorNerinyan,
    MirrorOsuDirect,
    MirrorSayobot,
    MirrorNekoha,
    MirrorBeatconnect,
    MirrorOsudl,
    MirrorCatboy,
    MirrorHinamizawa,
    MirrorOsuOfficial,
    /// One custom-mirror URL row, indexed into [`CustomMirrorList`]. The last
    /// index is always the empty "add new" entry slot.
    MirrorCustomUrl(usize),
    DownloadThreads,
    DownloadVideo,
    DownloadArchiveValidation,
    RetryFailedOnDownload,
    DownloadAutoSkipRateLimited,
    DownloadRateLimitSkipSecs,
    DownloadSkipAlreadyImported,
    LoggingEnabled,
    LoggingLevel,
    LoggingFormat,
    LoggingDirectory,
    AutoUpdate,
}

// Navigation order — must mirror the render order in `tui::config`
// (`build_config_items`): auth · display · mirrors · download · logging,
// matching the home tab's mirrors-before-download flow. The dynamic
// custom-mirror rows sit between these two slices (after the built-in mirrors).
const CONFIG_FIELDS_BEFORE_CUSTOM: &[ConfigField] = &[
    ConfigField::AuthChip,
    ConfigField::Theme,
    ConfigField::VimKeys,
    ConfigField::MirrorOsuDirect,
    ConfigField::MirrorNerinyan,
    ConfigField::MirrorSayobot,
    ConfigField::MirrorNekoha,
    ConfigField::MirrorBeatconnect,
    ConfigField::MirrorOsudl,
    ConfigField::MirrorCatboy,
    ConfigField::MirrorHinamizawa,
    ConfigField::MirrorOsuOfficial,
];

const CONFIG_FIELDS_AFTER_CUSTOM: &[ConfigField] = &[
    ConfigField::DownloadVideo,
    ConfigField::DownloadThreads,
    ConfigField::DownloadArchiveValidation,
    ConfigField::RetryFailedOnDownload,
    ConfigField::DownloadSkipAlreadyImported,
    ConfigField::DownloadAutoSkipRateLimited,
    ConfigField::DownloadRateLimitSkipSecs,
    ConfigField::LoggingEnabled,
    ConfigField::LoggingLevel,
    ConfigField::LoggingFormat,
    ConfigField::LoggingDirectory,
    ConfigField::AutoUpdate,
];

impl ConfigField {
    pub fn is_text_input(self) -> bool {
        matches!(
            self,
            ConfigField::MirrorCustomUrl(_)
                | ConfigField::LoggingDirectory
                | ConfigField::DownloadRateLimitSkipSecs
        )
    }

    pub fn is_stepper(self) -> bool {
        self == ConfigField::DownloadThreads
    }
}

pub struct ConfigTab {
    pub nerinyan: bool,
    pub osu_direct: bool,
    pub sayobot: bool,
    pub nekoha: bool,
    pub beatconnect: bool,
    pub osudl: bool,
    pub catboy: bool,
    pub hinamizawa: bool,
    pub osu_official: bool,
    pub custom_mirrors: CustomMirrorList,
    pub login_state: AuthLoginState,
    pub threads: InputField,
    pub video: bool,
    pub archive_validation: ArchiveValidation,
    pub retry_failed_on_download: RetryFailedOnDownload,
    pub auto_skip_rate_limited: bool,
    pub rate_limit_skip_secs: InputField,
    pub skip_already_imported: bool,
    pub logging_enabled: bool,
    pub logging_level: LogLevel,
    pub logging_format: LogFormat,
    pub logging_dir: InputField,
    pub theme: ThemeMode,
    pub vim_keys: bool,
    pub auto_update: bool,
    pub focus: ConfigField,
    pub message: Option<AppMessage>,
    pub default_threads: u8,
    /// Config as last persisted to disk. The config tab does not edit the
    /// `recent` last-used inputs, so [`build_config`](Self::build_config) reads
    /// them back from here to avoid wiping the prefill state on save.
    pub loaded_config: Config,
}

impl ConfigTab {
    pub fn new(config: &Config) -> Self {
        let auth_loaded = crate::auth::load().is_some();
        Self {
            nerinyan: config.mirror.nerinyan,
            osu_direct: config.mirror.osu_direct,
            sayobot: config.mirror.sayobot,
            nekoha: config.mirror.nekoha,
            beatconnect: config.mirror.beatconnect,
            osudl: config.mirror.osudl,
            catboy: config.mirror.catboy,
            hinamizawa: config.mirror.hinamizawa,
            osu_official: config.mirror.osu_official,
            custom_mirrors: CustomMirrorList::from_templates(&config.mirror.custom_templates()),
            login_state: login_state(auth_loaded),
            threads: threads_field(&config.download),
            video: config.download.video,
            archive_validation: config.download.archive_validation,
            retry_failed_on_download: config.download.retry_failed_on_download,
            auto_skip_rate_limited: config.download.auto_skip_rate_limited,
            rate_limit_skip_secs: rate_limit_skip_secs_field(&config.download),
            skip_already_imported: config.download.skip_already_imported,
            logging_enabled: config.logging.enabled,
            logging_level: config.logging.level,
            logging_format: config.logging.format,
            logging_dir: logging_dir_field(&config.logging),
            // Absent config key → show the default (full) palette in the cycle.
            theme: config.display.theme.unwrap_or_default(),
            vim_keys: config.display.vim_keys,
            auto_update: config.update.auto_update,
            // Start focus one row below the auth chip so an accidental enter
            // never opens the login tab on entry.
            focus: ConfigField::Theme,
            message: None,
            default_threads: default_threads(),
            loaded_config: config.clone(),
        }
    }

    /// Full focus order with one [`ConfigField::MirrorCustomUrl`] row per custom
    /// entry (including the trailing empty slot), rebuilt each call so the
    /// dynamic custom-mirror count is always reflected.
    pub(crate) fn fields(&self) -> Vec<ConfigField> {
        let mut fields = Vec::with_capacity(
            CONFIG_FIELDS_BEFORE_CUSTOM.len()
                + self.custom_mirrors.row_count()
                + CONFIG_FIELDS_AFTER_CUSTOM.len(),
        );
        fields.extend_from_slice(CONFIG_FIELDS_BEFORE_CUSTOM);
        for idx in 0..self.custom_mirrors.row_count() {
            fields.push(ConfigField::MirrorCustomUrl(idx));
        }
        fields.extend_from_slice(CONFIG_FIELDS_AFTER_CUSTOM);
        fields
    }

    /// Drop emptied custom rows once focus leaves the custom-mirror section.
    fn settle_custom_on_leave(&mut self, old: ConfigField, new: ConfigField) {
        if matches!(old, ConfigField::MirrorCustomUrl(_))
            && !matches!(new, ConfigField::MirrorCustomUrl(_))
        {
            self.custom_mirrors.compact();
        }
    }

    pub fn next_field(&mut self) {
        let next = next_field(&self.fields(), self.focus);
        self.settle_custom_on_leave(self.focus, next);
        self.focus = next;
    }

    pub fn prev_field(&mut self) {
        let prev = prev_field(&self.fields(), self.focus);
        self.settle_custom_on_leave(self.focus, prev);
        self.focus = prev;
    }

    pub fn first_field(&mut self) {
        let first = first_field(&self.fields(), self.focus);
        self.settle_custom_on_leave(self.focus, first);
        self.focus = first;
    }

    pub fn last_field(&mut self) {
        let last = last_field(&self.fields(), self.focus);
        self.settle_custom_on_leave(self.focus, last);
        self.focus = last;
    }

    /// Increment the thread count by one, capped at `default_threads`.
    pub fn step_up(&mut self) {
        self.step(1);
    }

    /// Decrement the thread count by one, floored at 1.
    pub fn step_down(&mut self) {
        self.step(-1);
    }

    fn step(&mut self, delta: i16) {
        let current = self.resolved_threads() as i16;
        let max = self.default_threads as i16;
        let next = (current + delta).clamp(1, max) as u8;
        self.threads.set_value(next.to_string());
    }

    pub fn handle_char(&mut self, ch: char) {
        clear_app_message(&mut self.message);
        if let Some(field) = self.focused_input_mut() {
            field.insert_char(ch);
        }
        self.grow_custom_rows();
    }

    /// Insert a bracketed-paste payload into the focused text field. No-op when
    /// focus is on a non-text field.
    pub fn handle_paste(&mut self, text: &str) {
        clear_app_message(&mut self.message);
        if let Some(field) = self.focused_input_mut() {
            field.insert_str(text);
        }
        self.grow_custom_rows();
    }

    /// After editing a custom-mirror row, keep a trailing empty entry slot.
    fn grow_custom_rows(&mut self) {
        if matches!(self.focus, ConfigField::MirrorCustomUrl(_)) {
            self.custom_mirrors.ensure_trailing_empty();
        }
    }

    pub fn backspace(&mut self) {
        clear_app_message(&mut self.message);
        if let Some(field) = self.focused_input_mut() {
            field.delete_before_caret();
        }
    }

    /// Delete the char at the caret in the focused text field (`Delete` key).
    pub fn delete_forward(&mut self) {
        clear_app_message(&mut self.message);
        if let Some(field) = self.focused_input_mut() {
            field.delete_at_caret();
        }
    }

    /// Delete the word left of the caret in the focused text field
    /// (alt/ctrl+backspace).
    pub fn backspace_word(&mut self) {
        clear_app_message(&mut self.message);
        if let Some(field) = self.focused_input_mut() {
            field.delete_word_before_caret();
        }
    }

    /// Move the caret in the focused text field. No-op on non-text fields.
    pub fn caret_left(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.caret_left();
        }
    }

    pub fn caret_right(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.caret_right();
        }
    }

    pub fn caret_home(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.caret_home();
        }
    }

    pub fn caret_end(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.caret_end();
        }
    }

    /// The focused text input, or `None` for non-text fields. Used by the
    /// renderer to place the caret.
    pub fn focused_input(&self) -> Option<&InputField> {
        match self.focus {
            ConfigField::MirrorCustomUrl(idx) => self.custom_mirrors.row(idx),
            ConfigField::LoggingDirectory => Some(&self.logging_dir),
            ConfigField::DownloadRateLimitSkipSecs => Some(&self.rate_limit_skip_secs),
            _ => None,
        }
    }

    fn focused_input_mut(&mut self) -> Option<&mut InputField> {
        match self.focus {
            ConfigField::MirrorCustomUrl(idx) => self.custom_mirrors.row_mut(idx),
            ConfigField::LoggingDirectory => Some(&mut self.logging_dir),
            ConfigField::DownloadRateLimitSkipSecs => Some(&mut self.rate_limit_skip_secs),
            _ => None,
        }
    }

    pub fn toggle_current(&mut self) {
        clear_app_message(&mut self.message);
        match self.focus {
            ConfigField::Theme => self.cycle_theme(),
            ConfigField::VimKeys => self.vim_keys = !self.vim_keys,
            ConfigField::MirrorNerinyan => self.nerinyan = !self.nerinyan,
            ConfigField::MirrorOsuDirect => self.osu_direct = !self.osu_direct,
            ConfigField::MirrorSayobot => self.sayobot = !self.sayobot,
            ConfigField::MirrorNekoha => self.nekoha = !self.nekoha,
            ConfigField::MirrorBeatconnect => self.beatconnect = !self.beatconnect,
            ConfigField::MirrorOsudl => self.osudl = !self.osudl,
            ConfigField::MirrorCatboy => self.catboy = !self.catboy,
            ConfigField::MirrorHinamizawa => self.hinamizawa = !self.hinamizawa,
            ConfigField::MirrorOsuOfficial => self.osu_official = !self.osu_official,
            ConfigField::DownloadVideo => self.video = !self.video,
            ConfigField::DownloadArchiveValidation => self.cycle_archive_validation(),
            ConfigField::RetryFailedOnDownload => self.cycle_retry_failed_on_download(),
            ConfigField::DownloadAutoSkipRateLimited => {
                self.auto_skip_rate_limited = !self.auto_skip_rate_limited;
            }
            ConfigField::DownloadSkipAlreadyImported => {
                self.skip_already_imported = !self.skip_already_imported;
            }
            ConfigField::LoggingEnabled => self.logging_enabled = !self.logging_enabled,
            ConfigField::LoggingLevel => self.cycle_logging_level(),
            ConfigField::LoggingFormat => self.cycle_logging_format(),
            ConfigField::AutoUpdate => self.auto_update = !self.auto_update,
            ConfigField::AuthChip
            | ConfigField::MirrorCustomUrl(_)
            | ConfigField::DownloadThreads
            | ConfigField::DownloadRateLimitSkipSecs
            | ConfigField::LoggingDirectory => {}
        }
    }

    pub fn cycle_theme(&mut self) {
        self.theme = next_value(THEME_MODES, self.theme);
    }

    pub fn cycle_logging_level(&mut self) {
        self.logging_level = next_value(LOG_LEVELS, self.logging_level);
    }

    pub fn cycle_logging_format(&mut self) {
        self.logging_format = next_value(LOG_FORMATS, self.logging_format);
    }

    pub fn cycle_archive_validation(&mut self) {
        self.archive_validation = next_value(ARCHIVE_VALIDATIONS, self.archive_validation);
    }

    pub fn cycle_retry_failed_on_download(&mut self) {
        self.retry_failed_on_download = next_value(
            RETRY_FAILED_ON_DOWNLOAD_MODES,
            self.retry_failed_on_download,
        );
    }

    pub fn build_config(&self) -> Result<Config, String> {
        let concurrent = self.parse_concurrent()?;
        let mirror = MirrorConfig {
            nerinyan: self.nerinyan,
            osu_direct: self.osu_direct,
            sayobot: self.sayobot,
            nekoha: self.nekoha,
            beatconnect: self.beatconnect,
            osudl: self.osudl,
            catboy: self.catboy,
            hinamizawa: self.hinamizawa,
            osu_official: self.osu_official,
            urls: self.custom_mirrors.nonempty_templates(),
            // Migrate any legacy single URL into `urls` on the next save.
            url: None,
        };

        let download = DownloadConfig {
            concurrent,
            video: self.video,
            archive_validation: self.archive_validation,
            retry_failed_on_download: self.retry_failed_on_download,
            auto_skip_rate_limited: self.auto_skip_rate_limited,
            rate_limit_skip_secs: self.parse_rate_limit_skip_secs().unwrap_or(60),
            skip_already_imported: self.skip_already_imported,
        };

        let logging = LoggingConfig {
            enabled: self.logging_enabled,
            level: self.logging_level,
            format: self.logging_format,
            file_dir: self
                .trimmed_logging_dir()
                .map(|value| value.into_boxed_str()),
        };

        Ok(Config {
            mirror,
            download,
            logging,
            display: DisplayConfig {
                theme: Some(self.theme),
                vim_keys: self.vim_keys,
            },
            update: UpdateConfig {
                auto_update: self.auto_update,
            },
            // The config tab does not edit last-used inputs; preserve whatever
            // was loaded so saving the form never wipes the prefill state.
            recent: self.loaded_config.recent.clone(),
        })
    }

    pub fn set_loading(&mut self, message: impl Into<String>) {
        let text: String = message.into();
        self.login_state = AuthLoginState::InProgress(text.clone());
        set_loading_message(&mut self.message, text);
    }

    pub fn set_login_in_progress(&mut self) {
        self.login_state = AuthLoginState::InProgress(String::new());
    }

    pub fn set_login_complete(&mut self) {
        self.login_state = AuthLoginState::LoggedIn;
        clear_app_message(&mut self.message);
    }

    pub fn set_login_failed(&mut self) {
        self.login_state = AuthLoginState::LoggedOut;
        clear_app_message(&mut self.message);
    }

    pub fn set_logged_out(&mut self) {
        self.login_state = AuthLoginState::LoggedOut;
        clear_app_message(&mut self.message);
    }

    pub fn resolved_threads(&self) -> u8 {
        if self.threads.value.trim().is_empty() {
            self.default_threads
        } else {
            self.threads
                .value
                .trim()
                .parse::<u8>()
                .unwrap_or(self.default_threads)
        }
    }

    fn parse_concurrent(&self) -> Result<Option<u8>, String> {
        let trimmed = self.threads.value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let value = trimmed
            .parse::<u8>()
            .map_err(|_| "Thread count must be a valid number between 1 and 100".to_string())?;
        if value == 0 || value > 100 {
            return Err("Thread count must be between 1 and 100".to_string());
        }

        Ok(Some(value))
    }

    pub(crate) fn parse_rate_limit_skip_secs(&self) -> Result<u32, String> {
        let trimmed = self.rate_limit_skip_secs.value.trim();
        if trimmed.is_empty() {
            return Ok(60);
        }
        trimmed
            .parse::<u32>()
            .map_err(|_| "Rate-limit skip delay must be a valid number".to_string())
    }

    fn trimmed_logging_dir(&self) -> Option<String> {
        let trimmed = self.logging_dir.value.trim();
        if trimmed.is_empty() {
            None
        } else {
            // Expand `~` at save time so the stored path is always absolute.
            Some(expand_tilde(trimmed))
        }
    }
}

fn login_state(auth_loaded: bool) -> AuthLoginState {
    if auth_loaded {
        AuthLoginState::LoggedIn
    } else {
        AuthLoginState::LoggedOut
    }
}

fn threads_field(download: &DownloadConfig) -> InputField {
    InputField::new(
        "default thread count",
        download
            .concurrent
            .map(|value| value.to_string())
            .unwrap_or_default(),
        default_threads().to_string(),
    )
}

fn rate_limit_skip_secs_field(download: &DownloadConfig) -> InputField {
    InputField::new(
        "defer after (secs)",
        download.rate_limit_skip_secs.to_string(),
        "60",
    )
}

fn logging_dir_field(logging: &LoggingConfig) -> InputField {
    InputField::new(
        "Logs directory",
        logging.file_dir.as_deref().unwrap_or(""),
        "~/.local/share/osu-collect/logs",
    )
}

fn next_value<T: Copy + PartialEq, const N: usize>(values: [T; N], current: T) -> T {
    values
        .iter()
        .position(|&value| value == current)
        .map(|idx| values[(idx + 1) % values.len()])
        .unwrap_or(values[0])
}

#[cfg(test)]
#[path = "../../tests/unit/app_config.rs"]
mod tests;
