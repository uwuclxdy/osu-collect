use super::{
    custom_mirrors::CustomMirrorList, first_field, last_field, messages::AppMessage, next_field,
    prev_field,
};
use crate::{
    app::runtime::ProbeResult,
    config::Config,
    download::{ArchiveValidation, DownloadConfig, DownloadRequest},
    mirrors::{Mirror, MirrorKind},
    osu_db::OsuClient,
    utils::{CompletionResult, complete_dir, expand_tilde, pretty_path},
};
use std::{collections::HashMap, env, str::FromStr};

/// Indicates what the collection-resolve row should look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveState {
    Loading,
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub struct InputField {
    pub label: &'static str,
    pub value: String,
    pub placeholder: String,
    /// Caret position as a **char index** into `value` (not a byte offset).
    /// Invariant: `0 ..= value.chars().count()`. Char indices keep the caret
    /// math aligned with the renderer, which measures columns in chars.
    caret: usize,
}

impl InputField {
    /// Build a field with the caret parked at the end of `value`.
    pub fn new(
        label: &'static str,
        value: impl Into<String>,
        placeholder: impl Into<String>,
    ) -> Self {
        let value = value.into();
        let caret = value.chars().count();
        Self {
            label,
            value,
            placeholder: placeholder.into(),
            caret,
        }
    }

    /// Current caret position, clamped to the value length.
    pub fn caret(&self) -> usize {
        self.caret.min(self.value.chars().count())
    }

    /// Byte offset of the caret, for slicing/inserting without splitting a
    /// multi-byte char.
    fn caret_byte(&self) -> usize {
        char_to_byte(&self.value, self.caret())
    }

    /// Replace the value and park the caret at its end. Every programmatic
    /// write (tab-completion, stepper, client-path detection) routes through
    /// here so the caret never lands mid-char or past the end.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.caret = self.value.chars().count();
    }

    /// Insert `ch` at the caret and advance the caret past it.
    pub(crate) fn insert_char(&mut self, ch: char) {
        let byte = self.caret_byte();
        self.value.insert(byte, ch);
        self.caret = self.caret() + 1;
    }

    /// Insert a pasted string at the caret, advancing the caret past it.
    /// Control characters (newlines, tabs, etc.) are dropped so a multi-line
    /// paste collapses into the single-line value the fields expect.
    pub(crate) fn insert_str(&mut self, text: &str) {
        let cleaned: String = text.chars().filter(|ch| !ch.is_control()).collect();
        if cleaned.is_empty() {
            return;
        }
        let byte = self.caret_byte();
        let added = cleaned.chars().count();
        self.value.insert_str(byte, &cleaned);
        self.caret = self.caret() + added;
    }

    /// Delete the char before the caret, moving the caret back one. No-op at
    /// the start of the value.
    pub(crate) fn delete_before_caret(&mut self) {
        let caret = self.caret();
        if caret == 0 {
            return;
        }
        let start = char_to_byte(&self.value, caret - 1);
        let end = char_to_byte(&self.value, caret);
        self.value.replace_range(start..end, "");
        self.caret = caret - 1;
    }

    /// Delete the char at the caret, leaving the caret in place. No-op at the
    /// end of the value.
    pub(crate) fn delete_at_caret(&mut self) {
        let caret = self.caret();
        let len = self.value.chars().count();
        if caret >= len {
            return;
        }
        let start = char_to_byte(&self.value, caret);
        let end = char_to_byte(&self.value, caret + 1);
        self.value.replace_range(start..end, "");
        self.caret = caret;
    }

    /// Delete the word immediately left of the caret (path/URL friendly),
    /// moving the caret to the deletion start.
    pub(crate) fn delete_word_before_caret(&mut self) {
        let caret = self.caret();
        self.caret = crate::utils::delete_word_left(&mut self.value, caret);
    }

    /// Move the caret one char left.
    pub(crate) fn caret_left(&mut self) {
        self.caret = self.caret().saturating_sub(1);
    }

    /// Move the caret one char right, clamped to the value length.
    pub(crate) fn caret_right(&mut self) {
        self.caret = (self.caret() + 1).min(self.value.chars().count());
    }

    /// Move the caret to the start of the value.
    pub(crate) fn caret_home(&mut self) {
        self.caret = 0;
    }

    /// Move the caret to the end of the value.
    pub(crate) fn caret_end(&mut self) {
        self.caret = self.value.chars().count();
    }
}

/// Byte offset of char index `idx` in `s`, or `s.len()` when `idx` is at or
/// past the end. Never splits a multi-byte char.
fn char_to_byte(s: &str, idx: usize) -> usize {
    s.char_indices()
        .nth(idx)
        .map(|(byte, _)| byte)
        .unwrap_or(s.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeField {
    Collection,
    Directory,
    /// One custom-mirror URL row, indexed into [`CustomMirrorList`]. The last
    /// index is always the empty "add new" entry slot.
    CustomMirror(usize),
    MirrorNerinyan,
    MirrorOsuDirect,
    MirrorSayobot,
    MirrorNekoha,
    MirrorBeatconnect,
    MirrorOsudl,
    MirrorCatboy,
    MirrorHinamizawa,
    MirrorOsuOfficial,
    Threads,
    AutoOverwrite,
    Video,
    /// The "start download" button; activated with `enter`.
    Download,
}

// The download-section fields, in render order, that trail the mirror rows —
// collection · (custom mirrors) · builtin mirrors · these. The built-in mirror
// rows sit just above, in the configured try-order (`ordered_mirror_rows`), so
// they are not listed here. The download directory is at the top of this tail
// (just below the last mirror), the start button at the bottom.
const HOME_FIELDS_TAIL: &[HomeField] = &[
    HomeField::Directory,
    HomeField::Threads,
    HomeField::AutoOverwrite,
    HomeField::Video,
    HomeField::Download,
];

impl HomeField {
    pub fn is_text_input(self) -> bool {
        matches!(
            self,
            HomeField::Collection | HomeField::Directory | HomeField::CustomMirror(_)
        )
    }

    pub fn is_stepper(self) -> bool {
        self == HomeField::Threads
    }

    /// Whether `enter` toggles this field (mirror/option checkboxes).
    pub fn is_toggle(self) -> bool {
        matches!(
            self,
            HomeField::MirrorNerinyan
                | HomeField::MirrorOsuDirect
                | HomeField::MirrorSayobot
                | HomeField::MirrorNekoha
                | HomeField::MirrorBeatconnect
                | HomeField::MirrorOsudl
                | HomeField::MirrorCatboy
                | HomeField::MirrorHinamizawa
                | HomeField::MirrorOsuOfficial
                | HomeField::AutoOverwrite
                | HomeField::Video
        )
    }
}

pub struct HomeTab {
    pub collection: InputField,
    pub directory: InputField,
    pub custom_mirrors: CustomMirrorList,
    pub threads: InputField,
    pub auto_overwrite: bool,
    pub nerinyan: bool,
    pub osu_direct: bool,
    pub sayobot: bool,
    pub nekoha: bool,
    pub beatconnect: bool,
    pub osudl: bool,
    pub catboy: bool,
    pub hinamizawa: bool,
    pub osu_official: bool,
    /// Built-in mirror try-order, seeded from
    /// [`MirrorConfig::ordered_builtins`](crate::config::MirrorConfig::ordered_builtins)
    /// and kept in sync with the Config tab via [`set_mirror_order`](Self::set_mirror_order).
    /// Drives the mirror-row render, the nav order, and the pipeline try-order.
    pub mirror_order: Vec<MirrorKind>,
    pub video: bool,
    pub focus: HomeField,
    pub message: Option<AppMessage>,
    /// Resolve status shown below the collection URL field.
    /// Unlike `message`, this is not TTL-expired; it persists until the field changes.
    pub collection_resolve: Option<(ResolveState, String)>,
    /// Cache of the last successfully resolved collection: `(id, beatmapset_ids)`.
    /// Used by `App::request_download` to intersect with the persisted
    /// failed-maps file before dispatching the pipeline.
    pub resolved_collection: Option<(u32, Vec<u32>)>,
    /// Per-collection subfolder (`Collection::folder_name`) the resolved
    /// collection downloads into, e.g. `"my collection-1234"`. `None` until a
    /// collection resolves. Display-only: powers the download-directory tooltip
    /// so the user sees the exact folder that will be created.
    pub resolved_folder_name: Option<String>,
    /// Latency probe results per built-in mirror. `None` = not yet probed,
    /// `Some(None)` = probe in flight (`…`), `Some(Some(_))` = result received.
    pub mirror_latency: HashMap<MirrorKind, Option<ProbeResult>>,
    pub quit_prompt: bool,
    pub default_threads: u8,
    default_directory: String,
    /// Persisted list scroll offset so the focused row isn't re-pinned to the
    /// panel's bottom edge every frame (see [`widgets::render_list`]).
    pub list_offset: std::cell::Cell<usize>,
}

impl HomeTab {
    pub fn new(config: &Config) -> Self {
        let nerinyan = config.mirror.nerinyan;
        let osu_direct = config.mirror.osu_direct;
        let sayobot = config.mirror.sayobot;
        let nekoha = config.mirror.nekoha;
        let beatconnect = config.mirror.beatconnect;
        let osudl = config.mirror.osudl;
        let catboy = config.mirror.catboy;
        let hinamizawa = config.mirror.hinamizawa;
        let osu_official = config.mirror.osu_official;
        let custom_templates = config.mirror.custom_templates();

        // One syscall: raw form for submit fallback, pretty form for placeholder.
        let cwd = env::current_dir();
        let default_directory = cwd
            .as_deref()
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string());
        // Placeholder shows the tilde-collapsed path so long cwd is readable.
        let placeholder_directory = cwd
            .as_deref()
            .map(|dir| pretty_path(dir).into_owned())
            .unwrap_or_else(|_| ".".to_string());

        let default_threads = config.download.resolved_concurrent();
        let threads_value = config
            .download
            .concurrent
            .map(|value| value.to_string())
            .unwrap_or_default();

        // Pre-fill the collection field and download directory with the last
        // values the user downloaded, so a repeat run is a single keypress.
        let last_collection = config.recent.collection.clone().unwrap_or_default();
        let last_directory = config.recent.directory.clone().unwrap_or_default();

        Self {
            collection: InputField::new(
                "Collection URL or ID",
                last_collection,
                "https://osucollector.com/collections/...",
            ),
            directory: InputField::new("Download directory", last_directory, placeholder_directory),
            custom_mirrors: CustomMirrorList::from_templates(&custom_templates),
            threads: InputField::new("threads", threads_value, default_threads.to_string()),
            auto_overwrite: false,
            nerinyan,
            osu_direct,
            sayobot,
            nekoha,
            beatconnect,
            osudl,
            catboy,
            hinamizawa,
            osu_official,
            mirror_order: config.mirror.ordered_builtins(),
            video: config.download.video,
            focus: HomeField::Collection,
            message: None,
            collection_resolve: None,
            resolved_collection: None,
            resolved_folder_name: None,
            mirror_latency: HashMap::with_capacity(MirrorKind::BUILTINS.len()),
            quit_prompt: false,
            default_threads,
            default_directory,
            list_offset: std::cell::Cell::new(0),
        }
    }

    /// Mark all built-in mirrors as "probe in flight" (`…`).
    pub fn mirror_probe_started(&mut self) {
        for kind in MirrorKind::BUILTINS {
            self.mirror_latency.insert(*kind, None);
        }
    }

    /// Store the result for a single mirror.
    pub fn set_mirror_latency(&mut self, kind: MirrorKind, result: ProbeResult) {
        self.mirror_latency.insert(kind, Some(result));
    }

    pub fn clear_collection_resolve(&mut self) {
        self.collection_resolve = None;
        self.resolved_collection = None;
        self.resolved_folder_name = None;
    }

    pub fn set_collection_resolve(&mut self, state: ResolveState, text: impl Into<String>) {
        self.collection_resolve = Some((state, text.into()));
    }

    /// Cache the resolved beatmapset id list for the current collection. Read
    /// by `App::request_download` to intersect with persisted failures.
    pub fn set_resolved_collection(&mut self, collection_id: u32, beatmapset_ids: Vec<u32>) {
        self.resolved_collection = Some((collection_id, beatmapset_ids));
    }

    /// The download directory to persist as "last used" — the raw typed value,
    /// or the default (cwd) when the field is left empty. Mirrors the fallback
    /// in [`build_request`](Self::build_request) so the prefill matches where the
    /// download actually went, even when the user never types a path.
    pub fn persisted_directory(&self) -> &str {
        let typed = self.directory.value.trim();
        if typed.is_empty() {
            self.default_directory.trim()
        } else {
            typed
        }
    }

    /// The absolute download directory a download would use right now: the typed
    /// value with a leading `~` expanded, or `default_directory` when the field
    /// is blank. Mirrors the resolution in [`build_request`](Self::build_request)
    /// so a directory-field tooltip can show exactly where maps will land.
    pub fn resolved_directory(&self) -> String {
        let typed = self.directory.value.trim();
        if typed.is_empty() {
            self.default_directory.clone()
        } else {
            expand_tilde(typed)
        }
    }

    /// Full focus order with one [`HomeField::CustomMirror`] row per custom
    /// entry (including the trailing empty slot), built fresh each call so the
    /// dynamic custom-mirror count is always reflected.
    fn fields(&self) -> Vec<HomeField> {
        let mirror_rows = self.ordered_mirror_rows();
        let mut fields = Vec::with_capacity(
            1 + self.custom_mirrors.row_count() + mirror_rows.len() + HOME_FIELDS_TAIL.len(),
        );
        fields.push(HomeField::Collection);
        for idx in 0..self.custom_mirrors.row_count() {
            fields.push(HomeField::CustomMirror(idx));
        }
        fields.extend(mirror_rows.iter().map(|&(_, field, _)| field));
        fields.extend_from_slice(HOME_FIELDS_TAIL);
        fields
    }

    /// Built-in mirror rows in the configured try-order: `(kind, nav field,
    /// enabled)`. Single source for the nav order ([`fields`](Self::fields)) and
    /// the Get Maps mirror render, so the two never drift.
    pub(crate) fn ordered_mirror_rows(&self) -> Vec<(MirrorKind, HomeField, bool)> {
        self.mirror_order
            .iter()
            .filter_map(|&kind| Some((kind, mirror_home_field(kind)?, self.mirror_enabled(kind))))
            .collect()
    }

    /// Adopt a new built-in try-order (e.g. after the Config tab reorders it) so
    /// the Get Maps rows and the pipeline follow the same order without a
    /// relaunch. Enable state is untouched.
    pub fn set_mirror_order(&mut self, order: Vec<MirrorKind>) {
        self.mirror_order = order;
    }

    /// Drop emptied custom rows once focus leaves the custom-mirror section, so a
    /// cleared row disappears without shifting focus mid-edit.
    fn settle_custom_on_leave(&mut self, old: HomeField, new: HomeField) {
        if matches!(old, HomeField::CustomMirror(_)) && !matches!(new, HomeField::CustomMirror(_)) {
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

    /// Run tab-completion on the directory input field.
    ///
    /// Only acts when focus is `HomeField::Directory`. On a single match the
    /// value is completed in-place. On multiple matches the value is completed
    /// to the longest common prefix and the candidates are returned for the
    /// caller to surface as an info toast. On no match nothing changes.
    pub fn tab_complete_directory(&mut self) -> Option<String> {
        if self.focus != HomeField::Directory {
            return None;
        }
        match complete_dir(&self.directory.value) {
            CompletionResult::Single(completed) => {
                self.directory.set_value(completed);
                None
            }
            CompletionResult::Ambiguous {
                completed,
                candidates,
            } => {
                self.directory.set_value(completed);
                // Show up to 5 candidates; truncate with "…" if more.
                const MAX_SHOWN: usize = 5;
                let display = if candidates.len() <= MAX_SHOWN {
                    candidates.join(", ")
                } else {
                    let shown = candidates[..MAX_SHOWN].join(", ");
                    format!("{shown}, … ({} more)", candidates.len() - MAX_SHOWN)
                };
                Some(display)
            }
            CompletionResult::NoMatch => None,
        }
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
        if let Some(field) = self.focused_input_mut() {
            field.insert_char(ch);
        }
        self.grow_custom_rows();
    }

    /// Insert a bracketed-paste payload into the focused text field. No-op when
    /// focus is on a non-text field.
    pub fn handle_paste(&mut self, text: &str) {
        if let Some(field) = self.focused_input_mut() {
            field.insert_str(text);
        }
        self.grow_custom_rows();
    }

    /// After editing a custom-mirror row, keep a trailing empty entry slot so
    /// there is always a row to type the next URL into.
    fn grow_custom_rows(&mut self) {
        if matches!(self.focus, HomeField::CustomMirror(_)) {
            self.custom_mirrors.ensure_trailing_empty();
        }
    }

    pub fn backspace(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.delete_before_caret();
        }
    }

    /// Delete the char at the caret in the focused text field (`Delete` key).
    pub fn delete_forward(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.delete_at_caret();
        }
    }

    /// Delete the word left of the caret in the focused text field
    /// (alt/ctrl+backspace).
    pub fn backspace_word(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.delete_word_before_caret();
        }
    }

    /// Move the caret in the focused text field. No-op when focus is on a
    /// non-text field.
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

    /// The focused text input, or `None` when focus is on a non-text field.
    /// Used by the renderer to place the caret.
    pub fn focused_input(&self) -> Option<&InputField> {
        match self.focus {
            HomeField::Collection => Some(&self.collection),
            HomeField::Directory => Some(&self.directory),
            HomeField::CustomMirror(idx) => self.custom_mirrors.row(idx),
            _ => None,
        }
    }

    fn focused_input_mut(&mut self) -> Option<&mut InputField> {
        match self.focus {
            HomeField::Collection => Some(&mut self.collection),
            HomeField::Directory => Some(&mut self.directory),
            HomeField::CustomMirror(idx) => self.custom_mirrors.row_mut(idx),
            _ => None,
        }
    }

    pub fn toggle_current(&mut self) {
        match self.focus {
            HomeField::MirrorNerinyan => {
                self.nerinyan = !self.nerinyan;
            }
            HomeField::MirrorOsuDirect => {
                self.osu_direct = !self.osu_direct;
            }
            HomeField::MirrorSayobot => {
                self.sayobot = !self.sayobot;
            }
            HomeField::MirrorNekoha => {
                self.nekoha = !self.nekoha;
            }
            HomeField::MirrorBeatconnect => {
                self.beatconnect = !self.beatconnect;
            }
            HomeField::MirrorOsudl => {
                self.osudl = !self.osudl;
            }
            HomeField::MirrorCatboy => {
                self.catboy = !self.catboy;
            }
            HomeField::MirrorHinamizawa => {
                self.hinamizawa = !self.hinamizawa;
            }
            HomeField::MirrorOsuOfficial => {
                self.osu_official = !self.osu_official;
            }
            HomeField::AutoOverwrite => {
                self.auto_overwrite = !self.auto_overwrite;
            }
            HomeField::Video => {
                self.video = !self.video;
            }
            _ => {}
        }
    }

    /// Count of enabled mirrors without allocating a `Vec`.
    ///
    /// Use this for display-only contexts (e.g. the summary metric in the TUI).
    /// Call `build_mirror_list` when the actual list of mirrors is needed.
    pub fn mirror_count(&self) -> usize {
        let builtin_count = self
            .mirror_order
            .iter()
            .filter(|&&kind| self.mirror_enabled(kind))
            .count();
        builtin_count + self.custom_mirrors.valid_count()
    }

    /// Whether the built-in mirror of `kind` is toggled on. Maps each
    /// [`MirrorKind`] to its backing toggle so the mirror list and count derive
    /// from the configured try-order ([`mirror_order`](Self::mirror_order) — the
    /// order the TUI renders and the download pipeline tries), and can't drift
    /// from it.
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
            MirrorKind::Custom => false,
        }
    }

    pub fn build_mirror_list(&self) -> Vec<Mirror> {
        // Built-ins follow the configured try-order (`mirror_order`) so the
        // pipeline tries them in the exact order the TUI lists them. OsuApi is
        // built header-less here; the download pipeline injects the `*`
        // (lazer-tier) bearer token + `x-api-version` header before the request
        // goes out.
        let mut mirrors: Vec<Mirror> = self
            .mirror_order
            .iter()
            .filter(|&&kind| self.mirror_enabled(kind))
            .filter_map(|&kind| {
                let mirror = Mirror::builtin(kind)?;
                Some(if self.video {
                    mirror
                } else {
                    mirror.no_video()
                })
            })
            .collect();

        mirrors.extend(self.custom_mirrors.build_mirrors(self.video));

        mirrors
    }

    pub fn build_request(
        &self,
        archive_validation: ArchiveValidation,
        auto_skip_rate_limited: bool,
        rate_limit_skip_secs: u32,
    ) -> Result<DownloadRequest, String> {
        let collection_input = self.collection.value.trim();
        if collection_input.is_empty() {
            return Err("Collection ID or URL is required".to_string());
        }

        // Expand `~` at submit time so the filesystem layer receives an absolute
        // path regardless of how the user typed the value.
        let directory = self.resolved_directory();

        let threads_value = if self.threads.value.trim().is_empty() {
            self.default_threads
        } else {
            parse_thread_count(&self.threads.value)?
        };

        if threads_value == 0 || threads_value > 100 {
            return Err("Thread count must be between 1 and 100".to_string());
        }

        let mirrors = self.build_mirror_list();
        if mirrors.is_empty() {
            return Err("Select at least one mirror".to_string());
        }

        let config = DownloadConfig {
            directory,
            mirrors,
            concurrent: threads_value,
            archive_validation,
            auto_skip_rate_limited,
            rate_limit_skip_secs,
        };

        Ok(DownloadRequest {
            collection_input: collection_input.to_string(),
            config,
            auto_overwrite: self.auto_overwrite,
            // Default `false`; `App::request_download` resolves the
            // retry-failed-on-download policy and overrides it (or surfaces a
            // modal under `Ask` before the download is dispatched).
            include_previously_failed: false,
            // Placeholders; `App::request_download` fills these from the live
            // config + Updates-tab client/path before the request is dispatched.
            skip_already_imported: false,
            osu_client: OsuClient::default(),
            osu_path: String::new(),
        })
    }

    pub fn resolved_threads(&self) -> u8 {
        if self.threads.value.trim().is_empty() {
            self.default_threads
        } else {
            parse_thread_count(&self.threads.value).unwrap_or(self.default_threads)
        }
    }
}

fn parse_thread_count(value: &str) -> Result<u8, String> {
    u8::from_str(value.trim()).map_err(|_| "Thread count must be between 1 and 100".to_string())
}

/// The nav/render field for a built-in mirror kind, or `None` for
/// [`MirrorKind::Custom`] (custom mirrors live in their own rows).
fn mirror_home_field(kind: MirrorKind) -> Option<HomeField> {
    Some(match kind {
        MirrorKind::Nerinyan => HomeField::MirrorNerinyan,
        MirrorKind::OsuDirect => HomeField::MirrorOsuDirect,
        MirrorKind::Sayobot => HomeField::MirrorSayobot,
        MirrorKind::Nekoha => HomeField::MirrorNekoha,
        MirrorKind::Beatconnect => HomeField::MirrorBeatconnect,
        MirrorKind::Osudl => HomeField::MirrorOsudl,
        MirrorKind::Catboy => HomeField::MirrorCatboy,
        MirrorKind::Hinamizawa => HomeField::MirrorHinamizawa,
        MirrorKind::OsuApi => HomeField::MirrorOsuOfficial,
        MirrorKind::Custom => return None,
    })
}

#[cfg(test)]
#[path = "../../tests/unit/home.rs"]
mod tests;
