use super::{
    collection_cache::CollectionCache,
    custom_mirrors::CustomMirrorList,
    find_source::{FindSource, FindStatusMsg, SetBrowse},
    first_field, last_field,
    messages::AppMessage,
    next_field, prev_field,
    update_source::{ScanCta, UpdateSource},
};
use crate::{
    app::runtime::ProbeResult,
    config::{Config, MirrorConfig},
    download::{
        ArchiveValidation, DownloadConfig, DownloadRequest, IdsRunSource, ids_folder_name,
        selective_folder_name,
    },
    mirrors::{Mirror, MirrorKind},
    osu_db::OsuClient,
    utils::{CompletionResult, complete_dir, expand_tilde, parse_collection_id, pretty_path},
};
use osu_downloader::search::BeatmapSetMeta;
use std::{
    collections::{HashMap, HashSet},
    env,
    str::FromStr,
};

/// Indicates what the collection-resolve row should look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveState {
    Loading,
    Success,
    Error,
}

/// Which get-maps source the tab is showing. The strip is the first focusable
/// row; `space`/`enter` cycle it, digits jump to it. All three sources are wired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetMapsSource {
    /// Discovery: one union criteria form auto-routed to an osu! api text search
    /// or an nzbasic attribute filter by [`FindSource::build_plan`](crate::app::FindSource::build_plan).
    Find,
    Collection,
    Update,
}

impl GetMapsSource {
    /// Strip order, left to right.
    pub const ALL: [GetMapsSource; 3] = [
        GetMapsSource::Find,
        GetMapsSource::Collection,
        GetMapsSource::Update,
    ];

    /// Lowercase strip label.
    pub fn label(self) -> &'static str {
        match self {
            GetMapsSource::Find => "find",
            GetMapsSource::Collection => "collection",
            GetMapsSource::Update => "update",
        }
    }

    /// The next source `forward` (right) or backward (left) along [`ALL`](Self::ALL),
    /// wrapping at the ends.
    fn cycled(self, forward: bool) -> Self {
        let idx = Self::ALL.iter().position(|&s| s == self).unwrap_or(0);
        let len = Self::ALL.len();
        let next = if forward { idx + 1 } else { idx + len - 1 };
        Self::ALL[next % len]
    }
}

/// Which backend a [`GetMapsSource::Find`] run actually hit. Not a UI control —
/// [`FindSource::build_plan`](crate::app::FindSource::build_plan) resolves the
/// route per-criterion; this only tags the loaded results
/// ([`FindSource::results_backend`](crate::app::FindSource::results_backend)) so
/// `m` / the download route by the true fetch backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindBackend {
    /// osu! api v2 text search.
    Osu,
    /// nzbasic BBD attribute filter.
    Nzbasic,
}

impl FindBackend {
    /// The backend's user-facing name. One spelling for every surface that names
    /// a route to the user — the form's `→ via <backend>` indicator and the toast
    /// that fires when the criteria move off the loaded results' backend read it
    /// from here, so the cue can't call it something the indicator doesn't.
    pub fn label(self) -> &'static str {
        match self {
            Self::Osu => "osu! api",
            Self::Nzbasic => "nzbasic",
        }
    }
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
    /// The source strip (find / collection / update). First focusable row;
    /// `space`/`enter` cycle the active source (arrows switch tabs), digits jump.
    Source,
    Collection,
    /// Read-only count of enabled mirrors; `enter` jumps to the Config tab's
    /// mirrors section, which owns all mirror editing (toggle / custom / order).
    Mirrors,
    Directory,
    Threads,
    AutoOverwrite,
    Video,
    /// The per-source download button; activated with `enter`. Dispatched by
    /// `dispatch_form_download`: collection downloads all (or the browse&pick
    /// subset), search the picked results, update the checked collections.
    Download,
    /// The collection source's `view N maps` CTA — opens the resolved collection
    /// in a checkbox browse to download a subset. Enabled only once a collection
    /// has resolved.
    CollectionBrowse,
    /// The update source's osu! path input. A text field editing
    /// [`App.library`](crate::app::App::library)'s path rather than a `HomeTab`
    /// field, so its text ops route through `library` in the app.
    UpdateOsuPath,
    /// The update source's scan CTA (`scan for updates` / `rescan`).
    UpdateScan,
    /// The update source's `view N maps` button — opens the two-pane browse over
    /// the scan's missing sets. Enabled once a scan finds updates.
    UpdateBrowse,
    /// The find source's free-text query input (osu-only — a non-empty value
    /// forces the osu route).
    FindQuery,
    /// The find source's preset seed-macro chip (`space`/`enter` cycle it; each
    /// step resets + seeds the criteria fields).
    FindPreset,
    /// The find source's special-tag chip (farm / stream / ranked mapper —
    /// nzbasic-only flags; a non-`none` value forces nzbasic).
    FindSpecial,
    /// The find source's game-mode chip.
    FindMode,
    /// The find source's rank-status chip.
    FindStatus,
    /// The find source's sort chip (curated field+order presets).
    FindSort,
    /// The find source's explicit-content chip (any / hide / show). Supporter-only
    /// — see [`HomeField::is_supporter_only`] — and sits in the main `filters`
    /// block beside the other facets the osu! website groups it with.
    FindExplicit,
    /// The find source's supporter-only single-select facets, behind the
    /// `advanced filters` disclosure.
    FindGenre,
    FindLanguage,
    /// The find source's supporter-only MULTI-select facets: several chips can be
    /// on at once (`e=video.storyboard`, `r=XH.X`). `↵` descends into the row,
    /// where `←`/`→` walk the chip cursor and `space` toggles the chip under it.
    FindExtra,
    FindRank,
    /// The find source's supporter-only play-state facet (any / played /
    /// unplayed), scoped to the logged-in account.
    FindPlayed,
    /// The find source's `advanced filters` disclosure: expands/collapses the 13
    /// per-attribute range inputs. `space`/`enter` toggle it; collapsed by
    /// default so the primary form (query + chips + find) fits on one screen.
    FindAdvanced,
    /// The find source's min-max range inputs (per-diff attributes).
    FindStars,
    FindAr,
    FindCs,
    FindOd,
    FindHp,
    FindBpm,
    FindLength,
    /// The find source's mania key-count / favourite-count ranges and ranked
    /// date range (osu-only — each forces the osu route when set).
    FindKeys,
    FindFavourites,
    FindRanked,
    /// The find source's substring text inputs.
    FindArtist,
    FindCreator,
    FindTitle,
    /// The find source's diff-row limit input (nzbasic-route-only; default 500).
    FindLimit,
    /// The find source's CTA button — dispatches the resolved plan (osu search
    /// or nzbasic filter).
    FindRun,
    /// The find source's `view N maps` button — reopens the results browse
    /// without re-fetching. Enabled once fresh results are loaded.
    FindBrowse,
}

/// Focus order when the collection source is active: the source strip, the
/// collection URL field, then its `view N maps` browse button — grouped with the
/// collection like find/update group their browse button with the run/scan CTA —
/// then the shared download section and the `Download` button. Mirror editing
/// lives on the Config tab, so no per-mirror rows appear here.
const COLLECTION_FIELDS: &[HomeField] = &[
    HomeField::Source,
    HomeField::Collection,
    HomeField::CollectionBrowse,
    HomeField::Mirrors,
    HomeField::Directory,
    HomeField::Threads,
    HomeField::AutoOverwrite,
    HomeField::Video,
    HomeField::Download,
];

/// Find-source focus order: the strip, the free-text query box, the `preset`
/// chip, the `filters` chips (mode → categories → explicit → special), the
/// `results` rows (sort → limit), the `advanced filters` disclosure, [the five
/// supporter facets + the 13 per-attribute range inputs when expanded], the
/// `find` / `view N maps` CTAs, then the shared
/// download section (mirrors / directory / threads / overwrite / video) and its
/// `Download` button. Mirrors the rendered order section for section — a
/// tab order that disagrees with the eyebrows is the bug this pairing prevents.
/// One union list — the resolved backend is an implementation detail, so there
/// is no per-backend field split. The download section is the same run settings
/// every source shares (`self.home.*`), rendered inline on all three. Descending
/// into the results browse suspends this nav (`SetBrowse::descend`); the
/// download fires from `Download`.
const FIND_FIELDS: &[HomeField] = &[
    HomeField::Source,
    HomeField::FindQuery,
    HomeField::FindPreset,
    HomeField::FindMode,
    HomeField::FindStatus,
    HomeField::FindExplicit,
    HomeField::FindSpecial,
    HomeField::FindSort,
    HomeField::FindLimit,
    HomeField::FindAdvanced,
    HomeField::FindGenre,
    HomeField::FindLanguage,
    HomeField::FindExtra,
    HomeField::FindRank,
    HomeField::FindPlayed,
    HomeField::FindStars,
    HomeField::FindAr,
    HomeField::FindCs,
    HomeField::FindOd,
    HomeField::FindHp,
    HomeField::FindBpm,
    HomeField::FindLength,
    HomeField::FindKeys,
    HomeField::FindFavourites,
    HomeField::FindRanked,
    HomeField::FindArtist,
    HomeField::FindCreator,
    HomeField::FindTitle,
    HomeField::FindRun,
    HomeField::FindBrowse,
    HomeField::Mirrors,
    HomeField::Directory,
    HomeField::Threads,
    HomeField::AutoOverwrite,
    HomeField::Video,
    HomeField::Download,
];

/// Find-source focus order with the `advanced filters` disclosure collapsed: the
/// five supporter facets and the 13 per-attribute range inputs are skipped so
/// navigation stays on the primary form. [`HomeTab::active_fields`] picks between
/// this and [`FIND_FIELDS`] based on the disclosure state, then drops the
/// supporter-only rows from whichever it picked — the two dimensions are
/// independent, and a row hidden on either must not be tab-reachable.
const FIND_FIELDS_COLLAPSED: &[HomeField] = &[
    HomeField::Source,
    HomeField::FindQuery,
    HomeField::FindPreset,
    HomeField::FindMode,
    HomeField::FindStatus,
    HomeField::FindExplicit,
    HomeField::FindSpecial,
    HomeField::FindSort,
    HomeField::FindLimit,
    HomeField::FindAdvanced,
    HomeField::FindRun,
    HomeField::FindBrowse,
    HomeField::Mirrors,
    HomeField::Directory,
    HomeField::Threads,
    HomeField::AutoOverwrite,
    HomeField::Video,
    HomeField::Download,
];

/// Focus order for the update source form: the strip, the osu! path input, the
/// scan CTA, the `view N maps` button, then the shared download section
/// (mirrors / directory / threads / overwrite / video) and its `Download`
/// button. Descending into the browse suspends this nav (the app gates it on
/// `HomeTab.update.is_browsing()`); the download fires from `Download`.
const UPDATE_FIELDS: &[HomeField] = &[
    HomeField::Source,
    HomeField::UpdateOsuPath,
    HomeField::UpdateScan,
    HomeField::UpdateBrowse,
    HomeField::Mirrors,
    HomeField::Directory,
    HomeField::Threads,
    HomeField::AutoOverwrite,
    HomeField::Video,
    HomeField::Download,
];

/// Stand-in for the folder name of a collection that has not resolved yet.
const PLACEHOLDER_COLLECTION: &str = "<collection>";
/// Stand-in while no collection is checked on the update source. Keeps the
/// `update-` prefix visible so the shape of the destination still reads.
const PLACEHOLDER_UPDATE: &str = "update-<collection>";

impl HomeField {
    pub fn is_text_input(self) -> bool {
        matches!(
            self,
            HomeField::Collection | HomeField::Directory | HomeField::UpdateOsuPath
        ) || self.is_find_input()
    }

    /// The find source's text-editable fields (free-text query, ranges, texts,
    /// limit).
    pub fn is_find_input(self) -> bool {
        matches!(
            self,
            HomeField::FindQuery
                | HomeField::FindStars
                | HomeField::FindAr
                | HomeField::FindCs
                | HomeField::FindOd
                | HomeField::FindHp
                | HomeField::FindBpm
                | HomeField::FindLength
                | HomeField::FindKeys
                | HomeField::FindFavourites
                | HomeField::FindRanked
                | HomeField::FindArtist
                | HomeField::FindCreator
                | HomeField::FindTitle
                | HomeField::FindLimit
        )
    }

    pub fn is_stepper(self) -> bool {
        self == HomeField::Threads
    }

    /// Whether `enter` toggles this field (the boolean option checkboxes).
    pub fn is_toggle(self) -> bool {
        matches!(self, HomeField::AutoOverwrite | HomeField::Video)
    }

    /// Whether this is a find-source SINGLE-select chip that `space`/`enter`
    /// cycle. Disjoint from [`is_find_multi_chip`](Self::is_find_multi_chip),
    /// whose rows toggle one member instead of stepping the whole row.
    pub fn is_find_chip(self) -> bool {
        matches!(
            self,
            HomeField::FindPreset
                | HomeField::FindSpecial
                | HomeField::FindMode
                | HomeField::FindStatus
                | HomeField::FindSort
                | HomeField::FindExplicit
                | HomeField::FindGenre
                | HomeField::FindLanguage
                | HomeField::FindPlayed
        )
    }

    /// Whether this is a find-source MULTI-select chip row: several members can
    /// be on at once, so the row carries its own chip cursor, reached by
    /// descending into it on `↵`.
    ///
    /// The descent is what makes the cursor legal: `←`/`→` only walk it while
    /// the row is down, the same suspension a focused text input takes for its
    /// caret. At rest they switch tabs on these rows like everywhere else.
    pub fn is_find_multi_chip(self) -> bool {
        matches!(self, HomeField::FindExtra | HomeField::FindRank)
    }

    /// Whether this row is one of the six osu!supporter-gated facets. Every one
    /// of them was only honoured for a supporter token when probed against the
    /// live API, so they are hidden — not disabled — for anyone else, per the
    /// design language's login-gated-section rule. Unknown supporter status
    /// reads as `false`, so the rows stay hidden until it is confirmed.
    pub fn is_supporter_only(self) -> bool {
        matches!(
            self,
            HomeField::FindExplicit
                | HomeField::FindGenre
                | HomeField::FindLanguage
                | HomeField::FindExtra
                | HomeField::FindRank
                | HomeField::FindPlayed
        )
    }

    /// Whether this is a disclosure row that `space`/`enter` expand/collapse.
    pub fn is_disclosure(self) -> bool {
        matches!(self, HomeField::FindAdvanced)
    }

    /// Whether this is one of the fields gated behind the `advanced filters`
    /// disclosure — the five supporter facets that live there plus the 13
    /// per-attribute inputs. Drives auto-expand: the section stays open while
    /// focus rests on an advanced field so the user can never be "stuck" focusing
    /// an invisible row. `FindExplicit` is supporter-gated but NOT advanced: it
    /// renders in the main `filters` block.
    pub fn is_advanced(self) -> bool {
        matches!(
            self,
            HomeField::FindGenre
                | HomeField::FindLanguage
                | HomeField::FindExtra
                | HomeField::FindRank
                | HomeField::FindPlayed
                | HomeField::FindStars
                | HomeField::FindAr
                | HomeField::FindCs
                | HomeField::FindOd
                | HomeField::FindHp
                | HomeField::FindBpm
                | HomeField::FindLength
                | HomeField::FindKeys
                | HomeField::FindFavourites
                | HomeField::FindRanked
                | HomeField::FindArtist
                | HomeField::FindCreator
                | HomeField::FindTitle
        )
    }

    /// Whether this field renders as a self-styling CTA button (`button_item`),
    /// which paints its own pill fill. The list's row-wide `bg_hover` highlight
    /// must be suppressed on such a row, else the tint doubles up with the pill
    /// (the broken box). Drives the `highlight` arg of every Get Maps
    /// `render_scrollable_panel`, so a new button can never forget to opt out.
    pub fn is_button(self) -> bool {
        matches!(
            self,
            HomeField::Download
                | HomeField::CollectionBrowse
                | HomeField::UpdateScan
                | HomeField::UpdateBrowse
                | HomeField::FindRun
                | HomeField::FindBrowse
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
    pub nzbasic: bool,
    /// Built-in mirror try-order, seeded from
    /// [`MirrorConfig::ordered_builtins`](crate::config::MirrorConfig::ordered_builtins)
    /// and kept in sync with the Config tab via
    /// [`sync_mirrors_from_config`](Self::sync_mirrors_from_config).
    /// Drives the enabled-mirror count and the pipeline try-order.
    pub mirror_order: Vec<MirrorKind>,
    pub video: bool,
    /// Active get-maps source. Per keep-both, switching never clears another
    /// source's state (all of it lives on this struct).
    pub source: GetMapsSource,
    pub focus: HomeField,
    pub message: Option<AppMessage>,
    /// Resolve status shown below the collection URL field.
    /// Unlike `message`, this is not TTL-expired; it persists until the field changes.
    pub collection_resolve: Option<(ResolveState, String)>,
    /// Cache of the last successfully resolved collection: `(id, beatmapset_ids)`.
    /// Used by `App::request_download` to intersect with the persisted
    /// failed-maps file before dispatching the pipeline.
    pub resolved_collection: Option<(u32, Vec<u32>)>,
    /// One `(set_id, diff_id)` pair per unique set of the resolved collection,
    /// paired with `resolved_collection` (same resolve). Seeds the browse&pick
    /// enrichment pager so its id-only previews backfill from the osu-batch
    /// endpoint; the set id lets a seed be pruned against
    /// [`meta_cache`](Self::meta_cache) when a title is already known.
    pub resolved_enrich_pairs: Vec<(u32, u32)>,
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
    /// State for the [`GetMapsSource::Update`] source (the former Updates tab).
    pub update: UpdateSource,
    /// The find source's union criteria form + results browse (both backends land
    /// their results here). Kept on `HomeTab` so it survives source switches
    /// (keep-both).
    pub find: FindSource,
    /// Collection browse&pick browse state (the collection source's
    /// `view N maps` CTA). Fed from [`resolved_collection`](Self::resolved_collection);
    /// separate from `find.browse` so each source's selection persists.
    pub collection_browse: SetBrowse,
    /// The collection id snapshotted when the browse&pick browse was opened, so
    /// the download dispatches against the collection the rows came from even if
    /// an in-flight resolve updates `resolved_collection` mid-browse.
    pub collection_browse_id: Option<u32>,
    /// Session-lived osu-batch metadata cache, keyed by beatmapSET id. Every
    /// landed enrichment page (and every osu search row) feeds it, and every
    /// id-only browse hydrates + prunes against it — so a reopen, rescan, or
    /// re-resolve never refetches a title the app already fetched. Never evicted.
    pub(crate) meta_cache: HashMap<u32, BeatmapSetMeta>,
    /// Session-lived osu!collector payload cache, keyed by COLLECTION id. Written
    /// by the collection resolve + the update scan (both fetch a full collection
    /// for display); read at download-request build so the pipeline reuses the
    /// payload instead of refetching it verbatim.
    pub(crate) collection_cache: CollectionCache,
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
        let nzbasic = config.mirror.nzbasic;
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
                "https://osucollector.com/collections/…",
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
            nzbasic,
            mirror_order: config.mirror.ordered_builtins(),
            video: config.download.video,
            source: GetMapsSource::Collection,
            focus: HomeField::Collection,
            message: None,
            collection_resolve: None,
            resolved_collection: None,
            resolved_enrich_pairs: Vec::new(),
            resolved_folder_name: None,
            mirror_latency: HashMap::with_capacity(MirrorKind::BUILTINS.len()),
            quit_prompt: false,
            default_threads,
            default_directory,
            list_offset: std::cell::Cell::new(0),
            update: UpdateSource::new(),
            find: FindSource::new(),
            collection_browse: SetBrowse::new(),
            collection_browse_id: None,
            meta_cache: HashMap::new(),
            collection_cache: CollectionCache::default(),
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
        self.resolved_enrich_pairs = Vec::new();
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

    /// Whether browse&pick holds a proper nonempty subset (some sets checked,
    /// but not all) **of the currently-resolved collection**. Drives the
    /// collection download button between `download all` (the whole resolved
    /// collection) and `download N selected` (the picked subset, via the
    /// selective path). Returns `false` when the browse is stale — its snapshot
    /// id no longer matches the resolved collection (a resolve moved on), so a
    /// left-over pick never mislabels or misdispatches the new collection.
    pub fn collection_subset_picked(&self) -> bool {
        let current = self.resolved_collection.as_ref().map(|(id, _)| *id);
        if self.collection_browse_id != current {
            return false;
        }
        let total = self.collection_browse.rows.len();
        let selected = self.collection_browse.selected_count();
        total > 0 && selected > 0 && selected < total
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

    /// Whether the cached resolve still describes what the collection field
    /// holds. [`schedule_resolve`] only clears the snapshot when the field turns
    /// UNPARSEABLE, so retyping one valid id to another leaves the previous
    /// collection's name and id in place for the whole debounce + fetch, and for
    /// good if that fetch fails. Every read of the snapshot goes through here.
    ///
    /// [`schedule_resolve`]: crate::app::runtime::schedule_resolve
    fn collection_resolve_is_current(&self) -> bool {
        let Some((resolved_id, _)) = self.resolved_collection.as_ref() else {
            return false;
        };
        parse_collection_id(&self.collection.value).is_ok_and(|typed| typed == *resolved_id)
    }

    /// The per-run subdir a download dispatched right now would land in, under
    /// [`resolved_directory`](Self::resolved_directory). The find and update arms
    /// call the same namer the matching `prepare_*` path uses; the collection arms
    /// replay a name the resolve already derived from `Collection::folder_name`.
    ///
    /// A placeholder stands in wherever the name is not knowable yet: the
    /// collection source until a fetch for the id currently in the field lands,
    /// the update source until a collection is checked.
    pub fn planned_folder_name(&self) -> String {
        match self.source {
            GetMapsSource::Collection if self.collection_subset_picked() => self
                .collection_browse_id
                .map(|id| selective_folder_name(&[id]))
                .unwrap_or_else(|| PLACEHOLDER_COLLECTION.to_string()),
            GetMapsSource::Collection => self
                .resolved_folder_name
                .as_ref()
                .filter(|_| self.collection_resolve_is_current())
                .cloned()
                .unwrap_or_else(|| PLACEHOLDER_COLLECTION.to_string()),
            GetMapsSource::Find => ids_folder_name(
                IdsRunSource::from(self.find.run_backend()).folder_prefix(),
                &self.find.folder_tag(),
            ),
            GetMapsSource::Update => {
                let ids: Vec<u32> = self
                    .update
                    .selected_collection_ids()
                    .into_iter()
                    .filter_map(|id| u32::try_from(id).ok())
                    .collect();
                if ids.is_empty() {
                    PLACEHOLDER_UPDATE.to_string()
                } else {
                    selective_folder_name(&ids)
                }
            }
        }
    }

    /// Adopt the Config tab's mirror settings (enable flags, try-order, custom
    /// URLs) so the Get Maps count and the download list track the sole mirror
    /// editor. Called after any Config change persists; keeps this tab from
    /// drifting now that it no longer edits mirrors itself.
    pub fn sync_mirrors_from_config(&mut self, mirror: &MirrorConfig) {
        self.nerinyan = mirror.nerinyan;
        self.osu_direct = mirror.osu_direct;
        self.sayobot = mirror.sayobot;
        self.nekoha = mirror.nekoha;
        self.beatconnect = mirror.beatconnect;
        self.osudl = mirror.osudl;
        self.catboy = mirror.catboy;
        self.hinamizawa = mirror.hinamizawa;
        self.osu_official = mirror.osu_official;
        self.nzbasic = mirror.nzbasic;
        self.mirror_order = mirror.ordered_builtins();
        self.custom_mirrors = CustomMirrorList::from_templates(&mirror.custom_templates());
    }

    /// Focusable fields for the active source. A placeholder source exposes only
    /// the strip; the collection source exposes the full form.
    ///
    /// The find source has TWO independent visibility dimensions — the `advanced
    /// filters` disclosure and the supporter gate — so the disclosure picks the
    /// list and the gate filters it. A row the render hides must not be reachable
    /// by tab, which is what makes this filter, rather than the render, the
    /// authority both read from.
    pub(crate) fn active_fields(&self, supporter: bool) -> Vec<HomeField> {
        let fields: &'static [HomeField] = match self.source {
            GetMapsSource::Collection => COLLECTION_FIELDS,
            GetMapsSource::Update => UPDATE_FIELDS,
            GetMapsSource::Find => {
                if self.find.show_advanced_filters() || self.focus.is_advanced() {
                    FIND_FIELDS
                } else {
                    FIND_FIELDS_COLLAPSED
                }
            }
        };
        fields
            .iter()
            .copied()
            .filter(|field| supporter || !field.is_supporter_only())
            .collect()
    }

    /// Move focus off a supporter row once the gate closes under it (a logout, a
    /// `/me` re-probe that comes back non-supporter). The row stops rendering the
    /// same frame, so leaving focus there would park the caret on nothing.
    ///
    /// Returns whether focus moved, which is what the caller keys any
    /// focus-scoped state (an open edit mode) off — asking here rather than
    /// re-deriving the condition beside it.
    pub fn clamp_supporter_focus(&mut self, supporter: bool) -> bool {
        if supporter || !self.focus.is_supporter_only() {
            return false;
        }
        self.focus = HomeField::Source;
        true
    }

    /// Cycle the source strip one step (`forward` = right, else left), wrapping.
    /// Focus stays on the strip, which is present in every source's field list,
    /// so no re-clamp is needed.
    pub fn cycle_source(&mut self, forward: bool) {
        self.source = self.source.cycled(forward);
    }

    pub fn next_field(&mut self, supporter: bool) {
        self.focus = next_field(&self.active_fields(supporter), self.focus);
    }

    pub fn prev_field(&mut self, supporter: bool) {
        self.focus = prev_field(&self.active_fields(supporter), self.focus);
    }

    pub fn first_field(&mut self, supporter: bool) {
        self.focus = first_field(&self.active_fields(supporter), self.focus);
    }

    pub fn last_field(&mut self, supporter: bool) {
        self.focus = last_field(&self.active_fields(supporter), self.focus);
    }

    /// Whether the form has the minimum inputs a collection download needs: a
    /// collection reference and at least one enabled mirror. Gates the collection
    /// `Download` button; final validation still happens in
    /// [`build_request`](Self::build_request) on activation.
    pub fn can_download(&self) -> bool {
        !self.collection.value.trim().is_empty() && self.mirror_count() > 0
    }

    /// Whether `field` is a form button that is currently "clickable" (enabled).
    /// Single source of truth for the button enabled-state, read by the `s`
    /// button jump/cycle ([`cycle_enabled_button`](Self::cycle_enabled_button))
    /// and the collection view's button helpers; the find/update views compute the
    /// same predicate from the same accessors. Non-button fields return `false`.
    pub fn button_enabled(&self, field: HomeField) -> bool {
        match field {
            HomeField::Download => match self.source {
                GetMapsSource::Collection => self.collection_subset_picked() || self.can_download(),
                GetMapsSource::Find => self.find.browse.selected_count() > 0,
                GetMapsSource::Update => self.update.selected_new_count() > 0,
            },
            HomeField::CollectionBrowse => self
                .resolved_collection
                .as_ref()
                .is_some_and(|(_, ids)| !ids.is_empty()),
            HomeField::FindRun => !matches!(self.find.status_msg, FindStatusMsg::Loading),
            HomeField::FindBrowse => {
                !self.find.browse.rows.is_empty() && self.find.results_current()
            }
            HomeField::UpdateScan => self.update.scan_cta() != ScanCta::Busy,
            HomeField::UpdateBrowse => self.update.total_new_count() > 0,
            _ => false,
        }
    }

    /// The active source's primary CTA — the furthest-along *enabled* action
    /// button in field order (`find`/`scan` → `view N maps` → `download`), so
    /// the eye is drawn to the next actionable step rather than always to the
    /// terminal `download`. Falls back to [`HomeField::Download`] when none are
    /// enabled (every action button is faint then anyway, so the pinned primary
    /// doesn't shout). This is the `None`-focus arm of
    /// [`cycle_enabled_button`](Self::cycle_enabled_button); the render reads it
    /// to pick each button's `ButtonProminence`.
    pub fn primary_action_field(&self, supporter: bool) -> HomeField {
        self.active_fields(supporter)
            .iter()
            .copied()
            .filter(|&field| field.is_button() && self.button_enabled(field))
            .last()
            .unwrap_or(HomeField::Download)
    }

    /// Focus target for a press of `s`, over the active source's enabled
    /// ("clickable") buttons in field order. When focus is **not** already on one,
    /// the *last* enabled button — the furthest-along CTA
    /// (`find`/`scan` → `view N maps` → `download`), falling back to the
    /// always-present `Download` button when none are enabled, so the jump always
    /// lands somewhere predictable. When focus **is** on an enabled button, the
    /// *next* enabled button (wrapping), so repeated `s` cycles the other
    /// available buttons.
    pub fn cycle_enabled_button(&self, supporter: bool) -> HomeField {
        let buttons: Vec<HomeField> = self
            .active_fields(supporter)
            .iter()
            .copied()
            .filter(|&field| field.is_button() && self.button_enabled(field))
            .collect();
        match buttons.iter().position(|&field| field == self.focus) {
            Some(idx) => buttons[(idx + 1) % buttons.len()],
            None => buttons.last().copied().unwrap_or(HomeField::Download),
        }
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
    }

    /// Insert a bracketed-paste payload into the focused text field. No-op when
    /// focus is on a non-text field.
    pub fn handle_paste(&mut self, text: &str) {
        if let Some(field) = self.focused_input_mut() {
            field.insert_str(text);
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
            HomeField::FindQuery => Some(&self.find.query),
            HomeField::FindStars => Some(&self.find.stars),
            HomeField::FindAr => Some(&self.find.ar),
            HomeField::FindCs => Some(&self.find.cs),
            HomeField::FindOd => Some(&self.find.od),
            HomeField::FindHp => Some(&self.find.hp),
            HomeField::FindBpm => Some(&self.find.bpm),
            HomeField::FindLength => Some(&self.find.length),
            HomeField::FindKeys => Some(&self.find.keys),
            HomeField::FindFavourites => Some(&self.find.favourites),
            HomeField::FindRanked => Some(&self.find.ranked),
            HomeField::FindArtist => Some(&self.find.artist),
            HomeField::FindCreator => Some(&self.find.creator),
            HomeField::FindTitle => Some(&self.find.title),
            HomeField::FindLimit => Some(&self.find.limit),
            _ => None,
        }
    }

    fn focused_input_mut(&mut self) -> Option<&mut InputField> {
        match self.focus {
            HomeField::Collection => Some(&mut self.collection),
            HomeField::Directory => Some(&mut self.directory),
            HomeField::FindQuery => Some(&mut self.find.query),
            HomeField::FindStars => Some(&mut self.find.stars),
            HomeField::FindAr => Some(&mut self.find.ar),
            HomeField::FindCs => Some(&mut self.find.cs),
            HomeField::FindOd => Some(&mut self.find.od),
            HomeField::FindHp => Some(&mut self.find.hp),
            HomeField::FindBpm => Some(&mut self.find.bpm),
            HomeField::FindLength => Some(&mut self.find.length),
            HomeField::FindKeys => Some(&mut self.find.keys),
            HomeField::FindFavourites => Some(&mut self.find.favourites),
            HomeField::FindRanked => Some(&mut self.find.ranked),
            HomeField::FindArtist => Some(&mut self.find.artist),
            HomeField::FindCreator => Some(&mut self.find.creator),
            HomeField::FindTitle => Some(&mut self.find.title),
            HomeField::FindLimit => Some(&mut self.find.limit),
            _ => None,
        }
    }

    pub fn toggle_current(&mut self) {
        match self.focus {
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

    /// Min/max numeric (`Ms`) latency across the *enabled* built-in mirrors, for
    /// the Get Maps summary range. `None` when no enabled mirror has a numeric
    /// ping yet — in-flight / timeout / error probes don't contribute. A lone
    /// value yields `(n, n)`, which the renderer collapses to a single readout.
    pub fn mirror_latency_range(&self) -> Option<(u32, u32)> {
        self.mirror_order
            .iter()
            .filter(|&&kind| self.mirror_enabled(kind))
            .filter_map(|&kind| match self.mirror_latency.get(&kind).copied() {
                Some(Some(ProbeResult::Ms(ms))) => Some(ms),
                _ => None,
            })
            .fold(None, |range, ms| {
                Some(match range {
                    Some((min, max)) => (min.min(ms), max.max(ms)),
                    None => (ms, ms),
                })
            })
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
            MirrorKind::Nzbasic => self.nzbasic,
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
            return Err("enter a collection url or id".to_string());
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
            return Err("thread count must be between 1 and 100".to_string());
        }

        let mirrors = self.build_mirror_list();
        if mirrors.is_empty() {
            return Err("select at least one mirror".to_string());
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
            // Empty (retry everything); `App::request_download` resolves the
            // retry-failed-on-download policy and fills it in (or surfaces a
            // modal under `Ask` before the download is dispatched).
            previously_failed_skipped: HashSet::new(),
            // Placeholders; `App::request_download` fills these from the live
            // config + `App.library` client/path + the session collection cache
            // before the request is dispatched.
            skip_already_imported: false,
            osu_client: OsuClient::default(),
            osu_path: String::new(),
            prefetched: None,
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
    u8::from_str(value.trim()).map_err(|_| "thread count must be between 1 and 100".to_string())
}

#[cfg(test)]
#[path = "../../tests/unit/home.rs"]
mod tests;
