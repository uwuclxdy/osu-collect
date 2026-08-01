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
    mirrors::MirrorKind,
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
    JumpToDownloads,
    MirrorNerinyan,
    MirrorOsuDirect,
    MirrorSayobot,
    MirrorNekoha,
    MirrorBeatconnect,
    MirrorOsudl,
    MirrorCatboy,
    MirrorHinamizawa,
    MirrorOsuOfficial,
    MirrorNzbasic,
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
    Prereleases,
}

// Navigation order — must mirror the render order in `tui::config`
// (`build_config_items`): auth · display · mirrors · download · logging,
// matching the home tab's mirrors-before-download flow. The built-in mirror
// rows sit between this head slice and the dynamic custom-mirror rows, in the
// configured try-order (`ConfigTab::ordered_mirror_rows`), so they are not
// listed here.
const CONFIG_FIELDS_HEAD: &[ConfigField] = &[
    ConfigField::AuthChip,
    ConfigField::Theme,
    ConfigField::VimKeys,
    ConfigField::JumpToDownloads,
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
    ConfigField::Prereleases,
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
    pub nzbasic: bool,
    pub custom_mirrors: CustomMirrorList,
    pub login_state: AuthLoginState,
    /// Cached osu!supporter status, mirroring `StoredAuth::is_supporter` at
    /// the moment `login_state` last became [`AuthLoginState::LoggedIn`].
    /// Only meaningful while logged in; `false` on every other state.
    ///
    /// PRIVATE on purpose. The six supporter-gated find facets reset their
    /// values and release focus when this goes false, and that settle runs in
    /// the setters below. A direct write would skip it and strand the form with
    /// an invisible filter still steering the query. Read it through
    /// [`ConfigTab::supporter`].
    supporter: bool,
    /// Cached `*`-scope fact from [`StoredAuth::has_lazer_scope`], mirroring
    /// `supporter` — updated on the same auth transitions so the two derived
    /// facts cannot drift. Read it through
    /// [`App::osu_official_unlocked`](crate::app::App::osu_official_unlocked),
    /// which ANDs it with `login_state == LoggedIn`.
    pub lazer_scope: bool,
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
    pub jump_to_downloads: bool,
    pub auto_update: bool,
    pub prereleases: bool,
    /// Built-in mirror try-order, seeded from
    /// [`MirrorConfig::ordered_builtins`](crate::config::MirrorConfig::ordered_builtins).
    /// Reordered in place by [`reorder_focused_mirror`](Self::reorder_focused_mirror)
    /// and written back to `[mirror].order` by [`build_config`](Self::build_config).
    pub mirror_order: Vec<MirrorKind>,
    pub focus: ConfigField,
    pub message: Option<AppMessage>,
    pub default_threads: u8,
    /// Config as last persisted to disk. The config tab does not edit the
    /// `recent` last-used inputs, so [`build_config`](Self::build_config) reads
    /// them back from here to avoid wiping the prefill state on save.
    pub loaded_config: Config,
    /// Persisted list scroll offset so the focused row isn't re-pinned to the
    /// panel's bottom edge every frame (see [`widgets::render_list`]).
    pub list_offset: std::cell::Cell<usize>,
}

impl ConfigTab {
    pub fn new(config: &Config) -> Self {
        let stored_auth = crate::auth::load();
        let auth_loaded = stored_auth.is_some();
        let supporter = stored_auth.as_ref().is_some_and(|auth| auth.is_supporter());
        let lazer_scope = stored_auth
            .as_ref()
            .is_some_and(|auth| auth.has_lazer_scope());
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
            nzbasic: config.mirror.nzbasic,
            custom_mirrors: CustomMirrorList::from_templates(&config.mirror.custom_templates()),
            login_state: login_state(auth_loaded),
            supporter,
            lazer_scope,
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
            jump_to_downloads: config.display.jump_to_downloads,
            auto_update: config.update.auto_update,
            prereleases: config.update.prereleases,
            mirror_order: config.mirror.ordered_builtins(),
            // Start focus one row below the auth chip so an accidental enter
            // never opens login on entry.
            focus: ConfigField::Theme,
            message: None,
            default_threads: default_threads(),
            loaded_config: config.clone(),
            list_offset: std::cell::Cell::new(0),
        }
    }

    /// Full focus order with one [`ConfigField::MirrorCustomUrl`] row per custom
    /// entry (including the trailing empty slot), rebuilt each call so the
    /// dynamic custom-mirror count is always reflected.
    pub(crate) fn fields(&self) -> Vec<ConfigField> {
        let mirror_rows = self.ordered_mirror_rows();
        let mut fields = Vec::with_capacity(
            CONFIG_FIELDS_HEAD.len()
                + mirror_rows.len()
                + self.custom_mirrors.row_count()
                + CONFIG_FIELDS_AFTER_CUSTOM.len(),
        );
        fields.extend_from_slice(CONFIG_FIELDS_HEAD);
        fields.extend(mirror_rows.iter().map(|&(_, field, _)| field));
        for idx in 0..self.custom_mirrors.row_count() {
            fields.push(ConfigField::MirrorCustomUrl(idx));
        }
        fields.extend_from_slice(CONFIG_FIELDS_AFTER_CUSTOM);
        fields
    }

    /// Built-in mirror rows in the configured try-order: `(kind, nav field,
    /// enabled)`. Single source for both the nav order ([`fields`](Self::fields))
    /// and the Config mirrors render, so the two never drift.
    pub(crate) fn ordered_mirror_rows(&self) -> Vec<(MirrorKind, ConfigField, bool)> {
        self.mirror_order
            .iter()
            .filter_map(|&kind| Some((kind, mirror_config_field(kind)?, self.mirror_enabled(kind))))
            .collect()
    }

    /// Whether the built-in mirror of `kind` is toggled on.
    fn mirror_enabled(&self, kind: MirrorKind) -> bool {
        match kind {
            MirrorKind::Nerinyan => self.nerinyan,
            MirrorKind::OsuDirect => self.osu_direct,
            MirrorKind::Sayobot => self.sayobot,
            MirrorKind::Nekoha => self.nekoha,
            MirrorKind::Beatconnect => self.beatconnect,
            MirrorKind::Osudl => self.osudl,
            MirrorKind::Catboy => self.catboy,
            MirrorKind::Hinamizawa => self.hinamizawa,
            MirrorKind::OsuApi => self.osu_official,
            MirrorKind::Nzbasic => self.nzbasic,
            MirrorKind::Custom => false,
        }
    }

    /// Whether focus is on a built-in mirror row (the reorderable set).
    pub fn focus_is_builtin_mirror(&self) -> bool {
        mirror_kind_of(self.focus).is_some()
    }

    /// Focus the first built-in mirror row in the current try-order — the top of
    /// the mirrors section. Used when the Get Maps mirrors summary jumps here.
    pub fn focus_mirrors(&mut self) {
        if let Some(field) = self
            .mirror_order
            .first()
            .and_then(|&k| mirror_config_field(k))
        {
            self.focus = field;
        }
    }

    /// Move the focused built-in mirror one slot up (`up`) or down in the
    /// try-order, keeping focus on that mirror. Returns whether the order
    /// changed — `false` when focus isn't a built-in mirror row or the row is
    /// already at the relevant edge. Enable state is untouched.
    pub fn reorder_focused_mirror(&mut self, up: bool) -> bool {
        let Some(kind) = mirror_kind_of(self.focus) else {
            return false;
        };
        let Some(idx) = self.mirror_order.iter().position(|&k| k == kind) else {
            return false;
        };
        let target = if up {
            idx.checked_sub(1)
        } else {
            Some(idx + 1).filter(|&t| t < self.mirror_order.len())
        };
        let Some(target) = target else {
            return false;
        };
        self.mirror_order.swap(idx, target);
        true
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
            ConfigField::JumpToDownloads => self.jump_to_downloads = !self.jump_to_downloads,
            ConfigField::MirrorNerinyan => self.nerinyan = !self.nerinyan,
            ConfigField::MirrorOsuDirect => self.osu_direct = !self.osu_direct,
            ConfigField::MirrorSayobot => self.sayobot = !self.sayobot,
            ConfigField::MirrorNekoha => self.nekoha = !self.nekoha,
            ConfigField::MirrorBeatconnect => self.beatconnect = !self.beatconnect,
            ConfigField::MirrorOsudl => self.osudl = !self.osudl,
            ConfigField::MirrorCatboy => self.catboy = !self.catboy,
            ConfigField::MirrorHinamizawa => self.hinamizawa = !self.hinamizawa,
            ConfigField::MirrorOsuOfficial => self.osu_official = !self.osu_official,
            ConfigField::MirrorNzbasic => self.nzbasic = !self.nzbasic,
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
            ConfigField::Prereleases => self.prereleases = !self.prereleases,
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
        // Persist an explicit order only once it diverges from the canonical
        // `BUILTINS` order, so an untouched config stays free of the key.
        let order = if self.mirror_order.as_slice() == MirrorKind::BUILTINS {
            Vec::new()
        } else {
            self.mirror_order
                .iter()
                .map(|kind| Box::<str>::from(kind.host()))
                .collect()
        };
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
            nzbasic: self.nzbasic,
            urls: self.custom_mirrors.nonempty_templates(),
            // Migrate any legacy single URL into `urls` on the next save.
            url: None,
            order,
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
                jump_to_downloads: self.jump_to_downloads,
                // Not editable from the config form; preserve whatever was loaded
                // (the delete modal's "don't ask again" toggle writes it directly).
                confirm_delete_history: self.loaded_config.display.confirm_delete_history,
            },
            update: UpdateConfig {
                auto_update: self.auto_update,
                prereleases: self.prereleases,
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

    pub fn set_login_complete(&mut self, supporter: bool) {
        self.login_state = AuthLoginState::LoggedIn;
        self.supporter = supporter;
        // A completed password grant always produces a `*`-scope token.
        self.lazer_scope = true;
        clear_app_message(&mut self.message);
    }

    pub fn set_login_failed(&mut self) {
        self.login_state = AuthLoginState::LoggedOut;
        self.supporter = false;
        self.lazer_scope = false;
        clear_app_message(&mut self.message);
    }

    pub fn set_logged_out(&mut self) {
        self.login_state = AuthLoginState::LoggedOut;
        self.supporter = false;
        self.lazer_scope = false;
        clear_app_message(&mut self.message);
    }

    /// Adopt a `/me` answer for a session that was already logged in — the
    /// startup re-probe. Only a CONFIRMED answer reaches here (see
    /// `AuthEvent::SupporterRefreshed`), so this may legitimately move the flag
    /// in either direction.
    ///
    /// Ignored unless the session is still logged in: a logout that raced the
    /// probe already zeroed the flag, and the answer describes a token that no
    /// longer exists.
    pub fn set_supporter(&mut self, supporter: bool) {
        if self.login_state == AuthLoginState::LoggedIn {
            self.supporter = supporter;
        }
    }

    /// Whether the signed-in account has a CONFIRMED osu!supporter. Unknown
    /// reads as `false`, so an unresolved probe never unlocks a gated feature.
    pub fn supporter(&self) -> bool {
        self.supporter
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
            .map_err(|_| "thread count must be a number between 1 and 100".to_string())?;
        if value == 0 || value > 100 {
            return Err("thread count must be between 1 and 100".to_string());
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
            .map_err(|_| "rate-limit skip delay must be a number".to_string())
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

/// The nav/render field for a built-in mirror kind, or `None` for
/// [`MirrorKind::Custom`] (custom mirrors live in their own rows).
fn mirror_config_field(kind: MirrorKind) -> Option<ConfigField> {
    Some(match kind {
        MirrorKind::Nerinyan => ConfigField::MirrorNerinyan,
        MirrorKind::OsuDirect => ConfigField::MirrorOsuDirect,
        MirrorKind::Sayobot => ConfigField::MirrorSayobot,
        MirrorKind::Nekoha => ConfigField::MirrorNekoha,
        MirrorKind::Beatconnect => ConfigField::MirrorBeatconnect,
        MirrorKind::Osudl => ConfigField::MirrorOsudl,
        MirrorKind::Catboy => ConfigField::MirrorCatboy,
        MirrorKind::Hinamizawa => ConfigField::MirrorHinamizawa,
        MirrorKind::OsuApi => ConfigField::MirrorOsuOfficial,
        MirrorKind::Nzbasic => ConfigField::MirrorNzbasic,
        MirrorKind::Custom => return None,
    })
}

/// The built-in [`MirrorKind`] a mirror nav field maps to, or `None` for any
/// non-mirror (or custom-mirror) field.
fn mirror_kind_of(field: ConfigField) -> Option<MirrorKind> {
    Some(match field {
        ConfigField::MirrorNerinyan => MirrorKind::Nerinyan,
        ConfigField::MirrorOsuDirect => MirrorKind::OsuDirect,
        ConfigField::MirrorSayobot => MirrorKind::Sayobot,
        ConfigField::MirrorNekoha => MirrorKind::Nekoha,
        ConfigField::MirrorBeatconnect => MirrorKind::Beatconnect,
        ConfigField::MirrorOsudl => MirrorKind::Osudl,
        ConfigField::MirrorCatboy => MirrorKind::Catboy,
        ConfigField::MirrorHinamizawa => MirrorKind::Hinamizawa,
        ConfigField::MirrorOsuOfficial => MirrorKind::OsuApi,
        ConfigField::MirrorNzbasic => MirrorKind::Nzbasic,
        _ => return None,
    })
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
