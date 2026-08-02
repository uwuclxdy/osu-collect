use super::{
    banner::BannerRecency,
    collection::CollectionPage,
    collection_state::{self, CollectionStateFile},
    config::{AuthLoginState, ConfigField, ConfigTab},
    covers::Covers,
    download_history::{DownloadHistory, HistoryRecord},
    downloads_tab::{DownloadsRow, DownloadsTab},
    failed_maps,
    find_source::{BrowseRow, EnrichSink, EnrichTarget, FindPlan, FindStatusMsg, SetBrowse},
    home::{FindBackend, GetMapsSource, HomeField, HomeTab},
    ignored_maps,
    library::LibraryState,
    login::{LoginField, LoginPhase, LoginTab},
    runtime, snapshots,
    tab::Tab,
    toast::{Toast, Toasts},
    update_source::{ScanCta, extract_collection_id},
};
use crate::auto_update::AvailableUpdate;
use crate::{
    config::{
        Config, RetryFailedOnDownload,
        constants::{
            DISK_CACHE_TTL, STATIC_TABS, TAB_CONFIG_LOWER, TAB_DOWNLOADS_LOWER, TAB_HOME_LOWER,
        },
        save_config,
    },
    core::collection::Collection,
    core::search::SearchQuery,
    download::{
        DownloadConfig, DownloadEvent, DownloadId, DownloadRequest, DownloadStage,
        IdsDownloadRequest, IdsRunSource, SelectiveDownloadCollection, SelectiveDownloadRequest,
    },
    utils,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fs2::available_space;
use osu_downloader::filter::FilterQuery;
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
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
    /// Downloads-tab view state (list cursor + pane focus). The rows it points
    /// into are built per frame from `downloads` + `history`.
    pub downloads_tab: DownloadsTab,
    /// Past-run records behind `download_history.json`. `App::new` starts an
    /// in-memory store; the runtime swaps in the disk-backed one, so tests
    /// never read or write the user's real file.
    pub history: DownloadHistory,
    /// Transient top-right notifications — results and errors. Ephemeral by
    /// design; durable signals live on banners, inline state, or tab markers.
    pub toasts: Toasts,
    /// Beatmapset cover-image cache + graphics picker for the set-browse
    /// preview. `App::new` seeds a test-safe halfblocks picker; the runtime
    /// swaps in the queried one before entering raw mode.
    pub covers: Covers,
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
    /// Pending confirmation for "Retry N failed mapsets?" when count > 50.
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
    /// Confirm-before-delete modal for the Downloads tab (`d`). Suppressed once
    /// the user arms its "don't ask again" toggle (`display.confirm_delete_history`).
    pub confirm_delete: Option<ConfirmDeleteModal>,
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

/// Identity of a Downloads-list row (see [`App::selected_row_key`]): a live
/// run by id, or a persisted record by its index into `history.records`.
#[derive(Debug, Clone, Copy)]
enum SelectedRow {
    Page(DownloadId),
    Record(usize),
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
    /// Start a fetch-skipping download of raw beatmapset ids picked from search
    /// or filter results (the ids are already in hand, so no collection fetch).
    StartIdsDownload {
        id: DownloadId,
        request: IdsDownloadRequest,
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
    /// Collection URL field changed and the form already holds that collection's
    /// snapshot, so kill the in-flight resolve without starting one.
    ///
    /// Separate from [`ResolveCollectionUrl`](Self::ResolveCollectionUrl) rather
    /// than a flag on it because cancelling and scheduling are separate effects
    /// that happened to share a command: skipping the fetch by suppressing the
    /// command also skipped the cancel, and the unnamed effect is the one that
    /// goes missing.
    CancelResolve,
    /// Collection URL field changed; schedule a debounced metadata resolve under
    /// `generation`, which every event that request emits is stamped with.
    ResolveCollectionUrl {
        generation: u64,
        value: String,
    },
    /// Run an osu! API v2 search (the `search` CTA or `load more`). `append` marks
    /// a `load more` page (append + dedup) versus a fresh search (replace).
    RunSearch {
        query: SearchQuery,
        append: bool,
    },
    /// Run an nzbasic filter fetch (the `filter` CTA).
    RunFilter {
        query: FilterQuery,
    },
    /// Fetch the next osu-batch enrichment page for an id-only browse's rows
    /// (auto once results land / the browse descends; `m` loads more).
    LoadEnrichment {
        target: EnrichTarget,
    },
    /// Probe latency for all built-in mirrors.
    ProbeMirrors,
    /// Fetch the cover image for the highlighted set-browse row (debounced,
    /// claimed `Pending` before dispatch).
    FetchCover {
        set_id: u32,
    },
    /// Backfill nekoha download sizes for the checked osu-routed find results
    /// (the download button's `· ~X` suffix). The dispatch claims which checked
    /// ids still need a probe, so emitting it on any selection change is cheap.
    ProbeFindSizes,
    /// Switch to the home tab and focus the output directory field.
    /// Triggered by the disk-low / disk-full banner action.
    FocusOutputDir,
    /// Confirm the update modal: download and apply the available update.
    StartUpdate,
    Quit,
}

/// State for the "Retry N failed mapsets?" confirm modal shown when `R` is pressed
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
/// activates the focused button — `retry` dispatches targeting the whole
/// collection, `skip` dispatches with [`previously_failed`](Self::previously_failed)
/// moved onto the request so the run never enqueues them, and `cancel` (or
/// `esc`) discards the queued download.
#[derive(Debug)]
pub struct RetryOnStartModal {
    pub id: DownloadId,
    /// The previously-failed beatmapsets of this collection: the set the prompt
    /// counts and the exact set `skip` drops from the run. One field so the
    /// number shown and the ids acted on cannot drift apart.
    pub previously_failed: HashSet<u32>,
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

/// What a Downloads-tab `d` delete removes. Captured at press time so a
/// background record promotion can't shift a stale index onto the wrong row: a
/// record is held by value (matched on delete), a live page by its stable id.
#[derive(Debug, Clone)]
pub enum DeleteTarget {
    /// A persisted history record — removed from `history.records`.
    Record(HistoryRecord),
    /// A settled-retained session page — hard-dropped WITHOUT a history record
    /// (unlike cancel/eviction, which promote one). Only a terminal-stage page
    /// is ever a target; an in-flight run must be cancelled first (`q`).
    Page(DownloadId),
}

/// State for the "Delete this entry?" confirmation modal (`d` on the Downloads
/// tab). Buttons (left→right): `cancel`, `delete`; ←/→ move `focus`, `space`
/// toggles `dont_ask_again`, `enter` activates the focused button, `esc`/`q`
/// cancel. Confirming `delete` with `dont_ask_again` armed persists the
/// suppression (`display.confirm_delete_history = false`).
#[derive(Debug)]
pub struct ConfirmDeleteModal {
    pub target: DeleteTarget,
    /// The entry's display title (record or page name), for the modal body.
    pub title: String,
    /// Selected button index into [`CONFIRM_DELETE_BUTTONS`].
    pub focus: usize,
    /// Whether the "don't ask again" checkbox is armed.
    pub dont_ask_again: bool,
}

/// Button labels for [`ConfirmDeleteModal`], left→right.
pub const CONFIRM_DELETE_BUTTONS: [&str; 2] = ["cancel", "delete"];
/// Default-focused button: `delete` — matches the retry/update modals (default
/// to the action); the confirm step and the named target are the safety.
pub const CONFIRM_DELETE_DEFAULT_FOCUS: usize = 1;

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
            downloads_tab: DownloadsTab::default(),
            history: DownloadHistory::default(),
            toasts: Toasts::default(),
            covers: Covers::new(),
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
            confirm_delete: None,
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

    /// Whether the osu! official mirror can be used: requires a logged-in
    /// session whose stored token carries the `*` (lazer-tier) scope. The
    /// greyed-out toggle, the "log in first" notice, the mirror count and the
    /// built list all read this so they match what the pipeline will try.
    ///
    /// This is a synchronous gate over the cached scope + login-state facts on
    /// [`ConfigTab`]. It cannot prove the token is still valid at request time
    /// — `ensure_valid` is a network call that runs in the pipeline
    /// (`resolve_osu_bearer`). A token that expires between render and dispatch
    /// still counts here; the pipeline's post-auth empty-check
    /// (`inject_mirror_auth` → `NoMirrors`) drops the run when no usable mirror
    /// survives.
    pub fn osu_official_unlocked(&self) -> bool {
        matches!(self.config.login_state, AuthLoginState::LoggedIn) && self.config.lazer_scope
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
                    .with_detail("open login from the config tab"),
            );
            return true;
        }
        false
    }

    /// Whether any download page is in a non-terminal stage (preparing,
    /// resolving, rechecking, or downloading). Drives the header brand
    /// animation, which idles once every page reaches `Completed`/`Failed`.
    pub fn is_downloading(&self) -> bool {
        self.downloads.iter().any(|p| !p.is_settled())
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

    /// Whether the Downloads tab's preview pane holds focus — the descended
    /// state where the download-control keys (`q` cancel, `s`/`S`, `r`) live.
    fn downloads_preview_focused(&self) -> bool {
        self.active_tab == Tab::Downloads && self.downloads_tab.preview_focused
    }

    /// Whether the Downloads run list owns navigation keys. A focused preview
    /// never falls through to list movement — on a record preview (no live
    /// page to scroll) the arrows are inert, so the cursor can't silently walk
    /// onto a different run while descended.
    fn downloads_list_focused(&self) -> bool {
        self.active_tab == Tab::Downloads && !self.downloads_tab.preview_focused
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
        if self.confirm_delete.is_some() {
            self.confirm_delete = None;
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
            || self.confirm_delete.is_some()
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
            Tab::Home => self.home.next_field(self.config.supporter()),
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
            Tab::Home => self.home.prev_field(self.config.supporter()),
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
            Tab::Home => self.home.first_field(self.config.supporter()),
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
            Tab::Home => self.home.last_field(self.config.supporter()),
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
        } else if self.home_set_browsing() {
            if let Some(browse) = self.active_set_browse_mut() {
                browse.scroll_to_edge(true);
            }
        } else if let Some(page) = self.active_download_page_mut() {
            page.jump_top();
        } else if self.downloads_list_focused() {
            self.downloads_tab.selected = 0;
        } else {
            self.focus_first_field();
        }
    }

    /// `G` / End: jump the active surface to its bottom.
    fn jump_bottom(&mut self) {
        if self.update_browsing() {
            self.home.update.scroll_to_edge(false);
        } else if self.home_set_browsing() {
            if let Some(browse) = self.active_set_browse_mut() {
                browse.scroll_to_edge(false);
            }
        } else if let Some(page) = self.active_download_page_mut() {
            page.jump_bottom();
        } else if self.downloads_list_focused() {
            self.downloads_tab.selected = self.downloads_row_count().saturating_sub(1);
        } else {
            self.focus_last_field();
        }
    }

    /// `Ctrl+u` / PageUp: page the active list up. Forms have no page, so they
    /// jump to the first field.
    fn page_up(&mut self) {
        if self.update_browsing() {
            self.home.update.page_up();
        } else if self.home_set_browsing() {
            if let Some(browse) = self.active_set_browse_mut() {
                browse.page_up();
            }
        } else if let Some(page) = self.active_download_page_mut() {
            page.page_up();
        } else if self.downloads_list_focused() {
            self.downloads_tab.page_up();
        } else {
            self.focus_first_field();
        }
    }

    /// `Ctrl+d` / PageDown: page the active list down.
    fn page_down(&mut self) {
        if self.update_browsing() {
            self.home.update.page_down();
        } else if self.home_set_browsing() {
            if let Some(browse) = self.active_set_browse_mut() {
                browse.page_down();
            }
        } else if let Some(page) = self.active_download_page_mut() {
            page.page_down();
        } else if self.downloads_list_focused() {
            let count = self.downloads_row_count();
            self.downloads_tab.page_down(count);
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
        // The Config tab is the sole mirror editor; push its settings into the
        // Get Maps tab so the enabled-count and the download list track the
        // change without a relaunch. Done before the save so a persistence
        // failure can't desync the visible count from the Config tab.
        self.home.sync_mirrors_from_config(&new_config.mirror);
        match save_config(&new_config) {
            Ok(_) => {
                // Theme is the only setting with a visible-now effect; swap the
                // live palette so the change shows without a relaunch.
                crate::tui::apply_theme(new_config.display.theme);
                self.config.loaded_config = new_config;
            }
            Err(err) => self.toast_err(err.to_string()),
        }
    }

    // ── auth transitions ──────────────────────────────────────────────────────
    //
    // The four writers of `config.supporter` that production uses. Each pairs the
    // config-tab half with [`settle_supporter_gate`](Self::settle_supporter_gate),
    // so the gate closing and the form reacting to it are one event: a settle
    // deferred to the next keypress leaves a frame where focus sits on a row the
    // render already dropped, and a settle skipped entirely leaves the six facet
    // VALUES applying from controls nobody can see.

    pub fn set_login_complete(&mut self, supporter: bool) {
        self.config.set_login_complete(supporter);
        self.settle_supporter_gate();
    }

    pub fn set_login_failed(&mut self) {
        self.config.set_login_failed();
        self.settle_supporter_gate();
    }

    pub fn set_logged_out(&mut self) {
        self.config.set_logged_out();
        self.settle_supporter_gate();
    }

    /// Adopt a confirmed `/me` supporter answer for a still-logged-in session
    /// (the startup re-probe). Ignored while logged out — see
    /// [`ConfigTab::set_supporter`](crate::app::ConfigTab::set_supporter).
    pub fn set_supporter(&mut self, supporter: bool) {
        self.config.set_supporter(supporter);
        self.settle_supporter_gate();
    }

    /// Bring everything gated on osu!supporter back in line with the flag. Only
    /// the closing direction needs work: opening the gate adds rows at their
    /// defaults, which needs no cleanup.
    fn settle_supporter_gate(&mut self) {
        if self.config.supporter() {
            return;
        }
        self.home.find.clear_supporter_facets();
        // The clamp can move focus off a descended multi-select chip row, and
        // `editing` would outlive it there — set on a row that is neither a text
        // input nor a chip row, so nothing would take it back down.
        if self.home.clamp_supporter_focus(self.config.supporter()) {
            self.editing = false;
        }
        // Every cleared facet was an osu-forcer, so the reset can hand the route
        // to nzbasic on its own. An auth event drives this with no keypress
        // behind it, which is the one path `handle_key`'s settle cannot reach.
        self.settle_find_route();
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
            self.set_login_failed();
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
        STATIC_TABS
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

    /// Persist the app-global library osu! client kind and path so the next launch
    /// restores them instead of re-detecting. Reads the on-disk config first so
    /// unsaved config-tab edits are never clobbered; failures are silent.
    fn persist_osu_path_inputs(&self) {
        let mut config = crate::config::load_config_or_default();
        config.recent.osu_client = Some(self.library.client_type);
        let path = self.library.osu_path.value.trim();
        config.recent.osu_path = (!path.is_empty()).then(|| path.to_string());
        let _ = save_config(&config);
    }

    /// Mark beatmapsets as installed: persist them to the ignore list and move
    /// them out of the missing list into the marked-installed group at once. A
    /// later scan that detects a genuine install auto-clears the entry (see
    /// `ignored_maps::reconcile_installed`); `unmark_installed` is the manual undo.
    fn mark_installed(&mut self, ids: Vec<u32>) {
        if ids.is_empty() {
            return;
        }
        let ids: HashSet<u32> = ids.into_iter().collect();
        if let Some(path) = ignored_maps::ignored_maps_path() {
            ignored_maps::record_ignored(&path, ids.iter().copied());
        }
        let count = ids.len();
        self.home.update.mark_installed_sets(&ids);
        self.toast_ok(format!(
            "marked {count} mapset{} installed",
            if count == 1 { "" } else { "s" }
        ));
    }

    /// Reverse `mark_installed`: prune the ids from the ignore list and move them
    /// back into the missing list so they reappear at once (no rescan needed).
    fn unmark_installed(&mut self, ids: Vec<u32>) {
        if ids.is_empty() {
            return;
        }
        let ids: HashSet<u32> = ids.into_iter().collect();
        if let Some(path) = ignored_maps::ignored_maps_path() {
            ignored_maps::record_unignored(&path, ids.iter().copied());
        }
        let count = ids.len();
        self.home.update.unmark_installed_sets(&ids);
        self.toast_ok(format!(
            "restored {count} mapset{}",
            if count == 1 { "" } else { "s" }
        ));
    }

    /// Flip the focused preview row between held back and re-included. Scoped to
    /// previously-deleted sets: those are the only ones the scan holds back, so
    /// on any other row the collection checkbox already decides membership and
    /// the key is inert (no toast — nothing changed).
    fn toggle_preview_included(&mut self) {
        match self.home.update.toggle_preview_included() {
            Some(true) => self.toast_ok("re-included 1 mapset"),
            Some(false) => self.toast_ok("held back 1 mapset"),
            None => {}
        }
    }

    /// Letter-key dispatch while browsing the update source's two panes. `a`/`A`
    /// select-all/none (list pane), `s` cycles the focused pane's sort, `i`/`I`
    /// mark the preview's focused row / whole collection installed, `u`/`U`
    /// reverse that (restore), and `r` rechecks known-bad maps. Only `r` yields a
    /// command; the rest mutate in place.
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
            'i' if self.home.update.preview_focused()
                && !self.home.update.preview_focused_is_marked() =>
            {
                let ids = self.home.update.preview_focused_id();
                self.mark_installed(ids);
            }
            'I' if self.home.update.preview_focused() => {
                let ids = self.home.update.highlighted_collection_missing_ids();
                self.mark_installed(ids);
            }
            'u' if self.home.update.preview_focused()
                && self.home.update.preview_focused_is_marked() =>
            {
                let ids = self.home.update.preview_focused_id();
                self.unmark_installed(ids);
            }
            'U' if self.home.update.preview_focused() => {
                let ids = self.home.update.highlighted_collection_marked_ids();
                self.unmark_installed(ids);
            }
            'r' if self.home.update.can_recheck_failed_maps() => {
                return Some(AppCommand::RecheckFailedMaps);
            }
            // Missing-set rows are id-only until enriched; `m` backfills the next
            // osu-batch page of titles (mirrors the flat browse's `m`).
            'm' if self.home.update.has_more_enrichment() => {
                return Some(AppCommand::LoadEnrichment {
                    target: EnrichTarget::Update,
                });
            }
            _ => {}
        }
        None
    }

    pub fn request_download(&mut self) -> Option<(DownloadId, DownloadRequest)> {
        let mut request = match self.home.build_request(
            self.osu_official_unlocked(),
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
        // Source mirrors the update-source scan (seeded from `[recent]`).
        request.skip_already_imported = self.config.skip_already_imported;
        request.osu_client = self.library.client_type;
        request.osu_path = self.library.osu_path();

        if self.downloads.len() >= usize::MAX - 1 {
            self.toast_err("too many downloads queued");
            return None;
        }

        self.persist_recent_inputs();

        let collection_id = utils::parse_collection_id(request.collection_input.trim()).ok();
        // The auto-resolve already fetched this exact payload to render the
        // "collection X · N mapsets" line; hand it over so `prepare` doesn't refetch.
        request.prefetched =
            collection_id.and_then(|id| self.home.collection_cache.get_fresh(id).cloned());
        let previously_failed = collection_id
            .map(|id| self.previously_failed_ids(id))
            .unwrap_or_default();

        // No prior failures for this collection — skip the modal entirely.
        if previously_failed.is_empty() {
            return Some(self.queue_download(request));
        }

        match self.config.retry_failed_on_download {
            RetryFailedOnDownload::Yes => Some(self.queue_download(request)),
            RetryFailedOnDownload::No => {
                request.previously_failed_skipped = previously_failed;
                Some(self.queue_download(request))
            }
            RetryFailedOnDownload::Ask => {
                let id = self.next_download_id;
                self.next_download_id += 1;
                self.confirm_retry_on_start = Some(RetryOnStartModal {
                    id,
                    previously_failed,
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
        self.focus_new_download_run();
    }

    /// Beatmapsets in `failed-beatmapsets.json` that belong to `collection_id`.
    /// The persisted file is not collection-scoped, so we pull the resolved id
    /// list from the `HomeTab` auto-resolve cache and intersect. This one set
    /// both sizes the prompt and, when the user declines the retry, becomes the
    /// run's exclusion — so what the prompt counts is what the run drops.
    ///
    /// Returns an empty set when:
    /// - the failed-maps file path is unavailable, OR
    /// - no resolved collection metadata is cached for `collection_id` (the
    ///   user hit `enter` before the 300 ms debounce fired). Suppressing the
    ///   prompt in that case matches "no prior context to compare" — the
    ///   pipeline will retry persisted failures in its normal flow.
    fn previously_failed_ids(&self, collection_id: u32) -> HashSet<u32> {
        let path = self
            .failed_maps_path_override
            .clone()
            .or_else(failed_maps::failed_maps_path);
        let Some(path) = path else {
            return HashSet::new();
        };

        let Some((cached_id, ids)) = self.home.resolved_collection.as_ref() else {
            return HashSet::new();
        };
        if *cached_id != collection_id {
            return HashSet::new();
        }

        let resolved_set: HashSet<u32> = ids.iter().copied().collect();
        intersect_failed_ids(&path, &resolved_set)
    }

    pub fn request_selective_download(&mut self) -> Option<(DownloadId, SelectiveDownloadRequest)> {
        let beatmapset_ids = self.home.update.selected_beatmapset_ids();
        if beatmapset_ids.is_empty() {
            // A soft validation block, not a scan failure — don't clobber the
            // `Ready` scan status (that would drop the CTA back to "scan for
            // updates" and discard the ready-to-review results still cached).
            self.toast_warn("no collections selected for download");
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
            self.toast_warn("no collections available");
            return None;
        }

        let mirrors = self.home.build_mirror_list(self.osu_official_unlocked());
        if mirrors.is_empty() {
            self.toast_warn("no mirrors enabled (configure in the config tab)");
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
            format!("update #{}", collection_ids[0])
        } else {
            format!("update ({} collections)", collection_ids.len())
        };

        let concurrent_usize = usize::from(concurrent.max(1));
        let mut page = CollectionPage::new(id, placeholder_title, concurrent_usize);
        page.stage = DownloadStage::Resolving;
        // config is stored after it is built below; we'll set it there
        self.downloads.push(page);
        self.focus_new_download_run();

        self.push_toast(
            Toast::success(format!("queued update download #{id}")).with_detail(format!(
                "{} mapset{}",
                beatmapset_ids.len(),
                if beatmapset_ids.len() == 1 { "" } else { "s" }
            )),
        );

        let config = DownloadConfig {
            directory,
            mirrors,
            concurrent,
            archive_validation: self.config.archive_validation,
            auto_skip_rate_limited: self.config.auto_skip_rate_limited,
            rate_limit_skip_secs: self.config.parse_rate_limit_skip_secs().unwrap_or(60),
        };

        let mut current_snapshots = snapshots::current_snapshots(
            self.library.client_type,
            &self.home.update.scan.local_collections_raw,
            self.home.update.scan.local_beatmapsets.iter(),
            |name| extract_collection_id(name).and_then(|id| u32::try_from(id).ok()),
        );
        // A held-back set is absent from the local library, so the baseline built
        // above would tell the next scan it is no longer deleted — and it is not a
        // run target, so the completion gate cannot withhold the write on its
        // behalf. Put it back before the snapshot leaves for the pipeline. The
        // diff (not the missing list) is the source: a set marked installed
        // leaves the missing list but stays in the diff for as long as it is
        // absent locally. Re-included sets are stripped first so the fold does
        // not carry them back as deleted, which would undo the re-include.
        let fold_diffs = runtime::exclude_reincluded_sets(
            self.home.update.scan.snapshot_diffs.clone(),
            self.library.client_type,
            &self.home.update.selection.cached_missing_sets,
        );
        runtime::retain_held_back_in_snapshots(
            &mut current_snapshots,
            self.library.client_type,
            &fold_diffs,
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
                beatmapset_ids: self.home.update.selected_beatmapset_ids_for(*collection_id),
            })
            .collect();

        // store config snapshot for potential retry
        if let Some(page) = self.downloads.last_mut() {
            page.download_config = Some(config.clone());
        }

        // The scan behind these results fetched every one of these collections;
        // reuse those payloads rather than refetching them at press time.
        let prefetched = self.prefetched_collections(&collection_ids);

        let request = SelectiveDownloadRequest {
            collection_ids,
            beatmapset_ids,
            collections,
            config,
            snapshot_dir: snapshots::snapshots_dir(),
            snapshots,
            skip_already_imported: self.config.skip_already_imported,
            osu_client: self.library.client_type,
            osu_path: self.library.osu_path(),
            prefetched,
        };

        Some((id, request))
    }

    /// The still-fresh session-cached payloads for `collection_ids`. A miss is
    /// simply absent — the pipeline fetches that collection itself.
    fn prefetched_collections(&self, collection_ids: &[u32]) -> HashMap<u32, Collection> {
        collection_ids
            .iter()
            .filter_map(|&id| {
                self.home
                    .collection_cache
                    .get_fresh(id)
                    .map(|collection| (id, collection.clone()))
            })
            .collect()
    }

    // ── search + collection browse&pick ───────────────────────────────────────

    /// The active source's flat browse (search results / collection browse&pick),
    /// or `None` for the update source (whose two-level browse is separate).
    fn active_set_browse(&self) -> Option<&SetBrowse> {
        match self.home.source {
            GetMapsSource::Find => Some(&self.home.find.browse),
            GetMapsSource::Collection => Some(&self.home.collection_browse),
            GetMapsSource::Update => None,
        }
    }

    fn active_set_browse_mut(&mut self) -> Option<&mut SetBrowse> {
        match self.home.source {
            GetMapsSource::Find => Some(&mut self.home.find.browse),
            GetMapsSource::Collection => Some(&mut self.home.collection_browse),
            GetMapsSource::Update => None,
        }
    }

    /// Advance the cover-image prefetch one tick: returns a [`AppCommand::FetchCover`]
    /// once the highlighted flat-browse row has held focus past the debounce and
    /// its cover isn't cached. `None` for any non-flat-browse view (the update
    /// source's missing-set preview has no single highlight) resets the debounce.
    pub fn poll_cover_prefetch(&mut self) -> Option<AppCommand> {
        let highlighted = if self.home_set_browsing() {
            self.active_set_browse()
                .and_then(|browse| browse.highlighted_row())
                .map(|row| row.id)
        } else {
            None
        };
        self.covers
            .poll_prefetch(highlighted)
            .map(|set_id| AppCommand::FetchCover { set_id })
    }

    /// Whether a flat set browse (search results / collection browse&pick) is
    /// descended on the Get Maps tab, so browse keys own input instead of the
    /// form-field nav.
    fn home_set_browsing(&self) -> bool {
        self.active_tab() == Tab::Home && self.active_set_browse().is_some_and(|b| b.is_browsing())
    }

    /// A size-probe command when the active browse is the OSU-routed find results
    /// — nzbasic keeps its `SizeMap` and the collection browse never gets sizes,
    /// so both return `None`. Emitted after a selection change (toggle / select
    /// all); the dispatch decides which checked ids actually need a probe.
    fn find_size_probe_cmd(&self) -> Option<AppCommand> {
        (self.home.source == GetMapsSource::Find
            && self.home.find.results_backend() == Some(FindBackend::Osu))
        .then_some(AppCommand::ProbeFindSizes)
    }

    /// Cycle the focused find-form chip (`space`/`enter`). One union form: preset
    /// / special are nzbasic-flavored, mode / status / sort edit the shared value
    /// that either resolved backend reads.
    fn cycle_find_chip(&mut self, forward: bool) {
        match self.home.focus {
            HomeField::FindPreset => self.home.find.cycle_preset(forward),
            HomeField::FindSpecial => self.home.find.cycle_special(forward),
            HomeField::FindMode => self.home.find.cycle_mode(forward),
            HomeField::FindStatus => self.home.find.cycle_status(forward),
            HomeField::FindSort => self.home.find.cycle_sort(forward),
            HomeField::FindExplicit => self.home.find.cycle_explicit(forward),
            HomeField::FindGenre => self.home.find.cycle_genre(forward),
            HomeField::FindLanguage => self.home.find.cycle_language(forward),
            HomeField::FindPlayed => self.home.find.cycle_played(forward),
            _ => {}
        }
    }

    /// Whether focus is descended into a multi-select find row — the state where
    /// `←`/`→` walk its chip cursor instead of switching tabs, and `space`
    /// toggles the chip under it. The arrow handler and the footer both read it.
    pub fn find_chip_editing(&self) -> bool {
        self.editing
            && self.active_tab() == Tab::Home
            && !self.home_set_browsing()
            && self.home.focus.is_find_multi_chip()
    }

    /// `space` on a descended multi-select find row: flip the member under the
    /// row's chip cursor.
    fn toggle_find_chip(&mut self) {
        match self.home.focus {
            HomeField::FindExtra => self.home.find.extra.toggle(),
            HomeField::FindRank => self.home.find.rank.toggle(),
            _ => {}
        }
    }

    /// `←`/`→` on a descended multi-select find row: walk its chip cursor,
    /// wrapping.
    fn move_find_chip_cursor(&mut self, forward: bool) {
        match self.home.focus {
            HomeField::FindExtra => self.home.find.extra.move_cursor(forward),
            HomeField::FindRank => self.home.find.rank.move_cursor(forward),
            _ => {}
        }
    }

    /// Open the find results browse over the form — on the `view maps` button,
    /// and on a fresh search / filter landing, which descends with no keypress
    /// behind it.
    ///
    /// The browse owns input from here, so it takes any row-scoped edit mode
    /// down with it. A multi-select chip row left descended under an async
    /// landing would swallow the browse's first `esc` (the edit-mode arm sits
    /// ahead of the ascend) and collapse its hint bar to a text-field
    /// affordance the browse has no field for.
    pub(crate) fn open_find_browse(&mut self) {
        self.home.find.browse.descend();
        self.editing = false;
    }

    /// Build + dispatch a fresh find run from the form. The single CTA routes
    /// here: the resolved plan runs the matching backend task; a conflict or bad
    /// input surfaces as a toast and nothing dispatches. The one-shot guest login
    /// nudge fires on a *successful* guest search (in `handle_home_search_event`),
    /// not here, so a no-creds build never shows it.
    fn dispatch_find_run(&mut self) -> Option<AppCommand> {
        match self.home.find.build_plan(None) {
            Ok(FindPlan::Osu(query)) => {
                self.home.find.status_msg = FindStatusMsg::Loading;
                Some(AppCommand::RunSearch {
                    query,
                    append: false,
                })
            }
            Ok(FindPlan::Nzbasic(query)) => {
                self.home.find.status_msg = FindStatusMsg::Loading;
                Some(AppCommand::RunFilter { query })
            }
            Err(reason) => {
                self.toast_warn(reason);
                None
            }
        }
    }

    /// Load the next osu page (`m` in the browse). No-op once the last page has
    /// been reached (`next_cursor` is `None`) or the form no longer routes osu.
    fn load_more_search(&mut self) -> Option<AppCommand> {
        let cursor = self.home.find.next_cursor.clone()?;
        match self.home.find.build_plan(Some(cursor)) {
            Ok(FindPlan::Osu(query)) => {
                self.home.find.status_msg = FindStatusMsg::Loading;
                Some(AppCommand::RunSearch {
                    query,
                    append: true,
                })
            }
            // The criteria drifted to force nzbasic (or errored): don't page a
            // query that no longer produced these results.
            _ => None,
        }
    }

    /// One-shot "searching as guest" nudge: fires once per logged-out session
    /// after a guest search actually returns, inviting login for the
    /// supporter-only filters. Called from `handle_home_search_event` so it only
    /// shows when a guest search succeeded (never in a no-creds build that errors).
    pub(crate) fn nudge_guest_search_if_logged_out(&mut self) {
        let logged_in = matches!(self.config.login_state, AuthLoginState::LoggedIn);
        if logged_in || self.home.find.login_nudged {
            return;
        }
        self.home.find.login_nudged = true;
        self.push_toast(
            Toast::info("searching as guest")
                .with_detail("log in from the config tab for more filters"),
        );
    }

    /// Character keys inside a flat set browse: `a`/`A` select-all/clear on the
    /// list pane, `m` loads more (next osu results page, or the next osu-batch
    /// enrichment page for id-only rows).
    fn handle_set_browse_char(&mut self, ch: char) -> Option<AppCommand> {
        match ch {
            'a' | 'A' => {
                if let Some(browse) = self.active_set_browse_mut()
                    && !browse.preview_focused()
                {
                    browse.set_all_selected(ch == 'a');
                    // Selection changed → backfill sizes for the newly-checked osu sets.
                    return self.find_size_probe_cmd();
                }
            }
            // `s` cycles the preview pane's difficulty sort (stars ↑ / ↓).
            's' => {
                if let Some(browse) = self.active_set_browse_mut()
                    && browse.preview_focused()
                {
                    browse.cycle_diff_sort();
                }
            }
            // Route `m` by the backend that produced the loaded results, not the
            // form chip (a cross-routed run can differ): osu pages the next
            // results page…
            'm' if self.home.source == GetMapsSource::Find
                && self.home.find.results_backend() == Some(FindBackend::Osu) =>
            {
                return self.load_more_search();
            }
            // …nzbasic results are all in hand, so `m` backfills the next page of
            // id-only rows with osu-batch metadata instead.
            'm' if self.home.source == GetMapsSource::Find
                && self.home.find.results_backend() == Some(FindBackend::Nzbasic)
                && self.home.find.browse.has_more_enrichment() =>
            {
                return Some(AppCommand::LoadEnrichment {
                    target: EnrichTarget::Find,
                });
            }
            // Collection browse&pick rows are id-only too; `m` enriches the next.
            'm' if self.home.source == GetMapsSource::Collection
                && self.home.collection_browse.has_more_enrichment() =>
            {
                return Some(AppCommand::LoadEnrichment {
                    target: EnrichTarget::Collection,
                });
            }
            _ => {}
        }
        None
    }

    /// Dispatch the download from the active source's form `Download` button,
    /// routed by source:
    /// - collection: the whole resolved collection, or — when a proper subset is
    ///   checked in browse&pick — the picked sets via the selective path.
    /// - search: the picked results via the fetch-skipping [`IdsDownloadRequest`].
    /// - update: every missing set of the checked collections via the selective path.
    fn dispatch_form_download(&mut self) -> Option<AppCommand> {
        match self.home.source {
            GetMapsSource::Collection => {
                if self.home.collection_subset_picked() {
                    let (id, request) = self.request_collection_pick_download()?;
                    Some(AppCommand::StartSelectiveDownload { id, request })
                } else {
                    let (id, request) = self.request_download()?;
                    Some(AppCommand::StartDownload { id, request })
                }
            }
            GetMapsSource::Find => {
                let (id, request) = self.request_find_download()?;
                Some(AppCommand::StartIdsDownload { id, request })
            }
            GetMapsSource::Update => {
                let (id, request) = self.request_selective_download()?;
                Some(AppCommand::StartSelectiveDownload { id, request })
            }
        }
    }

    /// Open the resolved collection in the checkbox browse (the collection
    /// source's `view N maps` button). A fresh collection defaults all sets
    /// selected; re-opening the same collection preserves the prior picks. No-op
    /// with a toast until a collection has resolved.
    ///
    /// The browse descends immediately, id-only (osu!collector exposes no set
    /// metadata), and titles fill in behind the loading cue. Returns
    /// `LoadEnrichment{Collection}` when any title still needs fetching, else
    /// `None`; a reopen of a fully-cached collection hydrates from `meta_cache`
    /// with no flash.
    fn open_collection_browse(&mut self) -> Option<AppCommand> {
        let Some((collection_id, ids)) = self.home.resolved_collection.clone() else {
            self.toast_warn("resolve a collection first");
            return None;
        };
        if ids.is_empty() {
            self.toast_warn("collection has no mapsets");
            return None;
        }
        // Re-opening the same collection keeps the user's selection alive;
        // `set_rows` retains checks for still-present ids, so only a fresh /
        // changed collection defaults to all-selected. Cached rows also hydrate
        // their metadata straight from the session cache here.
        let fresh = self.home.collection_browse_id != Some(collection_id);
        let rows: Vec<BrowseRow> = ids
            .into_iter()
            .map(|id| BrowseRow { id, meta: None })
            .collect();
        self.home
            .collection_browse
            .set_rows(rows, &self.home.meta_cache);
        if fresh {
            self.home.collection_browse.set_all_selected(true);
        }
        // Seed the pager from this resolve's (set, diff) pairs; a set already in
        // the cache is pruned, so a reopen only pages titles the app hasn't
        // fetched (osu!collector exposes no set metadata of its own).
        let seeds: Vec<(u32, Option<u32>)> = self
            .home
            .resolved_enrich_pairs
            .iter()
            .map(|&(set, diff)| (diff, Some(set)))
            .collect();
        self.home
            .collection_browse
            .seed_enrichment(seeds, &self.home.meta_cache);
        // Snapshot the id so the dispatch stays paired with these rows even if a
        // late resolve updates `resolved_collection` while the browse is open.
        self.home.collection_browse_id = Some(collection_id);
        self.home.collection_browse.descend();
        self.home
            .collection_browse
            .has_more_enrichment()
            .then_some(AppCommand::LoadEnrichment {
                target: EnrichTarget::Collection,
            })
    }

    /// Build a fetch-skipping download from the picked find results. The run
    /// source (`search-` / `filter-` subdir prefix + uploader label) follows the
    /// RESOLVED backend of the loaded results, falling back to the form's planned
    /// route when no fetch recorded one (e.g. test-seeded rows).
    pub fn request_find_download(&mut self) -> Option<(DownloadId, IdsDownloadRequest)> {
        // Every find dispatch converges here, which is the only place the route
        // check holds regardless of what reached it. `handle_key`'s settle covers
        // the key paths, but "no key both edits the criteria and dispatches in
        // one press" is a convention the compiler does not enforce — one that did
        // would otherwise tag the run with a backend the form stopped showing and
        // drop it in that backend's directory. A drifted route clears the rows
        // here, so the empty-selection guard below refuses the run.
        self.settle_find_route();
        let beatmapset_ids = self.home.find.browse.selected_ids();
        if beatmapset_ids.is_empty() {
            self.toast_warn("no mapsets selected for download");
            return None;
        }
        if self.home.mirror_count(self.osu_official_unlocked()) == 0 {
            self.toast_warn("no mirrors enabled (configure in the config tab)");
            return None;
        }
        if self.downloads.len() >= usize::MAX - 1 {
            self.toast_err("too many downloads queued");
            return None;
        }

        let source = IdsRunSource::from(self.home.find.run_backend());
        let word = source.uploader();
        let label = self.home.find.run_label();
        let folder_tag = self.home.find.folder_tag();
        let config = self.build_run_config();
        let id = self.next_download_id;
        self.next_download_id += 1;

        let concurrent = usize::from(config.concurrent.max(1));
        let mut page = CollectionPage::new(id, format!("{word} \"{label}\""), concurrent);
        page.stage = DownloadStage::Resolving;
        page.download_config = Some(config.clone());
        self.downloads.push(page);
        self.focus_new_download_run();

        self.push_toast(
            Toast::success(format!("queued {word} download #{id}")).with_detail(format!(
                "{} mapset{}",
                beatmapset_ids.len(),
                if beatmapset_ids.len() == 1 { "" } else { "s" }
            )),
        );

        // Both find routes already hold some sizes at request time (osu: the
        // lazy probe cache; nzbasic: its free per-set `SizeMap`); seeding them
        // here lets the run skip a probe entirely once every picked id is known.
        let known_sizes = self.home.find.known_sizes_for(&beatmapset_ids);
        let request = IdsDownloadRequest {
            beatmapset_ids,
            label,
            folder_tag,
            source,
            config,
            auto_overwrite: self.home.auto_overwrite,
            skip_already_imported: self.config.skip_already_imported,
            osu_client: self.library.client_type,
            osu_path: self.library.osu_path(),
            known_sizes,
        };
        Some((id, request))
    }

    /// Build a selective download from the picked collection browse&pick sets.
    /// Routes through the selective path (a single resolved collection), which
    /// re-fetches the collection for checksums + writes the selective `collection.db`.
    pub fn request_collection_pick_download(
        &mut self,
    ) -> Option<(DownloadId, SelectiveDownloadRequest)> {
        // Every picked-subset dispatch converges here, which is the only place
        // the check holds regardless of what reached it — `dispatch_form_download`
        // testing `collection_subset_picked` first is a convention the compiler
        // does not enforce, and this is `pub`. Neither half carries it alone: the
        // settle drops a resolve the field moved off but deliberately leaves the
        // browse's own id standing (the rows really did come from it), and
        // `picked_collection_id` is what refuses the pair the settle just broke.
        self.home.settle_collection_resolve();
        // The id snapshotted at browse-open, and only while the form still has
        // that collection resolved — so the run is paired with the rows it
        // dispatches AND with the collection the URL field names. Checked before
        // the selection: an unpaired browse is the more fundamental failure, and
        // reporting "no mapsets selected" for it would send the user to fix the
        // wrong thing.
        let Some(collection_id) = self.home.picked_collection_id() else {
            self.toast_warn("resolve a collection first");
            return None;
        };
        let beatmapset_ids = self.home.collection_browse.selected_ids();
        if beatmapset_ids.is_empty() {
            self.toast_warn("no mapsets selected for download");
            return None;
        }
        if self.home.mirror_count(self.osu_official_unlocked()) == 0 {
            self.toast_warn("no mirrors enabled (configure in the config tab)");
            return None;
        }
        if self.downloads.len() >= usize::MAX - 1 {
            self.toast_err("too many downloads queued");
            return None;
        }

        let config = self.build_run_config();
        let id = self.next_download_id;
        self.next_download_id += 1;

        let concurrent = usize::from(config.concurrent.max(1));
        let title = format!(
            "collection #{collection_id} ({} picked)",
            beatmapset_ids.len()
        );
        let mut page = CollectionPage::new(id, title, concurrent);
        page.stage = DownloadStage::Resolving;
        page.download_config = Some(config.clone());
        self.downloads.push(page);
        self.focus_new_download_run();

        self.push_toast(
            Toast::success(format!("queued collection download #{id}")).with_detail(format!(
                "{} mapset{}",
                beatmapset_ids.len(),
                if beatmapset_ids.len() == 1 { "" } else { "s" }
            )),
        );

        let collections = vec![SelectiveDownloadCollection {
            id: collection_id,
            name: String::new(),
            beatmapset_ids: beatmapset_ids.clone(),
        }];
        let request = SelectiveDownloadRequest {
            collection_ids: vec![collection_id],
            beatmapset_ids,
            collections,
            config,
            snapshot_dir: None,
            snapshots: Vec::new(),
            skip_already_imported: self.config.skip_already_imported,
            osu_client: self.library.client_type,
            osu_path: self.library.osu_path(),
            prefetched: self.prefetched_collections(&[collection_id]),
        };
        Some((id, request))
    }

    /// The shared run config (folder / mirrors / threads / validation) both flat
    /// browses download with — the collection source's fields, per keep-both.
    fn build_run_config(&self) -> DownloadConfig {
        DownloadConfig {
            directory: self.home.resolved_directory(),
            mirrors: self.home.build_mirror_list(self.osu_official_unlocked()),
            concurrent: self.home.resolved_threads(),
            archive_validation: self.config.archive_validation,
            auto_skip_rate_limited: self.config.auto_skip_rate_limited,
            rate_limit_skip_secs: self.config.parse_rate_limit_skip_secs().unwrap_or(60),
        }
    }

    /// Run `mutate` against the home form, then — only when focus is the
    /// collection field AND its value actually changed — settle the cached
    /// resolve against the new value and return a `ResolveCollectionUrl` command
    /// carrying it.
    ///
    /// No-op keystrokes (backspace on an empty field, digits typed into the
    /// threads input) thus do not spawn a wasted resolve task.
    ///
    /// Every key that can edit the collection field converges here — `handle_char`,
    /// space, paste, `backspace`, `backspace_word`, `delete_forward` are the whole
    /// set, and none of them is reachable from the Home tab any other way — which
    /// is what makes this the place to settle rather than
    /// [`handle_key`](Self::handle_key). The settle lands in the same press as the
    /// edit, so the frame this key produces can neither name nor dispatch a
    /// collection the field has stopped naming; the debounced fetch it also
    /// schedules is 300 ms away and a failed one never arrives at all, so waiting
    /// for a response to correct the form was never an option.
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
        // A settle that re-armed from the session cache leaves the field's
        // collection already resolved, so there is nothing left to FETCH — but the
        // previous value's fetch still has to be CANCELLED, which is why this
        // returns a command instead of `None`. The cache's TTL is the freshness
        // contract the download path reads it under; firing a confirming request
        // on a hit would be a second, stricter notion of fresh living only here,
        // and its `Failed` could land an error line over a snapshot that is
        // present and correct.
        // Bump first: from here every exit is a command, and the single `Some(`
        // is what makes that a compile-time fact rather than a convention three
        // `return None`s above it happen to respect. Anything the superseded
        // request still emits is dropped on the generation regardless, so a
        // fourth exit that forgot to cancel would leak a request, not a defect.
        let generation = self.home.supersede_resolve();
        Some(if self.home.settle_collection_resolve() {
            AppCommand::CancelResolve
        } else {
            AppCommand::ResolveCollectionUrl {
                generation,
                value: self.home.collection.value.clone(),
            }
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
            // While browsing (update or a flat set browse) the browse keys own
            // input, so no field is a text input even if `home.focus` names one.
            Tab::Home => {
                !self.update_browsing()
                    && !self.home_set_browsing()
                    && self.home.focus.is_text_input()
            }
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

    /// The controller's key entry point: dispatch the key, then settle whatever
    /// it knocked out of line before the frame it produces is drawn.
    ///
    /// The settle runs AFTER the dispatch so the state the next render reads is
    /// already consistent — settling first would leave one frame advertising a
    /// route the loaded results didn't come from. The ordering costs nothing in
    /// safety because it is not what carries it: the dispatch itself settles at
    /// the point every find run converges ([`request_find_download`]), so a
    /// handler that edited the criteria and dispatched in one press would still
    /// be refused rather than relying on this call having run first.
    ///
    /// [`request_find_download`]: Self::request_find_download
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<AppCommand> {
        let command = self.dispatch_key(key);
        self.settle_find_route();
        command
    }

    /// Bring the loaded find results back in line with the route the criteria
    /// now resolve to, cueing the user when that drops rows they picked.
    ///
    /// Paired with every mutation of the find criteria the same way
    /// [`settle_supporter_gate`](Self::settle_supporter_gate) is paired with
    /// every write of the supporter flag: [`handle_key`](Self::handle_key)
    /// covers the form edits, the supporter settle covers the facet reset that
    /// an auth event can fire with no keypress behind it.
    ///
    /// Silent row loss is its own defect, so the drop gets the same treatment
    /// the client switch gives its cleared scan — what went, and the way back.
    fn settle_find_route(&mut self) {
        let Some(backend) = self.home.find.settle_route() else {
            return;
        };
        self.push_toast(Toast::info("find results cleared").with_detail(format!(
            "criteria now route via {} · run find again",
            backend.label()
        )));
    }

    fn dispatch_key(&mut self, mut key: KeyEvent) -> Option<AppCommand> {
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
                    if modal.focus != RETRY_ON_START_DEFAULT_FOCUS {
                        // `skip`: the run drops exactly the set the prompt counted.
                        request.previously_failed_skipped = modal.previously_failed;
                    }
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

        // Confirm-delete modal: button-driven. ←/→ move across [cancel, delete];
        // `space` toggles "don't ask again"; `enter` activates the focused button
        // (delete arms suppression when the toggle is on); `esc`/`q` cancel.
        if self.confirm_delete.is_some() {
            match key.code {
                KeyCode::Left => {
                    if let Some(m) = self.confirm_delete.as_mut() {
                        m.focus = m.focus.saturating_sub(1);
                    }
                }
                KeyCode::Right => {
                    if let Some(m) = self.confirm_delete.as_mut() {
                        m.focus = (m.focus + 1).min(CONFIRM_DELETE_BUTTONS.len() - 1);
                    }
                }
                KeyCode::Char(' ') => {
                    if let Some(m) = self.confirm_delete.as_mut() {
                        m.dont_ask_again = !m.dont_ask_again;
                    }
                }
                KeyCode::Enter => {
                    let modal = self.confirm_delete.take()?;
                    // 0 cancel, 1 delete.
                    if modal.focus == 1 {
                        if modal.dont_ask_again {
                            self.disable_delete_confirm();
                        }
                        self.delete_download(modal.target);
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.confirm_delete = None;
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
            KeyCode::Char('u')
                if !typing && self.available_update.is_some() && !self.update_browsing() =>
            {
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
                // In a browse, esc ascends one level (preview → list → form)
                // before the back/quit cascade takes over.
                if self.update_browsing() && self.home.update.ascend() {
                    return None;
                }
                if self.home_set_browsing()
                    && let Some(browse) = self.active_set_browse_mut()
                    && browse.ascend()
                {
                    return None;
                }
                // esc is purely "back": it cancels an armed quit prompt and backs
                // out of a dynamic tab, but never quits. Only `q` quits.
                if matches!(key.code, KeyCode::Esc) {
                    return self.handle_back_key();
                }
                return self.handle_quit_key();
            }
            // In a focused text field, ←/→ move the caret; in a descended browse
            // they focus the list / preview pane. Everywhere else they switch
            // tabs — the source strip + search chips cycle on space/enter, not
            // arrows. Home/End jump to the field edges (text-field only).
            // A multi-select find row (`extra`, `rank`) descended into edit mode
            // takes the same arrow suspension a focused text input takes for its
            // caret: ←/→ walk its chip cursor. At rest the row is not descended,
            // so they switch tabs there like everywhere else.
            KeyCode::Left | KeyCode::Right if self.find_chip_editing() => {
                self.move_find_chip_cursor(key.code == KeyCode::Right);
            }
            KeyCode::Left => {
                if typing {
                    self.caret_left_focused();
                } else if self.update_browsing() {
                    // In browse, ←/h focuses the collections list pane.
                    self.home.update.focus_list();
                } else if self.home_set_browsing() {
                    if let Some(browse) = self.active_set_browse_mut() {
                        browse.focus_list();
                    }
                } else if self.downloads_preview_focused() {
                    // ←/esc both leave the run preview without cancelling (`q` is
                    // the cancel key); the list keeps arrow-tab-switching.
                    self.downloads_tab.preview_focused = false;
                } else if let Some(cmd) = self.prev_tab() {
                    return Some(cmd);
                }
            }
            KeyCode::Right => {
                if typing {
                    self.caret_right_focused();
                } else if self.update_browsing() {
                    // In browse, →/l focuses the preview pane.
                    self.home.update.focus_preview();
                } else if self.home_set_browsing() {
                    if let Some(browse) = self.active_set_browse_mut() {
                        browse.focus_preview();
                    }
                } else if self.downloads_preview_focused() {
                    // Already at the deepest pane; swallow so → doesn't switch
                    // tabs out from under the descended preview.
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
                } else if self.home_set_browsing() {
                    if let Some(browse) = self.active_set_browse_mut() {
                        browse.scroll_up();
                    }
                } else if let Some(page) = self.active_download_page_mut() {
                    if page.failed_section_expanded && !page.failed_maps.is_empty() {
                        page.failed_focus_prev();
                    } else {
                        page.scroll_threads_up();
                    }
                } else if self.downloads_list_focused() {
                    let count = self.downloads_row_count();
                    self.downloads_tab.select_prev(count);
                } else {
                    self.focus_prev_field();
                }
            }
            KeyCode::Down => {
                if self.update_browsing() {
                    self.home.update.scroll_down();
                } else if self.home_set_browsing() {
                    if let Some(browse) = self.active_set_browse_mut() {
                        browse.scroll_down();
                    }
                } else if let Some(page) = self.active_download_page_mut() {
                    if page.failed_section_expanded && !page.failed_maps.is_empty() {
                        page.failed_focus_next();
                    } else {
                        page.scroll_threads_down();
                    }
                } else if self.downloads_list_focused() {
                    let count = self.downloads_row_count();
                    self.downloads_tab.select_next(count);
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
                        // Update-source browse is a pure selector: enter toggles
                        // the highlighted collection, or — on the preview pane —
                        // a previously-deleted set's hold-back. The download
                        // fires from the form's `Download` button.
                        if self.update_browsing() {
                            if self.home.update.preview_focused() {
                                self.toggle_preview_included();
                            } else {
                                self.home.update.toggle_selected_collection();
                            }
                            return None;
                        }
                        // Flat set browse (search results / collection browse&pick)
                        // is a pure selector too: enter toggles the highlighted row
                        // (preview rows are read-only).
                        if self.home_set_browsing() {
                            if self
                                .active_set_browse()
                                .is_some_and(|b| b.preview_focused())
                            {
                                return None;
                            }
                            if let Some(browse) = self.active_set_browse_mut() {
                                browse.toggle_selected();
                            }
                            // Selection changed → size probe (osu find route only).
                            return self.find_size_probe_cmd();
                        }
                        match self.home.focus {
                            // The source picker is a cycle row: `enter` steps it
                            // forward, matching the config cycle fields.
                            HomeField::Source => self.home.cycle_source(true),
                            // Per-source download button; routed by active source.
                            HomeField::Download => return self.dispatch_form_download(),
                            // Open the resolved collection in the checkbox browse.
                            HomeField::CollectionBrowse => return self.open_collection_browse(),
                            // The mirrors row is read-only here; hand off to the
                            // Config tab, which owns all mirror editing.
                            HomeField::Mirrors => self.open_config_mirrors(),
                            // The scan button only ever scans (re-scan after a
                            // completed one); inert while a scan is in flight.
                            HomeField::UpdateScan => match self.home.update.scan_cta() {
                                ScanCta::Scan => {
                                    self.home.update.scan.scan_generation =
                                        self.home.update.scan.scan_generation.wrapping_add(1);
                                    return Some(AppCommand::ScanLocalDatabase);
                                }
                                ScanCta::Busy => {}
                            },
                            // Open the two-pane browse over the scan's missing
                            // sets. Inert until a scan finds updates — the button
                            // renders disabled meanwhile, and focus can still land
                            // on it, so guard the descend (disabled rows are no-ops).
                            HomeField::UpdateBrowse => {
                                if self.home.update.total_missing_count() > 0 {
                                    // Pure descend — the pager was seeded at
                                    // scan-land. Self-heal a missed/failed prefetch
                                    // by re-kicking page 1 ONLY when nothing was
                                    // ever fetched (cursor 0); cursor > 0 means page
                                    // 1 landed or is in flight, so this never
                                    // eager-fetches page 2 (that stays on `m`).
                                    self.home.update.descend();
                                    if self.home.update.enrich_cursor() == 0
                                        && self.home.update.has_more_enrichment()
                                    {
                                        return Some(AppCommand::LoadEnrichment {
                                            target: EnrichTarget::Update,
                                        });
                                    }
                                }
                            }
                            // Reopen the find results browse without re-fetching.
                            // Inert until fresh results are loaded and still match
                            // the current inputs — guard the descend so an unloaded
                            // / stale view button never opens the browse.
                            HomeField::FindBrowse => {
                                if !self.home.find.browse.rows.is_empty()
                                    && self.home.find.results_current()
                                {
                                    self.open_find_browse();
                                }
                            }
                            // The find CTA dispatches the resolved plan (osu search
                            // or nzbasic filter).
                            HomeField::FindRun => return self.dispatch_find_run(),
                            // Find chips step forward on enter (like the source strip);
                            // a multi-select row has no single "next" value, so
                            // enter descends into its own edit mode instead —
                            // where ←/→ walk the chip cursor and space toggles —
                            // and the next enter leaves it.
                            field if field.is_find_chip() => self.cycle_find_chip(true),
                            field if field.is_find_multi_chip() => self.editing = !self.editing,
                            field if field.is_disclosure() => {
                                self.home.find.toggle_advanced_filters()
                            }
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
                    Tab::Downloads => {
                        if self.downloads_tab.preview_focused {
                            // On the run preview, enter expands/collapses the
                            // failed section (history records have none).
                            if let Some(page) = self.active_download_page_mut() {
                                page.toggle_failed_section();
                            }
                        } else if self.downloads_row_count() > 0 {
                            // Descend into the highlighted run's preview.
                            self.downloads_tab.preview_focused = true;
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
                        // Toggle-alias for whichever checkbox the focused pane
                        // owns: the collection's, or a held-back set's.
                        if self.home.update.preview_focused() {
                            self.toggle_preview_included();
                        } else {
                            self.home.update.toggle_selected_collection();
                        }
                    } else if self.home_set_browsing() {
                        // Toggle-alias for the result checkbox (list pane only).
                        let inert = self
                            .active_set_browse()
                            .is_some_and(|b| b.preview_focused());
                        if !inert {
                            if let Some(browse) = self.active_set_browse_mut() {
                                browse.toggle_selected();
                            }
                            // Selection changed → size probe (osu find route only).
                            return self.find_size_probe_cmd();
                        }
                    } else if self.home.focus == HomeField::Source {
                        self.home.cycle_source(true);
                    } else if self.home.focus.is_find_chip() {
                        self.cycle_find_chip(true);
                    } else if self.home.focus.is_find_multi_chip() {
                        // Descended, `space` toggles the chip under the cursor;
                        // at rest it descends like `enter` rather than advancing
                        // a value, which is what a multi row has instead.
                        if self.editing {
                            self.toggle_find_chip();
                        } else {
                            self.editing = true;
                        }
                    } else if self.home.focus.is_disclosure() {
                        self.home.find.toggle_advanced_filters();
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
            // changes the owned library, so it clears the prior scan — but does
            // not auto-scan; the user scans manually from the update form.
            // Suppressed while typing so `c` types a literal char, and while the
            // login split is open (it traps every field key).
            KeyCode::Char('c') if !typing && self.login.is_none() => {
                self.library.switch_client();
                self.home.update.reset_for_client_switch();
                self.persist_osu_path_inputs();
                // The scan was cleared; nudge the user to re-scan against the new
                // library so the empty update form isn't a dead end.
                self.push_toast(
                    Toast::info(format!("switched to {}", self.library.client_type.label()))
                        .with_detail("scan to find updates"),
                );
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
                    // i/I mark-installed, u/U restore, r recheck).
                    if self.update_browsing() {
                        return self.handle_update_browse_char(ch);
                    }
                    // Flat set browse owns a/A select-all/clear + m load-more.
                    if self.home_set_browsing() {
                        return self.handle_set_browse_char(ch);
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
                    } else if ch == 's' {
                        // Jump to the last enabled ("clickable") button in the
                        // active source's form — the furthest-along CTA
                        // (find/scan → view maps → download), falling back to the
                        // download button when none are enabled. A repeat `s`
                        // while already on a button cycles the other available
                        // buttons. (`d` for the output-dir field stays
                        // Collection-only: search/update borrow it silently.)
                        self.home.focus = self.home.cycle_enabled_button(
                            self.config.supporter(),
                            self.osu_official_unlocked(),
                        );
                        self.editing = false;
                    } else if let Some(source) = ch
                        .to_digit(10)
                        .and_then(|d| (d as usize).checked_sub(1))
                        .and_then(|idx| GetMapsSource::ALL.get(idx).copied())
                    {
                        // Digit `k` jumps straight to the k-th strip source;
                        // generic over `ALL` so it tracks the strip as sources
                        // change. Focus returns to the strip so the switch reads.
                        self.home.source = source;
                        self.home.focus = HomeField::Source;
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
                if matches!(stage, DownloadStage::Completed | DownloadStage::Failed) {
                    self.settle_run(id, |page| {
                        page.stage = stage;
                        page.clear_active_downloads();
                    });
                } else if let Some(page) = self.page_mut(id) {
                    page.stage = stage;
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
            DownloadEvent::CollectionsUnresolved { id: _, count } => {
                self.toast_warn(format!(
                    "{count} collection{} could not be fetched; \
                     their maps arrived without collection membership",
                    if count == 1 { "" } else { "s" }
                ));
            }
            DownloadEvent::Finished { id, summary } => {
                self.settle_run(id, |page| {
                    page.stage = DownloadStage::Completed;
                    page.summary = Some(summary);
                });
            }
            DownloadEvent::Failed { id, message: _ } => {
                self.settle_run(id, |page| {
                    page.stage = DownloadStage::Failed;
                    page.summary = None;
                    page.clear_active_downloads();
                });
            }
        }
    }

    /// Apply a terminal-stage mutation to a run, persist its history record
    /// while the page stays retained (crash safety — the record is on disk
    /// before the page ever drops), and evict the oldest settled pages past
    /// the retention cap. Settling regroups the list (actives before settled),
    /// so the cursor is re-anchored to the run it was on.
    fn settle_run(&mut self, id: DownloadId, apply: impl FnOnce(&mut CollectionPage)) {
        let key = self.selected_row_key();
        let records_before = self.history.records.len();
        if let Some(page) = self.page_mut(id) {
            apply(page);
        }
        if let Some(page) = self.downloads.iter().find(|page| page.id == id) {
            self.history.record_settled(page);
        }
        self.evict_settled_pages_over_cap();
        self.reanchor_selection(key, records_before);
    }

    pub fn tab_titles(&self) -> Vec<Cow<'_, str>> {
        vec![
            Cow::Borrowed(TAB_HOME_LOWER),
            Cow::Borrowed(TAB_DOWNLOADS_LOWER),
            Cow::Borrowed(TAB_CONFIG_LOWER),
        ]
    }

    /// Indices into `downloads` in Downloads-list row order: active runs in
    /// push order (stable while running), then settled-retained runs newest
    /// first (most recent past on top, matching the history records below).
    fn download_page_row_order(&self) -> Vec<usize> {
        let settled = |i: &usize| self.downloads[*i].is_settled();
        let mut order: Vec<usize> = (0..self.downloads.len()).filter(|i| !settled(i)).collect();
        order.extend((0..self.downloads.len()).rev().filter(settled));
        order
    }

    /// The Downloads-tab list rows: live pages first, then persisted past-run
    /// records. Built per frame; `downloads_tab.selected` indexes into this.
    pub fn downloads_rows(&self) -> Vec<DownloadsRow<'_>> {
        let mut rows: Vec<DownloadsRow<'_>> = self
            .download_page_row_order()
            .into_iter()
            .map(|i| DownloadsRow::Page(&self.downloads[i]))
            .collect();
        rows.extend(self.history.records.iter().map(DownloadsRow::Record));
        rows
    }

    fn downloads_row_count(&self) -> usize {
        self.downloads.len() + self.history.records.len()
    }

    /// The `downloads` index of the page under the Downloads-list cursor, or
    /// `None` when the cursor sits on a history record (or the list is empty).
    fn selected_page_index(&self) -> Option<usize> {
        self.download_page_row_order()
            .get(self.downloads_tab.selected)
            .copied()
    }

    /// The page under the Downloads-list cursor (the preview auto-follows it).
    pub fn selected_download_page(&self) -> Option<&CollectionPage> {
        self.selected_page_index().map(|i| &self.downloads[i])
    }

    /// The page owning download-control input: the one under the cursor, but
    /// only while the Downloads preview pane holds focus — the scroll / defer /
    /// skip / retry keys are scoped there.
    pub fn active_download_page_mut(&mut self) -> Option<&mut CollectionPage> {
        if self.active_tab != Tab::Downloads || !self.downloads_tab.preview_focused {
            return None;
        }
        let index = self.selected_page_index()?;
        self.downloads.get_mut(index)
    }

    /// Identity of the row under the Downloads cursor, stable across row
    /// reorders — a settle regroups the page section, a removal promotes a
    /// record — so the cursor can re-anchor to the same RUN, not the same
    /// position (a positional cursor would silently switch the preview to a
    /// different run, and `q` would cancel the wrong download).
    fn selected_row_key(&self) -> Option<SelectedRow> {
        let order = self.download_page_row_order();
        let selected = self.downloads_tab.selected;
        if let Some(&page_index) = order.get(selected) {
            return Some(SelectedRow::Page(self.downloads[page_index].id));
        }
        let record = selected - order.len();
        (record < self.history.records.len()).then_some(SelectedRow::Record(record))
    }

    /// Move the cursor back onto the row identified by `key` after a mutation.
    /// A page that dropped re-anchors to its just-promoted record (records
    /// insert at the front); an existing record shifts by however many records
    /// were inserted above it (`records_before` is the pre-mutation count).
    fn reanchor_selection(&mut self, key: Option<SelectedRow>, records_before: usize) {
        let grew = self.history.records.len().saturating_sub(records_before);
        match key {
            Some(SelectedRow::Page(id)) => {
                let order = self.download_page_row_order();
                let pages = order.len();
                self.downloads_tab.selected = order
                    .iter()
                    .position(|&i| self.downloads[i].id == id)
                    // Dropped page → its record now leads the records section.
                    .unwrap_or(pages);
            }
            Some(SelectedRow::Record(i)) => {
                self.downloads_tab.selected = self.download_page_row_order().len() + i + grew;
            }
            None => {}
        }
        self.downloads_tab.clamp(self.downloads_row_count());
    }

    /// Point the Downloads-list cursor at a just-queued run (the newest active
    /// row) so opening the tab lands on it. Launch stays on the current tab
    /// (signalled by the queued toast) unless `display.jump_to_downloads` is
    /// on, which switches to the Downloads tab.
    fn focus_new_download_run(&mut self) {
        let actives = self.downloads.iter().filter(|p| !p.is_settled()).count();
        self.downloads_tab.selected = actives.saturating_sub(1);
        // Queued from another tab: land on the list. A retry dispatched from a
        // descended preview keeps the preview, now on the new retry run.
        if self.active_tab != Tab::Downloads {
            self.downloads_tab.preview_focused = false;
            if self.config.jump_to_downloads {
                // Writes active_tab without close_login(): safe because the
                // login focus-trap intercepts every queue keypress first.
                debug_assert!(self.login.is_none());
                self.active_tab = Tab::Downloads;
            }
        }
    }

    pub fn handle_cancel_result(&mut self, download_id: DownloadId, was_running: bool) {
        // Removal re-anchors the cursor onto the run's promoted record; hand
        // focus back to the list on it.
        let title = self.remove_download_page(download_id);
        self.downloads_tab.preview_focused = false;
        self.home.quit_prompt = false;

        let display = title.unwrap_or_else(|| format!("download #{download_id}"));
        if was_running {
            self.push_toast(Toast::info("download cancelled").with_detail(display));
        } else {
            self.push_toast(Toast::info("no active download to cancel").with_detail(display));
        }
    }

    fn page_mut(&mut self, id: DownloadId) -> Option<&mut CollectionPage> {
        self.downloads.iter_mut().find(|page| page.id == id)
    }

    /// Drop a run's page — the single removal choke point. The history record
    /// is written BEFORE the page goes (a never-settled page records as
    /// cancelled), so no removal path can lose a run. The cursor re-anchors by
    /// run identity; a removed selected run lands on its promoted record.
    fn remove_download_page(&mut self, download_id: DownloadId) -> Option<String> {
        let position = self
            .downloads
            .iter()
            .position(|page| page.id == download_id)?;
        let key = self.selected_row_key();
        let records_before = self.history.records.len();
        let page = self.downloads.remove(position);
        self.history.record_removed(&page);
        self.reanchor_selection(key, records_before);
        Some(page.title)
    }

    /// Whether the row under the Downloads cursor can be deleted with `d`: a
    /// persisted history record, or a settled-retained session run. An in-flight
    /// run is not deletable — cancel it with `q` first. Drives the `d delete`
    /// footer hint.
    pub fn selected_row_deletable(&self) -> bool {
        if self.active_tab != Tab::Downloads {
            return false;
        }
        match self.selected_row_key() {
            Some(SelectedRow::Record(_)) => true,
            Some(SelectedRow::Page(id)) => self
                .downloads
                .iter()
                .find(|p| p.id == id)
                .is_some_and(CollectionPage::is_settled),
            None => false,
        }
    }

    /// `d` on the Downloads tab: capture the entry under the cursor, then either
    /// open the confirm modal or delete straight away when the user disabled it.
    /// Inert on an in-flight run (cancel with `q` first) or an empty list.
    fn request_delete_download(&mut self) -> Option<AppCommand> {
        let (target, title) = match self.selected_row_key()? {
            SelectedRow::Record(index) => {
                let record = self.history.records.get(index)?.clone();
                let title = record.title.clone();
                (DeleteTarget::Record(record), title)
            }
            SelectedRow::Page(id) => {
                let page = self.downloads.iter().find(|p| p.id == id)?;
                if !page.is_settled() {
                    return None;
                }
                (DeleteTarget::Page(id), page.title.clone())
            }
        };
        if self.config.loaded_config.display.confirm_delete_history {
            self.confirm_delete = Some(ConfirmDeleteModal {
                target,
                title,
                focus: CONFIRM_DELETE_DEFAULT_FOCUS,
                dont_ask_again: false,
            });
        } else {
            self.delete_download(target);
        }
        None
    }

    /// Perform a confirmed delete. A record drops from `history.records`; a
    /// settled page is hard-dropped together with its crash-safe pending record,
    /// so — unlike cancel/eviction — it leaves NO history entry behind. The
    /// cursor keeps its row slot (the next entry slides up) and is clamped.
    fn delete_download(&mut self, target: DeleteTarget) {
        let title = match target {
            DeleteTarget::Record(record) => {
                if !self.history.remove_record(&record) {
                    return;
                }
                record.title
            }
            DeleteTarget::Page(id) => {
                let Some(pos) = self.downloads.iter().position(|p| p.id == id) else {
                    return;
                };
                let title = self.downloads[pos].title.clone();
                self.downloads.remove(pos);
                self.history.discard_pending(id);
                title
            }
        };
        // The previewed entry may be gone; drop to the list, then keep the cursor
        // on a real row.
        self.downloads_tab.preview_focused = false;
        self.downloads_tab.clamp(self.downloads_row_count());
        self.push_toast(Toast::info("deleted from downloads").with_detail(title));
    }

    /// Persist "don't ask again" for the delete modal: flip the in-memory config
    /// so this session skips the prompt, and write it to disk (re-read first so
    /// nothing else is clobbered). Failures are silent — a missed persist only
    /// means the prompt returns next launch.
    fn disable_delete_confirm(&mut self) {
        self.config.loaded_config.display.confirm_delete_history = false;
        let mut config = crate::config::load_config_or_default();
        config.display.confirm_delete_history = false;
        let _ = save_config(&config);
    }

    /// Keep at most [`HISTORY_CAP`](super::download_history::HISTORY_CAP)
    /// settled pages retained; the oldest evict to their history records.
    fn evict_settled_pages_over_cap(&mut self) {
        loop {
            let settled: Vec<DownloadId> = self
                .downloads
                .iter()
                .filter(|p| p.is_settled())
                .map(|p| p.id)
                .collect();
            if settled.len() <= super::download_history::HISTORY_CAP {
                return;
            }
            self.remove_download_page(settled[0]);
        }
    }

    /// Record every still-retained run on quit: settled pages promote their
    /// pending records, aborted-in-flight runs record as cancelled. Called by
    /// the runtime after the input loop ends.
    pub fn flush_history_on_exit(&mut self) {
        for page in std::mem::take(&mut self.downloads) {
            self.history.record_removed(&page);
        }
    }

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
        self.focus_new_download_run();

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
            // Carried for parity with the other two `SelectiveDownloadRequest`
            // sites rather than for an effect: `collection_ids` is empty here, so
            // `resolve_selective_with` returns `EmptyCollection` and this run
            // fails before `prepare`. The toggle takes effect once that is fixed.
            skip_already_imported: self.config.skip_already_imported,
            osu_client: self.library.client_type,
            osu_path: self.library.osu_path(),
            // A retry carries no collection ids to resolve, so there is nothing to
            // reuse a cached payload for.
            prefetched: HashMap::new(),
        };
        Some((new_id, request))
    }

    /// Letter keys scoped to the Downloads preview pane (a focused live run;
    /// [`Self::active_download_page_mut`] gates both). No text inputs exist
    /// here, so letter suppression never applies.
    fn handle_download_tab_key(&mut self, ch: char) -> Option<AppCommand> {
        match ch {
            // `d`/`D` deletes the entry under the cursor — a persisted history
            // record or a settled-retained session run (an in-flight run is inert:
            // cancel it with `q` first). Routes through the confirm modal unless
            // the user disabled it. Works at the list or preview level.
            'd' | 'D' => self.request_delete_download(),
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
                                .with_detail("failed mapsets were not found (404)"),
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
            // `handle_key`); a settled run just stays on the list.
            _ => None,
        }
    }

    /// `esc` as a pure "back" key. Runs after the edit/modal/login/browse
    /// cascade. Cancels an armed quit prompt; on the Downloads preview it only
    /// ascends back to the list — it never cancels a run (cancellation is `q`).
    /// It never arms or confirms a quit — quitting is `q`-only. The login split
    /// is closed earlier in the esc cascade.
    fn handle_back_key(&mut self) -> Option<AppCommand> {
        if self.home.quit_prompt {
            self.home.quit_prompt = false;
            return None;
        }
        if self.active_tab == Tab::Downloads && self.downloads_tab.preview_focused {
            self.downloads_tab.preview_focused = false;
        }
        None
    }

    fn handle_quit_key(&mut self) -> Option<AppCommand> {
        // On a descended Downloads preview, `q` is the run-control key: a running
        // run cancels, a settled run ascends back to the list. esc/← only ascend
        // (never cancel), so cancellation lives solely on `q`; the list level and
        // every other tab keep `q` as the 2-step quit.
        if self.active_tab == Tab::Downloads && self.downloads_tab.preview_focused {
            if let Some(page) = self.selected_download_page()
                && !page.is_settled()
            {
                return Some(AppCommand::CancelDownload { id: page.id });
            }
            self.downloads_tab.preview_focused = false;
            return None;
        }
        if self.home.quit_prompt {
            self.home.quit_prompt = false;
            return Some(AppCommand::Quit);
        }

        self.home.quit_prompt = true;
        debug!("Quit requested; showing confirmation prompt");
        None
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

/// Load the persisted failed-maps file at `path` and intersect its ids with
/// `collection_ids`. Returns the intersection.
pub(crate) fn intersect_failed_ids(path: &Path, collection_ids: &HashSet<u32>) -> HashSet<u32> {
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
#[path = "../../tests/unit/downloads_keys.rs"]
mod downloads_keys_tests;
