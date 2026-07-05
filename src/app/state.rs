use super::{
    banner::BannerRecency,
    collection::CollectionPage,
    collection_state::{self, CollectionStateFile},
    config::{AuthLoginState, ConfigField, ConfigTab},
    failed_maps,
    home::{GetMapsSource, HomeField, HomeTab},
    ignored_maps,
    library::LibraryState,
    login::{LoginField, LoginPhase, LoginTab},
    snapshots,
    tab::Tab,
    toast::{Toast, Toasts},
    update_source::{ScanCta, UpdateAction, extract_collection_id},
};
use crate::auto_update::AvailableUpdate;
use crate::{
    config::{
        Config, RetryFailedOnDownload,
        constants::{DISK_CACHE_TTL, STATIC_TABS, TAB_CONFIG_LOWER, TAB_HOME_LOWER},
        save_config,
    },
    download::{
        DownloadConfig, DownloadEvent, DownloadId, DownloadRequest, DownloadStage,
        SelectiveDownloadCollection, SelectiveDownloadRequest,
    },
    utils,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fs2::available_space;
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::debug;

/// Header brand-shimmer ease state. The ramp (0..1) crossfades between idle and
/// downloading: `anchor` is the tick the active ease began, `from` the ramp
/// value then, `to` the target (1.0 downloading, 0.0 idle). Re-anchored from the
/// live value on each state flip so an interrupted fade reverses in place.
#[derive(Clone, Copy)]
struct BrandAnim {
    anchor: u64,
    from: f32,
    to: f32,
}

impl Default for BrandAnim {
    fn default() -> Self {
        Self {
            anchor: 0,
            from: 0.0,
            to: 0.0,
        }
    }
}

pub struct App {
    pub home: HomeTab,
    /// App-global osu! client + library path. Read by the header chip, the
    /// download pipeline, and the library scan; edited through the update
    /// source's path field. Hoisted off `update` so one value backs all readers.
    pub library: LibraryState,
    pub config: ConfigTab,
    /// The login panel. `Some` only while the login split is open on the Config
    /// tab (opened from the auth chip, closed with `esc`/`q` or a tab switch).
    /// Rendered as a focus-trap panel docked on the right of the Config body;
    /// while it is open the config form is frozen and all input routes here.
    pub login: Option<LoginTab>,
    pub downloads: Vec<CollectionPage>,
    /// Transient top-right notifications — results and errors. Ephemeral by
    /// design; durable signals live on banners, inline state, or tab markers.
    pub toasts: Toasts,
    pub active_tab: Tab,
    pub collection_state: CollectionStateFile,
    pub collection_state_path: Option<PathBuf>,
    pub scan_handle: Option<tokio::task::JoinHandle<()>>,
    pub tick_count: u64,
    pub help_open: bool,
    /// Top-row scroll offset for the help overlay. Interior mutability because
    /// `draw()` borrows `App` immutably but clamps the offset to the viewport
    /// and writes the clamped value back (mirrors `disk_cache`).
    pub help_scroll: Cell<usize>,
    /// Edit mode for the focused text-input row. Edit is OFF by default — a
    /// focused text field is selected-not-editing (`❯`, no cursor, keys are
    /// global hotkeys) until `enter` descends into editing (`✎` +
    /// native cursor, keys type). Reset whenever focus moves or tabs switch.
    pub editing: bool,
    /// Vim keymap latch for the `gg` motion: set when a lone `g` is seen,
    /// consumed by the next key (a second `g` jumps to the top, anything else
    /// clears it). Only ever set while `config.vim_keys` is on.
    vim_pending_g: bool,
    /// Pending confirmation for "retry N failed maps?" when count > 50.
    pub confirm_retry: Option<RetryAllConfirmModal>,
    /// Pre-download prompt: previously failed beatmapsets in
    /// `failed-beatmapsets.json` intersect with the collection the user just
    /// submitted. Surfaces only when the config is `Ask`.
    pub confirm_retry_on_start: Option<RetryOnStartModal>,
    /// A newer release detected in notify-only mode (auto-update off). Drives
    /// the footer `u` hint and the update modal; cleared once the user confirms
    /// the apply.
    pub available_update: Option<AvailableUpdate>,
    /// Header self-update indicator phase. Independent of `available_update`
    /// (which is cleared on confirm): this rides the whole apply flow so the
    /// header keeps a live cue through download and the restart-pending wait,
    /// including auto-update, which never populates `available_update`.
    pub update_phase: Option<UpdateIndicator>,
    /// The update-changelog modal, opened with `u` when an update is available.
    pub update_modal: Option<UpdateModal>,
    /// Override for the on-disk failed-maps file, set by tests. Production
    /// callers always pass `None` and the path is resolved at use-site.
    pub(crate) failed_maps_path_override: Option<PathBuf>,
    next_download_id: DownloadId,
    /// Cached disk free-space result: `(checked_at, free_bytes)`. Interior
    /// mutability because `draw()` borrows `App` immutably but must refresh
    /// the cache at most once per `DISK_CACHE_TTL`.
    disk_cache: Cell<Option<(Instant, u64)>>,
    /// Ease state for the header brand shimmer. Crossfades the ramp in when
    /// downloading begins and back out when every download settles. Interior
    /// mutability because `draw()`/`brand_ramp()` advance it under an immutable
    /// borrow.
    brand_anim: Cell<BrandAnim>,
    /// Per-WARNING-condition entry timestamps used to break banner ties by
    /// most-recently-entered (`DiskLow` vs `TooSmall`). Updated under an
    /// immutable borrow during `draw()` (interior mutability mirrors
    /// `disk_cache`).
    pub(crate) banner_recency: BannerRecency,
}

/// Header self-update indicator phase (see [`App::update_phase`]). Each variant
/// swaps the trailing glyph after the current version: `Available` shimmers an
/// `↑`, `Downloading` spins in its place, `RestartPending` shows a static `↻`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateIndicator {
    /// A newer release exists; nothing applied yet (notify-only).
    Available,
    /// The new binary is downloading / installing.
    Downloading,
    /// Installed; a restart applies it.
    RestartPending,
}

#[derive(Debug)]
pub enum AppCommand {
    StartDownload {
        id: DownloadId,
        request: DownloadRequest,
    },
    StartSelectiveDownload {
        id: DownloadId,
        request: SelectiveDownloadRequest,
    },
    CancelDownload {
        id: DownloadId,
    },
    /// Defer (requeue) every map of this download currently waiting on a
    /// rate-limit cooldown, so it retries once a mirror frees instead of being
    /// dropped (`s`).
    DeferRateLimited {
        id: DownloadId,
    },
    /// Hard-drop every map of this download currently waiting on a rate-limit
    /// cooldown (`S`).
    SkipRateLimited {
        id: DownloadId,
    },
    /// Start an osu!lazer password (ROPC) login with the entered credentials.
    LazerLogin {
        username: String,
        password: String,
    },
    /// Submit the session-verification code osu! emailed for a new device.
    SubmitVerification {
        code: String,
    },
    /// Ask osu! to re-send the session-verification code.
    ReissueVerification,
    CancelLogin,
    Logout,
    ScanLocalDatabase,
    RecheckFailedMaps,
    /// Retry all retryable failed maps for a download page (excludes NotFound).
    RetryAllFailed {
        download_id: DownloadId,
    },
    /// Collection URL field changed; schedule a debounced metadata resolve.
    ResolveCollectionUrl {
        value: String,
    },
    /// Probe latency for all built-in mirrors.
    ProbeMirrors,
    /// Switch to the home tab and focus the output directory field.
    /// Triggered by the disk-low / disk-full banner action.
    FocusOutputDir,
    /// Confirm the update modal: download and apply the available update.
    StartUpdate,
    Quit,
}

/// State for the "retry N failed maps?" confirm modal shown when `R` is pressed
/// with more than 50 retryable failures.
///
/// Buttons (left→right): `cancel`, `retry`. `focus` is the index of the selected
/// button; ←/→ move it, `enter` activates it, `esc` cancels.
#[derive(Debug)]
pub struct RetryAllConfirmModal {
    pub download_id: DownloadId,
    pub retryable_count: usize,
    /// Selected button index into [`CONFIRM_RETRY_BUTTONS`].
    pub focus: usize,
}

/// State for the pre-download retry prompt. Surfaces under `Ask` when
/// previously failed beatmaps for this collection are persisted on disk.
///
/// Buttons (left→right): `cancel`, `skip`, `retry`. ←/→ move `focus`, `enter`
/// activates the focused button — `retry` dispatches with
/// `include_previously_failed = true`, `skip` dispatches with it `false`, and
/// `cancel` (or `esc`) discards the queued download.
#[derive(Debug)]
pub struct RetryOnStartModal {
    pub id: DownloadId,
    pub failed_count: usize,
    pub pending: DownloadRequest,
    /// Selected button index into [`RETRY_ON_START_BUTTONS`].
    pub focus: usize,
}

/// Button labels for [`RetryOnStartModal`], left→right.
pub const RETRY_ON_START_BUTTONS: [&str; 3] = ["cancel", "skip", "retry"];
/// Default-focused button: `retry` (the recommended, non-destructive action).
pub const RETRY_ON_START_DEFAULT_FOCUS: usize = 2;

/// Button labels for [`RetryAllConfirmModal`], left→right.
pub const CONFIRM_RETRY_BUTTONS: [&str; 2] = ["cancel", "retry"];
/// Default-focused button: `retry`.
pub const CONFIRM_RETRY_DEFAULT_FOCUS: usize = 1;

/// State for the update-changelog modal (`u`). The changelog text lives on
/// [`App::available_update`]; this carries only the button focus and the scroll
/// offset (interior-mutable so the renderer can clamp it under `&App`).
pub struct UpdateModal {
    /// Selected button index into [`UPDATE_MODAL_BUTTONS`].
    pub focus: usize,
    /// Top-row scroll offset for the changelog body.
    pub scroll: Cell<usize>,
}

/// Button labels for [`UpdateModal`], left→right.
pub const UPDATE_MODAL_BUTTONS: [&str; 2] = ["later", "update"];
/// Default-focused button: `update`.
pub const UPDATE_MODAL_DEFAULT_FOCUS: usize = 1;

impl App {
    pub fn new(config: Config) -> Self {
        let state_path = collection_state::state_path();
        let coll_state = state_path
            .as_deref()
            .map(collection_state::load)
            .unwrap_or_default();
        Self {
            home: HomeTab::new(&config),
            library: LibraryState::from_config(&config),
            config: ConfigTab::new(&config),
            login: None,
            downloads: Vec::new(),
            toasts: Toasts::default(),
            active_tab: Tab::Home,
            collection_state: coll_state,
            collection_state_path: state_path,
            scan_handle: None,
            tick_count: 0,
            help_open: false,
            help_scroll: Cell::new(0),
            editing: false,
            vim_pending_g: false,
            confirm_retry: None,
            confirm_retry_on_start: None,
            available_update: None,
            update_phase: None,
            update_modal: None,
            failed_maps_path_override: None,
            next_download_id: 1,
            disk_cache: Cell::new(None),
            brand_anim: Cell::new(BrandAnim::default()),
            banner_recency: BannerRecency::default(),
        }
    }

    pub fn active_tab(&self) -> Tab {
        self.active_tab
    }

    /// Whether the osu! official mirror can be enabled — only with a stored
    /// `*`-scope login. Drives the greyed-out toggle and the "log in first"
    /// notice; the toggle is inert until this is `true`.
    pub fn osu_official_unlocked(&self) -> bool {
        matches!(self.config.login_state, AuthLoginState::LoggedIn)
    }

    /// When the focused field is the osu! official mirror toggle and the user is
    /// logged out, surface a "log in first" toast and return `true` so the caller
    /// skips the toggle. The mirror cannot be enabled without a `*` token.
    fn block_osu_official_if_logged_out(&mut self) -> bool {
        let focused_osu_official =
            self.active_tab() == Tab::Config && self.config.focus == ConfigField::MirrorOsuOfficial;
        if focused_osu_official && !self.osu_official_unlocked() {
            self.push_toast(
                Toast::warning("log in to enable the osu! official mirror")
                    .with_detail("open login from the config auth chip"),
            );
            return true;
        }
        false
    }

    /// Whether any download page is in a non-terminal stage (preparing,
    /// resolving, rechecking, or downloading). Drives the header brand
    /// animation, which idles once every page reaches `Completed`/`Failed`.
    pub fn is_downloading(&self) -> bool {
        self.downloads
            .iter()
            .any(|p| !matches!(p.stage, DownloadStage::Completed | DownloadStage::Failed))
    }

    /// Eased ramp (0..1) for the header brand shimmer. Eases toward 1 over
    /// `BRAND_EASE_TICKS` (smoothstep) while downloading and back toward 0 over
    /// the same span once every download settles, so the shimmer fades in *and*
    /// out instead of snapping. Re-anchors from the live value on each state
    /// flip, so a stop mid-fade-in reverses smoothly rather than jumping. Read
    /// (and advanced) under `draw()`'s immutable borrow.
    pub fn brand_ramp(&self) -> f32 {
        // ~0.8s at the 50ms tick rate (see `tui::terminal`).
        const BRAND_EASE_TICKS: f32 = 16.0;
        let target = if self.is_downloading() { 1.0 } else { 0.0 };
        let mut anim = self.brand_anim.get();

        let progress =
            (self.tick_count.saturating_sub(anim.anchor) as f32 / BRAND_EASE_TICKS).clamp(0.0, 1.0);
        // smoothstep — a gentler start/stop than a linear ramp.
        let smooth = progress * progress * (3.0 - 2.0 * progress);
        let current = anim.from + (anim.to - anim.from) * smooth;

        // State flipped: re-anchor the ease at the live value so it reverses in
        // place (mid-fade-in stop eases back down from the partial value).
        if anim.to != target {
            anim = BrandAnim {
                anchor: self.tick_count,
                from: current,
                to: target,
            };
            self.brand_anim.set(anim);
        }
        current
    }

    /// Free bytes on the filesystem of the first active download's output
    /// directory, falling back to the home tab's configured directory, then to
    /// the OS home directory. Result is cached for `DISK_CACHE_TTL` to avoid
    /// per-frame syscalls.
    pub fn disk_free_bytes(&self) -> Option<u64> {
        let now = Instant::now();
        if let Some((checked_at, free)) = self.disk_cache.get()
            && now.duration_since(checked_at) < DISK_CACHE_TTL
        {
            return Some(free);
        }

        let typed_dir = self.home.directory.value.trim();
        let path: Option<PathBuf> = self
            .downloads
            .iter()
            .find_map(|p| p.output_dir.as_deref().map(PathBuf::from))
            .or_else(|| {
                if typed_dir.is_empty() {
                    dirs::home_dir()
                } else {
                    Some(PathBuf::from(utils::expand_tilde(typed_dir)))
                }
            });

        let free = available_space(path.as_deref()?).ok()?;
        self.disk_cache.set(Some((now, free)));
        Some(free)
    }

    pub fn next_tab(&mut self) -> Option<AppCommand> {
        // Commit a mid-edit config field before the tab (and its focus) changes.
        self.commit_field_edit();
        // Switching tabs closes the login split (it lives only on Config).
        self.close_login();
        let total = self.total_tabs();
        self.active_tab = Tab::from_index((self.active_tab.to_index() + 1) % total);
        self.editing = false;
        self.probe_on_home_activation()
    }

    pub fn prev_tab(&mut self) -> Option<AppCommand> {
        self.commit_field_edit();
        self.close_login();
        let total = self.total_tabs();
        let idx = self.active_tab.to_index();
        self.active_tab = Tab::from_index(if idx == 0 { total - 1 } else { idx - 1 });
        self.editing = false;
        self.probe_on_home_activation()
    }

    /// Switch to the home tab and place focus on the directory field.
    /// Called when the user activates the disk-low or disk-full banner action.
    /// The field lands selected-not-editing — `enter` starts editing.
    pub fn focus_output_dir(&mut self) {
        self.active_tab = Tab::Home;
        self.home.focus = HomeField::Directory;
        self.editing = false;
    }

    /// Probe mirror latency once when switching to Home with no results yet;
    /// existing pings persist across tab switches (manual `r` forces a fresh
    /// probe). The update source's scan is explicit (its CTA), never on switch.
    fn probe_on_home_activation(&mut self) -> Option<AppCommand> {
        if self.active_tab == Tab::Home && self.home.mirror_latency.is_empty() {
            Some(AppCommand::ProbeMirrors)
        } else {
            None
        }
    }

    /// Whether the Get Maps update source is in its two-pane browse (descended)
    /// state, so browse keys own input instead of form-field nav / tab switch.
    fn update_browsing(&self) -> bool {
        self.active_tab() == Tab::Home
            && self.home.source == GetMapsSource::Update
            && self.home.update.is_browsing()
    }

    /// Whether the update source's osu! path field is the focused, editable
    /// text input (its value lives on [`App::library`], so its text ops route
    /// there rather than through `HomeTab`).
    fn home_update_path_editing(&self) -> bool {
        self.active_tab() == Tab::Home
            && self.home.source == GetMapsSource::Update
            && self.home.focus == HomeField::UpdateOsuPath
            && !self.home.update.is_browsing()
    }

    /// Jump from the Get Maps mirrors summary to the Config tab's mirrors
    /// section (the sole mirror editor), focusing the first built-in mirror row.
    fn open_config_mirrors(&mut self) {
        self.active_tab = Tab::Config;
        self.config.focus_mirrors();
        self.editing = false;
    }

    /// Closes the topmost open modal. Returns `true` if one was closed.
    /// `esc` and `q` call this before falling through to the quit flow.
    /// Extend this as new modal types are added.
    fn close_modal(&mut self) -> bool {
        if self.confirm_retry_on_start.is_some() {
            self.cancel_retry_on_start();
            return true;
        }
        if self.confirm_retry.is_some() {
            self.confirm_retry = None;
            return true;
        }
        if self.update_modal.is_some() {
            self.update_modal = None;
            return true;
        }
        if self.help_open {
            self.close_help();
            return true;
        }
        false
    }

    /// Close the help overlay and reset its scroll offset.
    fn close_help(&mut self) {
        self.help_open = false;
        self.help_scroll.set(0);
    }

    /// Whether any modal is currently blocking input.
    pub fn any_modal_open(&self) -> bool {
        self.help_open
            || self.confirm_retry.is_some()
            || self.confirm_retry_on_start.is_some()
            || self.update_modal.is_some()
    }

    /// Cancel a pending pre-download retry prompt. Drops the queued page that
    /// `request_download` allocated for the prospective download so the tab
    /// list returns to its prior shape.
    fn cancel_retry_on_start(&mut self) {
        let Some(modal) = self.confirm_retry_on_start.take() else {
            return;
        };
        self.discard_pending_download(modal);
    }

    /// Drop the queued page allocated for an already-taken pre-download retry
    /// prompt. Shared by the `esc`/`cancel`-button paths after they have removed
    /// the modal from `confirm_retry_on_start`.
    fn discard_pending_download(&mut self, modal: RetryOnStartModal) {
        self.remove_download_page(modal.id);
        self.active_tab = Tab::Home;
        self.toast_info("download cancelled");
    }

    /// If a config text field was mid-edit, persist it before edit mode drops.
    /// Text-field edits apply immediately, so every path that leaves edit mode
    /// must commit first — enter/esc do this inline; field-nav and tab-switch
    /// route through here. Config rows flush to disk; the updates osu! path
    /// persists to `[recent]`.
    fn commit_field_edit(&mut self) {
        if !self.editing {
            return;
        }
        if self.active_tab() == Tab::Config
            && self.login.is_none()
            && self.config.focus.is_text_input()
        {
            self.apply_config_change();
        } else if self.home_update_path_editing() {
            self.persist_osu_path_inputs();
        }
    }

    fn focus_next_field(&mut self) {
        self.commit_field_edit();
        self.editing = false;
        if let Some(login) = self.login.as_mut() {
            login.next_field();
            return;
        }
        match self.active_tab() {
            Tab::Home => self.home.next_field(),
            Tab::Config => self.config.next_field(),
            _ => {}
        }
    }

    fn focus_prev_field(&mut self) {
        self.commit_field_edit();
        self.editing = false;
        if let Some(login) = self.login.as_mut() {
            login.prev_field();
            return;
        }
        match self.active_tab() {
            Tab::Home => self.home.prev_field(),
            Tab::Config => self.config.prev_field(),
            _ => {}
        }
    }

    fn focus_first_field(&mut self) {
        self.commit_field_edit();
        self.editing = false;
        if let Some(login) = self.login.as_mut() {
            login.first_field();
            return;
        }
        match self.active_tab() {
            Tab::Home => self.home.first_field(),
            Tab::Config => self.config.first_field(),
            _ => {}
        }
    }

    fn focus_last_field(&mut self) {
        self.commit_field_edit();
        self.editing = false;
        if let Some(login) = self.login.as_mut() {
            login.last_field();
            return;
        }
        match self.active_tab() {
            Tab::Home => self.home.last_field(),
            Tab::Config => self.config.last_field(),
            _ => {}
        }
    }

    /// `gg` / Home: jump the active surface to its top — an open list cursor to
    /// the first row, a download page to the top, else field focus to the first
    /// field. Mirrors the per-tab branching of the `Up`/`Down` handler.
    fn jump_top(&mut self) {
        if self.update_browsing() {
            self.home.update.scroll_to_edge(true);
        } else if let Some(page) = self.active_download_page_mut() {
            page.jump_top();
        } else {
            self.focus_first_field();
        }
    }

    /// `G` / End: jump the active surface to its bottom.
    fn jump_bottom(&mut self) {
        if self.update_browsing() {
            self.home.update.scroll_to_edge(false);
        } else if let Some(page) = self.active_download_page_mut() {
            page.jump_bottom();
        } else {
            self.focus_last_field();
        }
    }

    /// `Ctrl+u` / PageUp: page the active list up. Forms have no page, so they
    /// jump to the first field.
    fn page_up(&mut self) {
        if self.update_browsing() {
            self.home.update.page_up();
        } else if let Some(page) = self.active_download_page_mut() {
            page.page_up();
        } else {
            self.focus_first_field();
        }
    }

    /// `Ctrl+d` / PageDown: page the active list down.
    fn page_down(&mut self) {
        if self.update_browsing() {
            self.home.update.page_down();
        } else if let Some(page) = self.active_download_page_mut() {
            page.page_down();
        } else {
            self.focus_last_field();
        }
    }

    /// Persist the current config form to disk and apply any live-effect change
    /// (theme) immediately. Config-tab edits have no save step, so this runs
    /// after every settled change.
    ///
    /// Invalid input (e.g. all mirrors disabled, a malformed custom URL) surfaces
    /// an error toast and leaves the on-disk config untouched. A successful apply
    /// is silent — the UI already reflects the change.
    fn apply_config_change(&mut self) {
        let mut new_config = match self.config.build_config() {
            Ok(config) => config,
            Err(err) => {
                self.toast_err(err);
                return;
            }
        };
        // A download may have refreshed the last-used inputs after this tab was
        // loaded; keep the freshest on-disk `recent` so frequent auto-saves never
        // revert the prefill to the load-time snapshot.
        new_config.recent = crate::config::load_config_or_default().recent;
        if let Err(err) = new_config.validate() {
            self.toast_err(err.to_string());
            return;
        }
        match save_config(&new_config) {
            Ok(_) => {
                // Theme is the only setting with a visible-now effect; swap the
                // live palette so the change shows without a relaunch.
                crate::tui::apply_theme(new_config.display.theme);
                // The Config tab is the sole mirror editor; push its saved mirror
                // settings into the Get Maps tab so the enabled-count and the
                // download list track the change without a relaunch.
                self.home.sync_mirrors_from_config(&new_config.mirror);
                self.config.loaded_config = new_config;
            }
            Err(err) => self.toast_err(err.to_string()),
        }
    }

    /// Whether a login / verification request is currently in flight.
    fn login_in_flight(&self) -> bool {
        matches!(self.config.login_state, AuthLoginState::InProgress(_))
    }

    /// Enter on the login panel. Text-input rows toggle edit mode in the caller
    /// (via `focused_text_input`); this handles the action chips.
    fn login_enter(&mut self) -> Option<AppCommand> {
        match self.login.as_ref()?.focus {
            LoginField::Submit => self.login_chip_enter(),
            LoginField::Resend => self.request_reissue(),
            _ => None,
        }
    }

    /// Enter on the login panel's primary action chip. Cancels a running request,
    /// else branches on the phase: log in / verify / log out.
    fn login_chip_enter(&mut self) -> Option<AppCommand> {
        if self.login_in_flight() {
            // Drop the in-progress state (phase is preserved, so cancelling a
            // mid-verification request keeps the code field) and abort the task.
            self.config.set_login_failed();
            self.toast_info("login cancelled");
            return Some(AppCommand::CancelLogin);
        }
        match self.login.as_ref()?.phase {
            LoginPhase::Credentials => self.request_lazer_login(),
            LoginPhase::NeedsVerification => self.request_verification(),
            LoginPhase::LoggedIn => self.request_logout(),
        }
    }

    /// Start the lazer password grant from the entered username + password.
    fn request_lazer_login(&mut self) -> Option<AppCommand> {
        let (username, password) = {
            let login = self.login.as_ref()?;
            (
                login.username.value.trim().to_string(),
                login.password.value.clone(),
            )
        };
        if username.is_empty() || password.is_empty() {
            self.toast_warn("enter your osu! username and password");
            return None;
        }
        // Wipe the password from the field the moment it is handed off, so the
        // secret never lingers in the UI buffer.
        if let Some(login) = self.login.as_mut() {
            login.clear_password();
        }
        self.config.set_loading("signing in…");
        Some(AppCommand::LazerLogin { username, password })
    }

    /// Submit the entered session-verification code.
    fn request_verification(&mut self) -> Option<AppCommand> {
        let code = self.login.as_ref()?.code.value.trim().to_string();
        if code.is_empty() {
            self.toast_warn("enter the verification code");
            return None;
        }
        self.config.set_loading("verifying…");
        Some(AppCommand::SubmitVerification { code })
    }

    /// Ask osu! to re-send the verification code (no login-state change).
    fn request_reissue(&mut self) -> Option<AppCommand> {
        if self.login_in_flight() {
            return None;
        }
        self.toast_info("resending code…");
        Some(AppCommand::ReissueVerification)
    }

    fn request_logout(&mut self) -> Option<AppCommand> {
        match self.config.login_state {
            AuthLoginState::LoggedIn => {
                self.config.set_loading("logging out…");
                Some(AppCommand::Logout)
            }
            AuthLoginState::LoggedOut => {
                self.toast_info("already logged out");
                None
            }
            AuthLoginState::InProgress(_) => None,
        }
    }

    fn total_tabs(&self) -> usize {
        STATIC_TABS + self.downloads.len()
    }

    /// Whether the login split is open. While it is, the active tab stays on
    /// Config, the config form is frozen, and all field input routes to the
    /// login panel (a focus trap).
    pub fn login_open(&self) -> bool {
        self.login.is_some()
    }

    /// Open the login split on the Config tab (or leave it open if already
    /// shown). The active tab stays on Config; the phase is seeded from the
    /// current login state so a logged-in user lands on the account view rather
    /// than the credentials form.
    fn open_login(&mut self) {
        if self.login.is_none() {
            let logged_in = matches!(self.config.login_state, AuthLoginState::LoggedIn);
            self.login = Some(LoginTab::new(logged_in));
        }
        self.editing = false;
    }

    /// Close the login split and hand focus back to the config auth chip (which
    /// stays focused underneath while the panel is open). An in-flight login
    /// keeps running — its result updates the chip in the background.
    fn close_login(&mut self) {
        if self.login.take().is_none() {
            return;
        }
        self.editing = false;
        self.config.focus = ConfigField::AuthChip;
    }

    /// Persist the current collection and download-directory inputs to the
    /// config so the next launch pre-fills them. Reads the on-disk config first
    /// so unsaved config-tab edits are never clobbered; failures are silent —
    /// a missed prefill must not block a download.
    fn persist_recent_inputs(&self) {
        let mut config = crate::config::load_config_or_default();
        let collection = self.home.collection.value.trim();
        config.recent.collection = (!collection.is_empty()).then(|| collection.to_string());
        let directory = self.home.persisted_directory();
        config.recent.directory = (!directory.is_empty()).then(|| directory.to_string());
        let _ = save_config(&config);
    }

    /// Persist the updates-tab osu! client kind and path so the next launch
    /// restores them instead of re-detecting. Reads the on-disk config first so
    /// unsaved config-tab edits are never clobbered; failures are silent.
    fn persist_osu_path_inputs(&self) {
        let mut config = crate::config::load_config_or_default();
        config.recent.osu_client = Some(self.library.client_type);
        let path = self.library.osu_path.value.trim();
        config.recent.osu_path = (!path.is_empty()).then(|| path.to_string());
        let _ = save_config(&config);
    }

    /// Mark beatmapsets as installed: persist them to the ignore list and drop
    /// them from the missing list at once. A later scan that detects a genuine
    /// install auto-clears the entry (see `ignored_maps::reconcile_installed`).
    fn mark_installed(&mut self, ids: Vec<u32>) {
        if ids.is_empty() {
            return;
        }
        let ids: HashSet<u32> = ids.into_iter().collect();
        if let Some(path) = ignored_maps::ignored_maps_path() {
            ignored_maps::record_ignored(&path, ids.iter().copied());
        }
        let count = ids.len();
        self.home.update.hide_missing(&ids);
        self.toast_ok(format!(
            "marked {count} beatmapset{} installed",
            if count == 1 { "" } else { "s" }
        ));
    }

    /// Letter-key dispatch while browsing the update source's two panes. `a`/`A`
    /// select-all/none (list pane), `s` cycles the focused pane's sort, `i`/`I`
    /// mark the preview's focused row / whole collection installed, and `r`
    /// rechecks known-bad maps. Only `r` yields a command; the rest mutate in
    /// place.
    fn handle_update_browse_char(&mut self, ch: char) -> Option<AppCommand> {
        let list_focused = !self.home.update.preview_focused();
        match ch {
            'a' if list_focused => self.home.update.set_all_collections_selected(true),
            'A' if list_focused => self.home.update.set_all_collections_selected(false),
            's' => {
                if self.home.update.preview_focused() {
                    self.home.update.cycle_preview_sort();
                } else {
                    self.home.update.cycle_collection_sort();
                }
            }
            'i' if self.home.update.preview_focused() => {
                let ids = self.home.update.preview_focused_id();
                self.mark_installed(ids);
            }
            'I' if self.home.update.preview_focused() => {
                let ids = self.home.update.highlighted_collection_missing_ids();
                self.mark_installed(ids);
            }
            'r' if self.home.update.can_recheck_failed_maps() => {
                return Some(AppCommand::RecheckFailedMaps);
            }
            _ => {}
        }
        None
    }

    pub fn request_download(&mut self) -> Option<(DownloadId, DownloadRequest)> {
        let mut request = match self.home.build_request(
            self.config.archive_validation,
            self.config.auto_skip_rate_limited,
            self.config.parse_rate_limit_skip_secs().unwrap_or(60),
        ) {
            Ok(request) => request,
            Err(err) => {
                self.toast_err(err);
                return None;
            }
        };

        // Cheap, sync inputs for the pre-skip of already-imported sets. The
        // owned-id set itself is resolved off the UI thread in the pipeline task
        // (`run_collection`), so the blocking db read never stalls the event loop.
        // Source mirrors the Updates-tab scan (seeded from `[recent]`).
        request.skip_already_imported = self.config.skip_already_imported;
        request.osu_client = self.library.client_type;
        request.osu_path = self.library.osu_path();

        if self.downloads.len() >= usize::MAX - 1 {
            self.toast_err("too many downloads queued");
            return None;
        }

        self.persist_recent_inputs();

        let collection_id = utils::parse_collection_id(request.collection_input.trim()).ok();
        let failed_count = collection_id
            .map(|id| self.previously_failed_count(id))
            .unwrap_or(0);

        // No prior failures for this collection — skip the modal entirely.
        if failed_count == 0 {
            return Some(self.queue_download(request));
        }

        match self.config.retry_failed_on_download {
            RetryFailedOnDownload::Yes => {
                request.include_previously_failed = true;
                Some(self.queue_download(request))
            }
            RetryFailedOnDownload::No => {
                request.include_previously_failed = false;
                Some(self.queue_download(request))
            }
            RetryFailedOnDownload::Ask => {
                let id = self.next_download_id;
                self.next_download_id += 1;
                self.confirm_retry_on_start = Some(RetryOnStartModal {
                    id,
                    failed_count,
                    pending: request,
                    focus: RETRY_ON_START_DEFAULT_FOCUS,
                });
                None
            }
        }
    }

    /// Allocate a `CollectionPage` for `request` and return the id + request
    /// to dispatch to the pipeline.
    fn queue_download(&mut self, request: DownloadRequest) -> (DownloadId, DownloadRequest) {
        let id = self.next_download_id;
        self.next_download_id += 1;
        self.push_pending_page(id, &request);
        self.toast_ok(format!("queued download #{id}"));
        (id, request)
    }

    /// Allocate a `CollectionPage` for an id reserved earlier by the retry
    /// prompt and dispatch the queued download.
    fn dispatch_pending(
        &mut self,
        id: DownloadId,
        request: DownloadRequest,
    ) -> (DownloadId, DownloadRequest) {
        self.push_pending_page(id, &request);
        self.toast_ok(format!("queued download #{id}"));
        (id, request)
    }

    fn push_pending_page(&mut self, id: DownloadId, request: &DownloadRequest) {
        let placeholder_title = Self::placeholder_title(&request.collection_input, id);
        let concurrent = usize::from(request.config.concurrent.max(1));
        let mut page = CollectionPage::new(id, placeholder_title, concurrent);
        page.stage = DownloadStage::Resolving;
        page.download_config = Some(request.config.clone());
        self.downloads.push(page);
        self.active_tab = Tab::Download(self.downloads.len() - 1);
    }

    /// Count beatmaps in `failed-beatmapsets.json` that belong to
    /// `collection_id`. The persisted file is not collection-scoped, so we
    /// pull the resolved id list from the `HomeTab` auto-resolve cache and
    /// intersect.
    ///
    /// Returns 0 when:
    /// - the failed-maps file path is unavailable, OR
    /// - no resolved collection metadata is cached for `collection_id` (the
    ///   user hit `enter` before the 300 ms debounce fired). Suppressing the
    ///   prompt in that case matches "no prior context to compare" — the
    ///   pipeline will retry persisted failures in its normal flow.
    fn previously_failed_count(&self, collection_id: u32) -> usize {
        let path = self
            .failed_maps_path_override
            .clone()
            .or_else(failed_maps::failed_maps_path);
        let Some(path) = path else { return 0 };

        let Some((cached_id, ids)) = self.home.resolved_collection.as_ref() else {
            return 0;
        };
        if *cached_id != collection_id {
            return 0;
        }

        let resolved_set: HashSet<u32> = ids.iter().copied().collect();
        intersect_failed_ids(&path, &resolved_set).len()
    }

    pub fn request_selective_download(&mut self) -> Option<(DownloadId, SelectiveDownloadRequest)> {
        let beatmapset_ids = self.home.update.selected_beatmapset_ids();
        if beatmapset_ids.is_empty() {
            self.report_scan_error("no beatmaps selected for download");
            return None;
        }

        let collection_ids: Vec<u32> = self
            .home
            .update
            .selected_collection_ids()
            .into_iter()
            .filter_map(|id| u32::try_from(id).ok())
            .collect();

        if collection_ids.is_empty() {
            self.report_scan_error("no collections available");
            return None;
        }

        let mirrors = self.home.build_mirror_list();
        if mirrors.is_empty() {
            self.report_scan_error("no mirrors selected (configure in the home tab)");
            return None;
        }

        let concurrent = self.home.resolved_threads();

        let directory = if self.home.directory.value.trim().is_empty() {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".to_string())
        } else {
            utils::expand_tilde(self.home.directory.value.trim())
        };

        if self.downloads.len() >= usize::MAX - 1 {
            self.report_scan_error("too many downloads queued");
            return None;
        }

        let id = self.next_download_id;
        self.next_download_id += 1;

        let placeholder_title = if collection_ids.len() == 1 {
            format!("Update #{}", collection_ids[0])
        } else {
            format!("Update ({} collections)", collection_ids.len())
        };

        let concurrent_usize = usize::from(concurrent.max(1));
        let mut page = CollectionPage::new(id, placeholder_title, concurrent_usize);
        page.stage = DownloadStage::Resolving;
        // config is stored after it is built below; we'll set it there
        self.downloads.push(page);
        self.active_tab = Tab::Download(self.downloads.len() - 1);

        self.push_toast(
            Toast::success(format!("queued update download #{id}"))
                .with_detail(format!("{} beatmaps", beatmapset_ids.len())),
        );

        let config = DownloadConfig {
            directory,
            mirrors,
            concurrent,
            archive_validation: self.config.archive_validation,
            auto_skip_rate_limited: self.config.auto_skip_rate_limited,
            rate_limit_skip_secs: self.config.parse_rate_limit_skip_secs().unwrap_or(60),
        };

        let current_snapshots = snapshots::current_snapshots(
            self.library.client_type,
            &self.home.update.scan.local_collections_raw,
            self.home.update.scan.local_beatmapsets.iter(),
            |name| extract_collection_id(name).and_then(|id| u32::try_from(id).ok()),
        );
        let snapshots: Vec<_> = collection_ids
            .iter()
            .filter_map(|collection_id| current_snapshots.get(collection_id).cloned())
            .collect();
        let collections = collection_ids
            .iter()
            .map(|collection_id| SelectiveDownloadCollection {
                id: *collection_id,
                name: snapshots
                    .iter()
                    .find(|snapshot| snapshot.collection_id.parse::<u32>() == Ok(*collection_id))
                    .map(|snapshot| snapshot.name.clone())
                    .unwrap_or_default(),
                beatmapset_ids: self
                    .home
                    .update
                    .selection
                    .cached_missing_sets
                    .iter()
                    .filter(|beatmap| beatmap.collection_id == *collection_id)
                    .map(|beatmap| beatmap.id)
                    .collect(),
            })
            .collect();

        // store config snapshot for potential retry
        if let Some(page) = self.downloads.last_mut() {
            page.download_config = Some(config.clone());
        }

        let request = SelectiveDownloadRequest {
            collection_ids,
            beatmapset_ids,
            collections,
            config,
            snapshot_dir: snapshots::snapshots_dir(),
            snapshots,
        };

        Some((id, request))
    }

    /// Run `mutate` against the home form, then — only when focus is the
    /// collection field AND its value actually changed — return a
    /// `ResolveCollectionUrl` command carrying the new value.
    ///
    /// No-op keystrokes (backspace on an empty field, digits typed into the
    /// threads input) thus do not spawn a wasted resolve task.
    fn mutate_collection_then_resolve(
        &mut self,
        mutate: impl FnOnce(&mut HomeTab),
    ) -> Option<AppCommand> {
        let before = if self.home.focus == HomeField::Collection {
            Some(self.home.collection.value.clone())
        } else {
            None
        };
        mutate(&mut self.home);
        let before = before?;
        if self.home.focus != HomeField::Collection || self.home.collection.value == before {
            return None;
        }
        Some(AppCommand::ResolveCollectionUrl {
            value: self.home.collection.value.clone(),
        })
    }

    /// Whether the focused field on the active tab is an editable text input.
    /// Drives keybind suppression (so `q`/`?`/`x` type instead of acting) and
    /// caret rendering.
    fn focused_text_input(&self) -> bool {
        // While the help overlay is up, keys act on it, not the background field.
        if self.help_open {
            return false;
        }
        // The login split traps focus while open, regardless of active tab.
        if let Some(login) = self.login.as_ref() {
            return login.focus.is_text_input();
        }
        match self.active_tab() {
            // While browsing the update source its keys own input, so no field is
            // a text input even if `home.focus` still names one.
            Tab::Home => !self.update_browsing() && self.home.focus.is_text_input(),
            Tab::Config => self.config.focus.is_text_input(),
            _ => false,
        }
    }

    /// Move the caret one char in the focused text field. Caret moves never
    /// re-resolve the collection (the value is unchanged), so unlike typing they
    /// return no command.
    fn caret_left_focused(&mut self) {
        if let Some(login) = self.login.as_mut() {
            login.caret_left();
            return;
        }
        match self.active_tab() {
            Tab::Home if self.home_update_path_editing() => self.library.caret_left(),
            Tab::Home => self.home.caret_left(),
            Tab::Config => self.config.caret_left(),
            _ => {}
        }
    }

    fn caret_right_focused(&mut self) {
        if let Some(login) = self.login.as_mut() {
            login.caret_right();
            return;
        }
        match self.active_tab() {
            Tab::Home if self.home_update_path_editing() => self.library.caret_right(),
            Tab::Home => self.home.caret_right(),
            Tab::Config => self.config.caret_right(),
            _ => {}
        }
    }

    fn caret_home_focused(&mut self) {
        if let Some(login) = self.login.as_mut() {
            login.caret_home();
            return;
        }
        match self.active_tab() {
            Tab::Home if self.home_update_path_editing() => self.library.caret_home(),
            Tab::Home => self.home.caret_home(),
            Tab::Config => self.config.caret_home(),
            _ => {}
        }
    }

    fn caret_end_focused(&mut self) {
        if let Some(login) = self.login.as_mut() {
            login.caret_end();
            return;
        }
        match self.active_tab() {
            Tab::Home if self.home_update_path_editing() => self.library.caret_end(),
            Tab::Home => self.home.caret_end(),
            Tab::Config => self.config.caret_end(),
            _ => {}
        }
    }

    /// Delete the char at the caret in the focused text field (`Delete` key).
    /// On the home collection field this re-resolves, matching backspace.
    fn delete_forward_focused(&mut self) -> Option<AppCommand> {
        if let Some(login) = self.login.as_mut() {
            login.delete_forward();
            return None;
        }
        match self.active_tab() {
            Tab::Home if self.home_update_path_editing() => {
                self.library.delete_forward();
                None
            }
            Tab::Home => self.mutate_collection_then_resolve(HomeTab::delete_forward),
            Tab::Config => {
                self.config.delete_forward();
                None
            }
            _ => None,
        }
    }

    /// Delete the previous word from the focused text field (alt/ctrl+backspace,
    /// ctrl+w). No-op when focus is not on a text input. On the home collection
    /// field this re-resolves, matching plain backspace.
    fn backspace_word_focused(&mut self) -> Option<AppCommand> {
        if let Some(login) = self.login.as_mut() {
            login.backspace_word();
            return None;
        }
        match self.active_tab() {
            Tab::Home if self.home_update_path_editing() => {
                self.library.backspace_word();
                None
            }
            Tab::Home => self.mutate_collection_then_resolve(HomeTab::backspace_word),
            Tab::Config => {
                self.config.backspace_word();
                None
            }
            _ => None,
        }
    }

    pub fn handle_key(&mut self, mut key: KeyEvent) -> Option<AppCommand> {
        // ctrl+c always quits unconditionally
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(AppCommand::Quit);
        }

        // ctrl+w and ctrl+backspace delete the previous word in the focused text
        // field (no-op elsewhere). Many terminals send ctrl+backspace as ^H
        // (ctrl+h), so both are intercepted early — otherwise they type 'w'/'h'.
        if self.editing
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('w') | KeyCode::Char('h'))
        {
            return self.backspace_word_focused();
        }

        // Opt-in vim normal-layer remap: rewrite the key into its existing
        // arrow/Home/End/Page equivalent before any handler sees it, so every
        // downstream branch (tabs, modals, help, lists) works unchanged. Active
        // only while the keymap is on AND we are not editing a text field — so
        // insert-mode typing stays literal. ctrl+w/ctrl+h are consumed above, so
        // a bare `h` here is always the motion key.
        if self.config.vim_keys && !(self.editing && self.focused_text_input()) {
            let plain = !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
            // Any key resolves the pending `g`; only a second `g` forms `gg`.
            let pending_g = std::mem::take(&mut self.vim_pending_g);
            if pending_g && plain && key.code == KeyCode::Char('g') {
                key.code = KeyCode::Home;
            } else if plain {
                match key.code {
                    KeyCode::Char('h') => key.code = KeyCode::Left,
                    KeyCode::Char('j') => key.code = KeyCode::Down,
                    KeyCode::Char('k') => key.code = KeyCode::Up,
                    KeyCode::Char('l') => key.code = KeyCode::Right,
                    KeyCode::Char('G') => key.code = KeyCode::End,
                    // First `g`: latch and wait for the second to form `gg`.
                    KeyCode::Char('g') => {
                        self.vim_pending_g = true;
                        return None;
                    }
                    // `i`/`a` descend into edit mode on a focused text field.
                    KeyCode::Char('i') | KeyCode::Char('a') if self.focused_text_input() => {
                        self.editing = true;
                        return None;
                    }
                    _ => {}
                }
            } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('d') => key.code = KeyCode::PageDown,
                    KeyCode::Char('u') => key.code = KeyCode::PageUp,
                    _ => {}
                }
            }
        }

        // Pre-download retry prompt: button-driven. ←/→ move the focused button
        // across [cancel, skip, retry]; `enter` activates it; `esc` cancels the
        // download. All other keys are swallowed while the modal is open.
        if self.confirm_retry_on_start.is_some() {
            match key.code {
                KeyCode::Left => {
                    if let Some(m) = self.confirm_retry_on_start.as_mut() {
                        m.focus = m.focus.saturating_sub(1);
                    }
                    return None;
                }
                KeyCode::Right => {
                    if let Some(m) = self.confirm_retry_on_start.as_mut() {
                        m.focus = (m.focus + 1).min(RETRY_ON_START_BUTTONS.len() - 1);
                    }
                    return None;
                }
                KeyCode::Enter => {
                    let modal = self.confirm_retry_on_start.take()?;
                    // 0 cancel, 1 skip, 2 retry.
                    if modal.focus == 0 {
                        self.discard_pending_download(modal);
                        return None;
                    }
                    let mut request = modal.pending;
                    request.include_previously_failed = modal.focus == RETRY_ON_START_DEFAULT_FOCUS;
                    let (id, request) = self.dispatch_pending(modal.id, request);
                    return Some(AppCommand::StartDownload { id, request });
                }
                KeyCode::Esc => {
                    self.cancel_retry_on_start();
                    return None;
                }
                _ => return None,
            }
        }

        // Confirm-retry modal: button-driven. ←/→ move across [cancel, retry];
        // `enter` activates the focused button; `esc`/`q` cancel.
        if self.confirm_retry.is_some() {
            match key.code {
                KeyCode::Left => {
                    if let Some(m) = self.confirm_retry.as_mut() {
                        m.focus = m.focus.saturating_sub(1);
                    }
                    return None;
                }
                KeyCode::Right => {
                    if let Some(m) = self.confirm_retry.as_mut() {
                        m.focus = (m.focus + 1).min(CONFIRM_RETRY_BUTTONS.len() - 1);
                    }
                    return None;
                }
                KeyCode::Enter => {
                    let modal = self.confirm_retry.take()?;
                    // 0 cancel, 1 retry.
                    if modal.focus == CONFIRM_RETRY_DEFAULT_FOCUS {
                        return Some(AppCommand::RetryAllFailed {
                            download_id: modal.download_id,
                        });
                    }
                    return None;
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.confirm_retry = None;
                    return None;
                }
                _ => return None,
            }
        }

        // Update-changelog modal: ←/→ move [later, update]; ↵ activates; esc/q
        // close; ↑/↓/PageUp/Down/Home/End scroll the changelog. All other keys
        // are inert while open.
        if self.update_modal.is_some() {
            const UPDATE_PAGE: usize = 8;
            match key.code {
                KeyCode::Left => {
                    if let Some(m) = self.update_modal.as_mut() {
                        m.focus = m.focus.saturating_sub(1);
                    }
                }
                KeyCode::Right => {
                    if let Some(m) = self.update_modal.as_mut() {
                        m.focus = (m.focus + 1).min(UPDATE_MODAL_BUTTONS.len() - 1);
                    }
                }
                KeyCode::Up => {
                    if let Some(m) = self.update_modal.as_ref() {
                        m.scroll.set(m.scroll.get().saturating_sub(1));
                    }
                }
                KeyCode::Down => {
                    if let Some(m) = self.update_modal.as_ref() {
                        m.scroll.set(m.scroll.get().saturating_add(1));
                    }
                }
                KeyCode::PageUp => {
                    if let Some(m) = self.update_modal.as_ref() {
                        m.scroll.set(m.scroll.get().saturating_sub(UPDATE_PAGE));
                    }
                }
                KeyCode::PageDown => {
                    if let Some(m) = self.update_modal.as_ref() {
                        m.scroll.set(m.scroll.get().saturating_add(UPDATE_PAGE));
                    }
                }
                KeyCode::Home => {
                    if let Some(m) = self.update_modal.as_ref() {
                        m.scroll.set(0);
                    }
                }
                KeyCode::End => {
                    if let Some(m) = self.update_modal.as_ref() {
                        m.scroll.set(usize::MAX);
                    }
                }
                KeyCode::Enter => {
                    let modal = self.update_modal.take()?;
                    // 0 later, 1 update.
                    if modal.focus == UPDATE_MODAL_DEFAULT_FOCUS {
                        self.available_update = None;
                        return Some(AppCommand::StartUpdate);
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.update_modal = None;
                }
                _ => {}
            }
            return None;
        }

        // Help overlay owns all input while open: ↑/↓ (and PageUp/Down, Home/End)
        // scroll it; `?`/esc/`q` close it; everything else is inert. Render clamps
        // the offset to the viewport, so over-scrolling is harmless.
        if self.help_open {
            const HELP_PAGE: usize = 8;
            let scroll = self.help_scroll.get();
            match key.code {
                KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => self.close_help(),
                KeyCode::Up => self.help_scroll.set(scroll.saturating_sub(1)),
                KeyCode::Down => self.help_scroll.set(scroll.saturating_add(1)),
                KeyCode::PageUp => self.help_scroll.set(scroll.saturating_sub(HELP_PAGE)),
                KeyCode::PageDown => self.help_scroll.set(scroll.saturating_add(HELP_PAGE)),
                KeyCode::Home => self.help_scroll.set(0),
                KeyCode::End => self.help_scroll.set(usize::MAX),
                _ => {}
            }
            return None;
        }

        // We are "typing" only when a text field is focused AND in edit mode.
        // Outside edit mode a focused text field is selected-not-editing, so
        // letter/`?`/`x` keybinds fire as global hotkeys.
        let typing = self.editing && self.focused_text_input();

        // `q` only quits outside text fields; esc always does. Typing a key
        // therefore counts as a non-quit action and dismisses the quit prompt.
        let is_quit_key =
            matches!(key.code, KeyCode::Esc) || (matches!(key.code, KeyCode::Char('q')) && !typing);
        if self.home.quit_prompt && !is_quit_key {
            self.home.quit_prompt = false;
        }

        // `x` dismisses the topmost toast before any per-tab handler sees it
        // (dismissal precedence: toast → footer alert → app binding).
        // Skipped while typing so `x` reaches the field.
        if key.code == KeyCode::Char('x')
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && !typing
            && self.toasts.dismiss_top()
        {
            return None;
        }

        match key.code {
            KeyCode::Char('?') if !typing => {
                // Help is closed here (the open case is handled by the guard above).
                self.help_open = true;
                self.help_scroll.set(0);
                return None;
            }
            KeyCode::Char('u') if !typing && self.available_update.is_some() => {
                self.open_update_modal();
                return None;
            }
            KeyCode::Esc | KeyCode::Char('q') if matches!(key.code, KeyCode::Esc) || !typing => {
                // esc exits edit mode before anything else (back/quit cascade).
                if self.editing {
                    self.editing = false;
                    // Committing a config text edit applies it immediately. Login
                    // fields hold their own value and persist nothing on exit.
                    if self.active_tab() == Tab::Config && self.login.is_none() {
                        self.apply_config_change();
                    }
                    return None;
                }
                if self.close_modal() {
                    return None;
                }
                // The login split is a focus-trap panel: esc/q (non-typing) close
                // it in place and hand focus back to the config auth chip.
                if self.login.is_some() {
                    self.close_login();
                    return None;
                }
                // In the update-source browse, esc ascends one level (preview →
                // list → form) before the back/quit cascade takes over.
                if self.update_browsing() && self.home.update.ascend() {
                    return None;
                }
                // esc is purely "back": it cancels an armed quit prompt and backs
                // out of a dynamic tab, but never quits. Only `q` quits.
                if matches!(key.code, KeyCode::Esc) {
                    return self.handle_back_key();
                }
                return self.handle_quit_key();
            }
            // In a focused text field, ←/→ move the caret. On the get-maps source
            // strip they cycle the active source. Everywhere else they switch
            // tabs. Home/End jump to the field edges (text-field only).
            KeyCode::Left => {
                if typing {
                    self.caret_left_focused();
                } else if self.active_tab() == Tab::Home && self.home.focus == HomeField::Source {
                    self.home.cycle_source(false);
                } else if self.update_browsing() {
                    // In browse, ←/h focuses the collections list pane.
                    self.home.update.focus_list();
                } else if let Some(cmd) = self.prev_tab() {
                    return Some(cmd);
                }
            }
            KeyCode::Right => {
                if typing {
                    self.caret_right_focused();
                } else if self.active_tab() == Tab::Home && self.home.focus == HomeField::Source {
                    self.home.cycle_source(true);
                } else if self.update_browsing() {
                    // In browse, →/l focuses the preview pane.
                    self.home.update.focus_preview();
                } else if let Some(cmd) = self.next_tab() {
                    return Some(cmd);
                }
            }
            KeyCode::Home if typing => self.caret_home_focused(),
            KeyCode::End if typing => self.caret_end_focused(),
            // Outside a text field these jump to the top/bottom of the active
            // surface (also the target of vim `gg`/`G`). Help owns these keys
            // while open via the guard above.
            KeyCode::Home => self.jump_top(),
            KeyCode::End => self.jump_bottom(),
            // Page the active list (vim `Ctrl+u`/`Ctrl+d`). Forms jump to ends.
            KeyCode::PageUp => self.page_up(),
            KeyCode::PageDown => self.page_down(),
            KeyCode::Delete if typing => {
                if let Some(cmd) = self.delete_forward_focused() {
                    return Some(cmd);
                }
            }
            KeyCode::Tab => {
                // While editing the home directory field, `tab` completes the
                // path. Everywhere else it cycles to the next tab (←/→ also do).
                if self.editing
                    && self.active_tab() == Tab::Home
                    && self.home.focus == HomeField::Directory
                {
                    if let Some(candidates) = self.home.tab_complete_directory() {
                        self.push_toast(Toast::info("directory matches").with_detail(candidates));
                    }
                } else if let Some(cmd) = self.next_tab() {
                    return Some(cmd);
                }
            }
            // `shift+tab` cycles to the previous tab (mirrors `tab`).
            KeyCode::BackTab => {
                if let Some(cmd) = self.prev_tab() {
                    return Some(cmd);
                }
            }
            // ⇧↑ / ⇧↓ reorder the focused built-in mirror row in the Config tab's
            // try-order; the new order persists and the Get Maps count + pipeline
            // follow it (`apply_config_change` syncs it into the Get Maps tab).
            // Only fires on a mirror row, so shift+arrow elsewhere falls through
            // to plain focus movement.
            KeyCode::Up | KeyCode::Down
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && self.active_tab() == Tab::Config
                    && self.config.focus_is_builtin_mirror() =>
            {
                if self.config.reorder_focused_mirror(key.code == KeyCode::Up) {
                    self.apply_config_change();
                }
            }
            KeyCode::Up => {
                if self.update_browsing() {
                    self.home.update.scroll_up();
                } else if let Some(page) = self.active_download_page_mut() {
                    if page.failed_section_expanded && !page.failed_maps.is_empty() {
                        page.failed_focus_prev();
                    } else {
                        page.scroll_threads_up();
                    }
                } else {
                    self.focus_prev_field();
                }
            }
            KeyCode::Down => {
                if self.update_browsing() {
                    self.home.update.scroll_down();
                } else if let Some(page) = self.active_download_page_mut() {
                    if page.failed_section_expanded && !page.failed_maps.is_empty() {
                        page.failed_focus_next();
                    } else {
                        page.scroll_threads_down();
                    }
                } else {
                    self.focus_next_field();
                }
            }
            // `enter` is the universal activate/toggle key: it confirms the
            // focused button, toggles the focused checkbox/option, or opens a
            // list — replacing the old `space`-to-toggle binding. On a text
            // input it toggles edit mode instead.
            KeyCode::Enter => {
                if self.focused_text_input() {
                    let was_editing = self.editing;
                    let was_update_path = self.home_update_path_editing();
                    self.editing = !self.editing;
                    // Leaving edit mode commits: config rows flush to disk, the
                    // update source's osu! path persists to `[recent]`. Login
                    // fields hold their own value and persist nothing on exit.
                    if was_editing && self.active_tab() == Tab::Config && self.login.is_none() {
                        self.apply_config_change();
                    } else if was_editing && was_update_path {
                        self.persist_osu_path_inputs();
                    }
                    return None;
                }
                // The login split traps activation while open (before the tab match).
                if self.login.is_some() {
                    return self.login_enter();
                }
                match self.active_tab() {
                    Tab::Home => {
                        // Update-source browse: enter toggles the highlighted
                        // collection, or dispatches the download from the action
                        // bar. Preview rows are read-only.
                        if self.update_browsing() {
                            if self.home.update.preview_focused() {
                                return None;
                            }
                            if self.home.update.cursor_on_action() {
                                if let Some((id, request)) = self.request_selective_download() {
                                    return Some(AppCommand::StartSelectiveDownload {
                                        id,
                                        request,
                                    });
                                }
                            } else {
                                self.home.update.toggle_selected_collection();
                            }
                            return None;
                        }
                        match self.home.focus {
                            // The source picker is a cycle row: `enter` steps it
                            // forward, matching the config cycle fields.
                            HomeField::Source => self.home.cycle_source(true),
                            HomeField::Download => {
                                if let Some((id, request)) = self.request_download() {
                                    return Some(AppCommand::StartDownload { id, request });
                                }
                            }
                            // The mirrors row is read-only here; hand off to the
                            // Config tab, which owns all mirror editing.
                            HomeField::Mirrors => self.open_config_mirrors(),
                            // The scan CTA drives the scan → descend → re-scan
                            // state machine.
                            HomeField::UpdateScan => match self.home.update.scan_cta() {
                                ScanCta::Scan => {
                                    self.home.update.scan.scan_generation =
                                        self.home.update.scan.scan_generation.wrapping_add(1);
                                    return Some(AppCommand::ScanLocalDatabase);
                                }
                                ScanCta::Descend => self.home.update.descend(),
                                ScanCta::Busy => {}
                            },
                            field if field.is_toggle() => self.home.toggle_current(),
                            // Text inputs and the threads stepper have nothing to activate.
                            _ => {}
                        }
                    }
                    Tab::Config => match self.config.focus {
                        ConfigField::AuthChip => {
                            // The chip is a navigation affordance: open the login
                            // split on the right, which owns the login flow.
                            self.open_login();
                            return None;
                        }
                        field if field.is_text_input() || field.is_stepper() => {}
                        // toggle / cycle fields (space also works) — applied live
                        _ => {
                            if !self.block_osu_official_if_logged_out() {
                                self.config.toggle_current();
                                self.apply_config_change();
                            }
                        }
                    },
                    _ => {
                        // download page: enter expands/collapses the failed section
                        if let Some(page) = self.active_download_page_mut() {
                            page.toggle_failed_section();
                        }
                    }
                }
            }
            // `space` is a toggle alias for `enter`: it flips the focused
            // checkbox / list selection but never activates buttons, opens lists,
            // or confirms the auth chip. In a text input it types a literal
            // space instead.
            KeyCode::Char(' ') if self.login.is_some() => {
                // Login split: chips trigger on enter only; space just types.
                if typing && let Some(login) = self.login.as_mut() {
                    login.handle_char(' ');
                }
            }
            KeyCode::Char(' ') => match self.active_tab() {
                Tab::Home => {
                    if typing {
                        if self.home_update_path_editing() {
                            self.library.insert_char(' ');
                        } else if let Some(cmd) =
                            self.mutate_collection_then_resolve(|h| h.handle_char(' '))
                        {
                            return Some(cmd);
                        }
                    } else if self.update_browsing() {
                        // Toggle-alias for the collection checkbox (list pane only,
                        // not the action bar); the preview is read-only.
                        if !self.home.update.preview_focused()
                            && !self.home.update.cursor_on_action()
                        {
                            self.home.update.toggle_selected_collection();
                        }
                    } else if self.home.focus == HomeField::Source {
                        self.home.cycle_source(true);
                    } else if self.home.focus.is_toggle() {
                        self.home.toggle_current();
                    }
                }
                Tab::Config => match self.config.focus {
                    _ if typing => self.config.handle_char(' '),
                    ConfigField::AuthChip => {}
                    field if field.is_text_input() || field.is_stepper() => {}
                    _ => {
                        if !self.block_osu_official_if_logged_out() {
                            self.config.toggle_current();
                            self.apply_config_change();
                        }
                    }
                },
                _ => {
                    if let Some(page) = self.active_download_page_mut() {
                        page.toggle_failed_section();
                    }
                }
            },
            // `c` switches the osu! client (stable ↔ lazer) from any tab. It
            // changes the owned library, so it clears the prior scan and kicks a
            // fresh one. Suppressed while typing so `c` types a literal char, and
            // while the login split is open (it traps every field key).
            KeyCode::Char('c') if !typing && self.login.is_none() => {
                self.library.switch_client();
                let action = self.home.update.reset_for_client_switch();
                self.persist_osu_path_inputs();
                if action == UpdateAction::RefreshAll {
                    return Some(AppCommand::ScanLocalDatabase);
                }
                return None;
            }
            // Login split traps chars: no non-typing hotkeys, just type the field.
            KeyCode::Char(ch) if self.login.is_some() => {
                if typing && let Some(login) = self.login.as_mut() {
                    login.handle_char(ch);
                }
            }
            KeyCode::Char(ch) => match self.active_tab() {
                Tab::Home => {
                    // Update-source browse owns letter keys (a/A select, s sort,
                    // i/I mark-installed, r recheck).
                    if self.update_browsing() {
                        return self.handle_update_browse_char(ch);
                    }
                    // Stepper: +/- adjust thread count when threads field is focused.
                    if self.home.focus.is_stepper() {
                        match ch {
                            '+' => {
                                self.home.step_up();
                                return None;
                            }
                            '-' => {
                                self.home.step_down();
                                return None;
                            }
                            _ => {}
                        }
                    }
                    // When editing, the char types into the focused field (the
                    // update source's path field lives on `library`). Otherwise
                    // letters are global hotkeys: `d` jumps to the output-dir
                    // field, `s` to the download button.
                    if typing {
                        if self.home_update_path_editing() {
                            self.library.insert_char(ch);
                        } else if let Some(cmd) =
                            self.mutate_collection_then_resolve(|h| h.handle_char(ch))
                        {
                            return Some(cmd);
                        }
                    } else if ch == 'r' {
                        // The update source rechecks known-bad maps; the others
                        // (re-)probe mirror latency.
                        if self.home.source == GetMapsSource::Update {
                            if self.home.update.can_recheck_failed_maps() {
                                return Some(AppCommand::RecheckFailedMaps);
                            }
                        } else {
                            return Some(AppCommand::ProbeMirrors);
                        }
                    } else if ch == 'd' && self.home.source == GetMapsSource::Collection {
                        return Some(AppCommand::FocusOutputDir);
                    } else if ch == 's' && self.home.source == GetMapsSource::Collection {
                        // Jump straight to the download button — no arrowing down
                        // from the just-pasted collection field. Only the
                        // collection source has a download button to jump to.
                        self.home.focus = HomeField::Download;
                        self.editing = false;
                    }
                }
                Tab::Config => {
                    let focus = self.config.focus;
                    // Stepper: +/- adjust thread count when threads field is focused;
                    // each step applies to disk immediately.
                    if focus.is_stepper() {
                        match ch {
                            '+' => {
                                self.config.step_up();
                                self.apply_config_change();
                                return None;
                            }
                            '-' => {
                                self.config.step_down();
                                self.apply_config_change();
                                return None;
                            }
                            _ => {}
                        }
                    }
                    if typing {
                        self.config.handle_char(ch);
                    }
                }
                _ => {
                    if let Some(cmd) = self.handle_download_tab_key(ch) {
                        return Some(cmd);
                    }
                }
            },
            // Backspace edits only while in edit mode; outside it there is no
            // text-capture context, so it is inert.
            KeyCode::Backspace if typing => {
                // alt/ctrl+backspace deletes the previous word.
                if key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
                {
                    return self.backspace_word_focused();
                }
                if let Some(login) = self.login.as_mut() {
                    login.backspace();
                } else if self.home_update_path_editing() {
                    self.library.backspace();
                } else {
                    match self.active_tab() {
                        Tab::Home => {
                            if let Some(cmd) =
                                self.mutate_collection_then_resolve(HomeTab::backspace)
                            {
                                return Some(cmd);
                            }
                        }
                        Tab::Config => self.config.backspace(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        None
    }

    /// Route a bracketed-paste payload into the focused text field, mirroring
    /// the per-tab dispatch of a typed character. Inert unless a text input is
    /// focused and in edit mode (same `typing` gate as `handle_key`). Pasting
    /// into the home collection field re-resolves the collection, just as
    /// typing into it does.
    pub fn handle_paste(&mut self, text: String) -> Option<AppCommand> {
        if !(self.editing && self.focused_text_input()) {
            return None;
        }
        if let Some(login) = self.login.as_mut() {
            login.handle_paste(&text);
            return None;
        }
        if self.home_update_path_editing() {
            self.library.insert_str(&text);
            return None;
        }
        match self.active_tab() {
            Tab::Home => self.mutate_collection_then_resolve(|h| h.handle_paste(&text)),
            Tab::Config => {
                self.config.handle_paste(&text);
                None
            }
            _ => None,
        }
    }

    /// Record a newer release found in notify-only mode. Surfaces via the header
    /// version indicator and the footer `u` hint.
    pub fn set_available_update(&mut self, info: AvailableUpdate) {
        self.available_update = Some(info);
        self.update_phase = Some(UpdateIndicator::Available);
    }

    /// Open the update-changelog modal. No-op when no update is available.
    fn open_update_modal(&mut self) {
        if self.available_update.is_some() {
            self.update_modal = Some(UpdateModal {
                focus: UPDATE_MODAL_DEFAULT_FOCUS,
                scroll: Cell::new(0),
            });
        }
    }

    /// Advance the animation clock and drop any auto-dismiss toasts past their
    /// dwell. Driven by the runtime's periodic `Tick`.
    pub fn on_tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);
        self.toasts.clear_expired();
    }

    /// Push a success result toast (top-right).
    pub fn toast_ok(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast::success(message));
    }

    /// Push a neutral info toast (top-right).
    pub fn toast_info(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast::info(message));
    }

    /// Push a needs-attention toast (top-right, `WARNING`) — validation prompts
    /// and soft blocks, distinct from a failed op (`toast_err`).
    pub fn toast_warn(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast::warning(message));
    }

    /// Push an error toast (top-right, `DANGER`, longer dwell).
    pub fn toast_err(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast::danger(message));
    }

    /// Push a pre-built toast — for detail lines or sticky lifetimes.
    pub fn push_toast(&mut self, toast: Toast) {
        self.toasts.push(toast);
    }

    /// Mark the updates scan as errored and surface the reason as a toast.
    pub fn report_scan_error(&mut self, message: impl Into<String>) {
        self.home.update.mark_scan_error();
        self.toast_err(message);
    }

    pub fn handle_download_event(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::CollectionReady {
                id,
                collection_name,
                uploader,
                total_maps,
                output_dir,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.set_title(collection_name.clone());
                    page.uploader = Some(uploader);
                    page.total_maps = total_maps;
                    page.download_target = total_maps;
                    page.output_dir = Some(output_dir);
                    page.stage = DownloadStage::Downloading;
                    if page.session_start.is_none() {
                        page.session_start = Some(std::time::Instant::now());
                    }
                }
            }
            DownloadEvent::ResolveProgress { id, current, total } => {
                if let Some(page) = self.page_mut(id) {
                    page.resolve_progress = Some((current, total));
                }
            }
            DownloadEvent::CollectionSizeResolved { id, total_bytes } => {
                if let Some(page) = self.page_mut(id) {
                    page.stats.total_collection_bytes = Some(total_bytes);
                }
            }
            DownloadEvent::LowDiskSpace {
                id,
                available_bytes,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.low_disk_space = Some(available_bytes);
                }
            }
            DownloadEvent::VerifiedMapSizes { id, total_bytes } => {
                if let Some(page) = self.page_mut(id) {
                    page.stats.verified_bytes += total_bytes;
                }
            }
            DownloadEvent::BeatmapProgress {
                id,
                beatmapset_id,
                downloaded,
                total,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.update_progress(beatmapset_id, downloaded, total);
                    page.update_active_progress(beatmapset_id, downloaded, total);
                }
            }
            DownloadEvent::BeatmapStatus {
                id,
                beatmapset_id,
                stage,
                message,
                rate_limited,
                cooldown_until,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.update_active_status(
                        beatmapset_id,
                        stage,
                        &message,
                        rate_limited,
                        cooldown_until,
                    );
                }
            }
            DownloadEvent::DownloadTarget { id, remaining } => {
                if let Some(page) = self.page_mut(id) {
                    page.download_target = remaining;
                }
            }
            DownloadEvent::OverallProgress {
                id,
                downloaded,
                skipped,
                failed,
                unverified,
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.stats.downloaded = downloaded;
                    page.stats.skipped = skipped;
                    page.stats.failed = failed;
                    page.stats.unverified = unverified;
                }
            }
            DownloadEvent::StageChanged { id, stage } => {
                if let Some(page) = self.page_mut(id) {
                    page.stage = stage;
                    if matches!(stage, DownloadStage::Completed | DownloadStage::Failed) {
                        page.clear_active_downloads();
                    }
                }
            }
            DownloadEvent::BeatmapDeferred {
                id, beatmapset_id, ..
            } => {
                if let Some(page) = self.page_mut(id) {
                    page.mark_deferred(beatmapset_id);
                }
            }
            DownloadEvent::BeatmapVerified { id, duration_us } => {
                if let Some(page) = self.page_mut(id) {
                    page.stats.verify_total_count = page.stats.verify_total_count.saturating_add(1);
                    page.stats.verify_total_us =
                        page.stats.verify_total_us.saturating_add(duration_us);
                }
            }
            DownloadEvent::FailedMaps { id, failures } => {
                if let Some(page) = self.page_mut(id) {
                    // auto-expand only the first time failures appear — if the
                    // user manually collapsed the section, don't reopen it on
                    // a follow-up batch
                    let was_empty = page.failed_maps.is_empty();
                    page.set_failed_maps(failures);
                    if was_empty && !page.failed_maps.is_empty() {
                        page.failed_section_expanded = true;
                    }
                }
            }
            DownloadEvent::SkippedImported { id: _, count } => {
                self.toast_info(format!("skipped {count} already imported"));
            }
            DownloadEvent::Finished { id, summary } => {
                if let Some(page) = self.page_mut(id) {
                    page.stage = DownloadStage::Completed;
                    page.summary = Some(summary);
                }
            }
            DownloadEvent::Failed { id, message: _ } => {
                if let Some(page) = self.page_mut(id) {
                    page.stage = DownloadStage::Failed;
                    page.summary = None;
                    page.clear_active_downloads();
                }
            }
        }
    }

    pub fn tab_titles(&self) -> Vec<Cow<'_, str>> {
        let mut titles = Vec::with_capacity(self.total_tabs());
        titles.push(Cow::Borrowed(TAB_HOME_LOWER));
        titles.push(Cow::Borrowed(TAB_CONFIG_LOWER));
        for page in &self.downloads {
            titles.push(download_tab_title(page));
        }
        titles
    }

    pub fn download_for_tab(&self, tab: Tab) -> Option<&CollectionPage> {
        match tab {
            Tab::Download(slot) => self.downloads.get(slot),
            _ => None,
        }
    }

    pub fn active_download_page_mut(&mut self) -> Option<&mut CollectionPage> {
        match self.active_tab {
            Tab::Download(slot) => self.downloads.get_mut(slot),
            _ => None,
        }
    }

    pub fn handle_cancel_result(&mut self, download_id: DownloadId, was_running: bool) {
        let title = self.remove_download_page(download_id);
        self.active_tab = Tab::Home;
        self.home.quit_prompt = false;

        let display = title.unwrap_or_else(|| format!("download #{download_id}"));
        if was_running {
            self.push_toast(Toast::info("cancelled download").with_detail(display));
        } else {
            self.push_toast(Toast::info("no active download to cancel").with_detail(display));
        }
    }

    fn page_mut(&mut self, id: DownloadId) -> Option<&mut CollectionPage> {
        self.downloads.iter_mut().find(|page| page.id == id)
    }

    fn remove_download_page(&mut self, download_id: DownloadId) -> Option<String> {
        if let Some(position) = self
            .downloads
            .iter()
            .position(|page| page.id == download_id)
        {
            let title = self.downloads[position].title.clone();
            self.downloads.remove(position);
            Some(title)
        } else {
            None
        }
    }

    /// Handle `r`/`R` on the active download tab. Letter suppression never
    /// applies here (there are no text inputs on download pages).
    /// Allocate a new download page for a retry batch and return the ID + request.
    /// Returns `None` if the source page is missing or has no stored config.
    ///
    /// The output directory is reused from the original download so the files
    /// land in the same folder without requiring a new resolve step.
    pub fn start_retry_download(
        &mut self,
        source_download_id: DownloadId,
        ids: Vec<u32>,
    ) -> Option<(DownloadId, SelectiveDownloadRequest)> {
        let page = self.downloads.iter().find(|p| p.id == source_download_id)?;
        let config = page.download_config.clone()?;
        let output_dir = page
            .output_dir
            .clone()
            .unwrap_or_else(|| config.directory.clone());

        if self.downloads.len() >= usize::MAX - 1 {
            return None;
        }

        let retry_config = DownloadConfig {
            directory: output_dir,
            mirrors: config.mirrors.clone(),
            concurrent: config.concurrent,
            archive_validation: config.archive_validation,
            auto_skip_rate_limited: config.auto_skip_rate_limited,
            rate_limit_skip_secs: config.rate_limit_skip_secs,
        };

        let new_id = self.next_download_id;
        self.next_download_id += 1;

        let title = format!("retry #{source_download_id}");
        let concurrent = usize::from(retry_config.concurrent.max(1));
        let mut retry_page = CollectionPage::new(new_id, title.clone(), concurrent);
        retry_page.stage = DownloadStage::Resolving;
        retry_page.download_config = Some(retry_config.clone());
        self.downloads.push(retry_page);
        self.active_tab = Tab::Download(self.downloads.len() - 1);

        let request = SelectiveDownloadRequest {
            collection_ids: vec![],
            beatmapset_ids: ids.clone(),
            collections: vec![SelectiveDownloadCollection {
                id: 0,
                name: title,
                beatmapset_ids: ids,
            }],
            config: retry_config,
            snapshot_dir: None,
            snapshots: vec![],
        };
        Some((new_id, request))
    }

    fn handle_download_tab_key(&mut self, ch: char) -> Option<AppCommand> {
        match ch {
            // Case-insensitive retry (hotkeys are case-insensitive):
            // `r`/`R` retry ALL retryable failed maps (NotFound skipped); >50
            // routes through the confirm modal.
            'r' | 'R' => {
                let page = self.active_download_page_mut()?;
                let retryable = page.retryable_ids(None);
                let has_failed = !page.failed_maps.is_empty();
                let download_id = page.id;
                if retryable.is_empty() {
                    // `r` is advertised only when retryable maps exist, but if the
                    // user presses it with nothing but 404s (NotFound is never
                    // retryable), say why instead of silently doing nothing.
                    if has_failed {
                        self.push_toast(
                            Toast::info("nothing to retry")
                                .with_detail("failed maps are 404 / not found"),
                        );
                    }
                    return None;
                }
                let count = retryable.len();
                if count > 50 {
                    self.confirm_retry = Some(RetryAllConfirmModal {
                        download_id,
                        retryable_count: count,
                        focus: CONFIRM_RETRY_DEFAULT_FOCUS,
                    });
                    None
                } else {
                    Some(AppCommand::RetryAllFailed { download_id })
                }
            }
            // `s` defers (requeues) and `S` hard-drops rate-limit-stuck maps.
            // `defer_rate_limited` wakes only rows parked on an inline cooldown
            // *right now*, so `s` is gated on an inline-parked row existing;
            // gating it on the broader parked-or-deferred set would advertise a
            // dead key when everything is queue-deferred and nothing is parked.
            // `S` additionally drains deferred-pending queue items, so it keeps
            // the broader gate.
            's' => {
                let page = self.active_download_page_mut()?;
                if page.any_active_rate_limited() {
                    Some(AppCommand::DeferRateLimited { id: page.id })
                } else {
                    None
                }
            }
            'S' => {
                let page = self.active_download_page_mut()?;
                if page.rate_limited_or_deferred() {
                    Some(AppCommand::SkipRateLimited { id: page.id })
                } else {
                    None
                }
            }
            // `x` is reserved for toast dismissal (handled globally in
            // `handle_key`); settled download tabs close via `esc`/`q`.
            _ => None,
        }
    }

    /// Remove a settled download page and move focus to the adjacent tab.
    ///
    /// Navigation matches browser tab close: focus shifts to the tab to the
    /// LEFT. Since `STATIC_TABS >= 1`, this always lands on a valid tab —
    /// closing the leftmost download tab lands on `config`.
    fn close_settled_download_tab(&mut self, download_id: DownloadId) {
        let Some(position) = self
            .downloads
            .iter()
            .position(|page| page.id == download_id)
        else {
            return;
        };
        let closed_tab_index = Tab::Download(position).to_index();
        self.downloads.remove(position);
        self.active_tab = Tab::from_index(
            closed_tab_index
                .saturating_sub(1)
                .min(self.total_tabs() - 1),
        );
    }

    /// `esc` as a pure "back" key. Runs after the edit/modal/login/updates
    /// cascade. Cancels an armed quit prompt, backs out of a settled-download tab
    /// (closes it or cancels a running download), and otherwise does nothing at
    /// the static top level. It never arms or confirms a quit — quitting is
    /// `q`-only. The login split is closed earlier in the esc cascade.
    fn handle_back_key(&mut self) -> Option<AppCommand> {
        if self.home.quit_prompt {
            self.home.quit_prompt = false;
            return None;
        }
        if matches!(self.active_tab(), Tab::Download(_)) {
            return self.cancel_command_for_active_tab();
        }
        None
    }

    fn handle_quit_key(&mut self) -> Option<AppCommand> {
        if !matches!(self.active_tab(), Tab::Download(_)) {
            if self.home.quit_prompt {
                self.home.quit_prompt = false;
                return Some(AppCommand::Quit);
            }

            self.home.quit_prompt = true;
            debug!("Quit requested; showing confirmation prompt");
            return None;
        }

        self.home.quit_prompt = false;
        self.cancel_command_for_active_tab()
    }

    fn cancel_command_for_active_tab(&mut self) -> Option<AppCommand> {
        let Tab::Download(idx) = self.active_tab else {
            return None;
        };
        let Some(page) = self.downloads.get(idx) else {
            self.active_tab = Tab::Home;
            return None;
        };

        // Settled tabs have nothing to cancel — `esc`/`q` just closes them
        // in place, matching the `esc/q close` footer hint.
        if matches!(page.stage, DownloadStage::Completed | DownloadStage::Failed) {
            let download_id = page.id;
            self.close_settled_download_tab(download_id);
            return None;
        }

        Some(AppCommand::CancelDownload { id: page.id })
    }

    fn placeholder_title(input: &str, download_id: DownloadId) -> String {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return format!("collection {download_id}");
        }

        match utils::parse_collection_id(trimmed) {
            Ok(collection_id) => format!("collection {collection_id}"),
            Err(_) => format!("collection {trimmed}"),
        }
    }
}

/// "0" through "100" as `&'static str`, indexed by value.
static PCT_STR: [&str; 101] = [
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
    "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30", "31", "32",
    "33", "34", "35", "36", "37", "38", "39", "40", "41", "42", "43", "44", "45", "46", "47", "48",
    "49", "50", "51", "52", "53", "54", "55", "56", "57", "58", "59", "60", "61", "62", "63", "64",
    "65", "66", "67", "68", "69", "70", "71", "72", "73", "74", "75", "76", "77", "78", "79", "80",
    "81", "82", "83", "84", "85", "86", "87", "88", "89", "90", "91", "92", "93", "94", "95", "96",
    "97", "98", "99", "100",
];

/// Build the tab title for one download page.
///
/// - No progress data yet (`download_target == 0`): returns the bare name.
/// - In-progress: appends ` (N%)` where N = `(downloaded + skipped) / target * 100`.
/// - Completed (`DownloadStage::Completed`): appends ` (✓)`.
/// - Any failed maps: appends `*` after the progress suffix.
fn download_tab_title(page: &CollectionPage) -> Cow<'_, str> {
    let has_progress = page.download_target > 0;
    let has_failures = page.stats.failed > 0;

    if !has_progress && !has_failures {
        return Cow::Borrowed(page.title_lower());
    }

    let name = page.title_lower();
    // Reserve: name + " (100%)" + "*" = name + 8 bytes at most
    let mut s = String::with_capacity(name.len() + 8);
    s.push_str(name);

    if has_progress {
        if page.stage == DownloadStage::Completed {
            s.push_str(" (✓)");
        } else {
            let done = u64::from(page.stats.downloaded) + u64::from(page.stats.skipped);
            let pct = ((done * 100) / page.download_target as u64).min(100) as usize;
            s.push_str(" (");
            s.push_str(PCT_STR[pct]);
            s.push_str("%)");
        }
    }

    if has_failures {
        s.push('*');
    }

    Cow::Owned(s)
}

/// Load the persisted failed-maps file at `path` and intersect its ids with
/// `collection_ids`. Returns the intersection.
pub(crate) fn intersect_failed_ids(path: &Path, collection_ids: &HashSet<u32>) -> Vec<u32> {
    let file = failed_maps::load(path);
    file.beatmapset_ids
        .iter()
        .copied()
        .filter(|id| collection_ids.contains(id))
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/app_state.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/retry_keybind.rs"]
mod retry_keybind_tests;

#[cfg(test)]
#[path = "../../tests/unit/retry_on_download.rs"]
mod retry_on_download_tests;

#[cfg(test)]
#[path = "../../tests/unit/tab_titles.rs"]
mod tab_titles_tests;

#[cfg(test)]
#[path = "../../tests/unit/close_download_tab.rs"]
mod close_download_tab_tests;
