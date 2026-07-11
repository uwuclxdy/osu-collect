//! State for the Get Maps `Find` source: ONE union criteria form that compiles
//! to either an osu! API v2 search or an nzbasic BBD attribute filter. Which
//! backend runs is an implementation detail resolved by [`FindSource::build_plan`]
//! — every criterion carries per-backend expressibility, some criteria force a
//! backend, and a pair of conflicting forcers is a hard build error. The routing
//! contract lives in `docs/plan/find-backend-merge.md`.
//!
//! Transitional: two rendered forms still exist behind the `Backend` chip
//! (`src/tui/search_source.rs` / `src/tui/filter_source.rs`), each a SUBSET view
//! of this one union state — the shared mode/status/sort chips edit the same
//! value from either form. Phase 4 collapses them into a single form with a
//! read-only resolved-backend indicator.
//!
//! Like the old pair, a fetch is CTA-triggered — nothing here fires on keystroke.
//! The osu `next_cursor` pager and the nzbasic `beatmapDetails` pager both live
//! on this struct; phase 3 replaces the details pager with a shared osu-batch
//! enrichment service.

use super::home::{FindBackend, InputField};
use super::update_source::{LIST_PAGE, scroll_list};
use osu_downloader::filter::{
    FilterDirection, FilterMode, FilterQuery, FilterRange, FilterSort, FilterSpecial, FilterStatus,
};
use osu_downloader::search::{
    BeatmapSetMeta, QueryRange, SearchMode, SearchQuery, SearchStatus, SortField, SortOrder,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

/// Diff ids per `beatmapDetails` request. The first page fetches automatically
/// when nzbasic results land; `m` in the browse loads the next (a free solo-dev
/// instance — never sweep every page unprompted).
pub const DETAILS_PAGE: usize = 250;

/// One row in a [`SetBrowse`]: a beatmapset id plus optional metadata for the
/// preview. Find rows carry full [`BeatmapSetMeta`] (osu results directly,
/// nzbasic results once `beatmapDetails` pages fold in); collection browse&pick
/// rows are id-only (osu!collector exposes no per-set metadata).
#[derive(Debug, Clone)]
pub struct BrowseRow {
    pub id: u32,
    pub meta: Option<BeatmapSetMeta>,
}

/// A reusable flat checkbox-list browse over beatmapsets: a selectable list on
/// the left and a read-only detail preview of the highlighted row on the right.
/// A pure selector — the download button lives on the source form, not here.
///
/// Shared by the find-results browse ([`FindSource::browse`]) and the collection
/// browse&pick surface (`HomeTab::collection_browse`); each consumer owns its own
/// instance so keep-both persistence holds across source switches.
#[derive(Debug, Clone, Default)]
pub struct SetBrowse {
    pub rows: Vec<BrowseRow>,
    /// Picked set ids (always a subset of `rows`' ids).
    selected: HashSet<u32>,
    /// `false` = the source form; `true` = the two-pane browse.
    descended: bool,
    /// `false` = list pane focused; `true` = preview pane focused.
    preview_focused: bool,
    /// Cursor in the list (a row index into `rows`).
    list_cursor: Option<usize>,
    pub list_offset: Cell<usize>,
    pub preview_offset: Cell<usize>,
}

impl SetBrowse {
    pub fn new() -> Self {
        Self::default()
    }

    // ── row population ────────────────────────────────────────────────────────

    /// Replace the rows (a fresh find / a fresh collection pick), homing the
    /// cursor and dropping selections for ids no longer present.
    pub fn set_rows(&mut self, rows: Vec<BrowseRow>) {
        let present: HashSet<u32> = rows.iter().map(|r| r.id).collect();
        self.selected.retain(|id| present.contains(id));
        self.rows = rows;
        self.list_cursor = Some(0);
    }

    /// Append more rows (search `load more`), dropping ids already present so a
    /// page overlap (osu-web#9270) doesn't duplicate a set. Selections are kept.
    pub fn append_rows(&mut self, more: Vec<BrowseRow>) {
        let existing: HashSet<u32> = self.rows.iter().map(|r| r.id).collect();
        self.rows
            .extend(more.into_iter().filter(|r| !existing.contains(&r.id)));
    }

    /// Select or clear every current row (`a` / `A`).
    pub fn set_all_selected(&mut self, value: bool) {
        if value {
            self.selected = self.rows.iter().map(|r| r.id).collect();
        } else {
            self.selected.clear();
        }
    }

    // ── descend / ascend / focus ──────────────────────────────────────────────

    /// Descend from the form into the two-pane browse, focusing the list.
    pub fn descend(&mut self) {
        self.descended = true;
        self.preview_focused = false;
        if self.list_cursor.is_none() {
            self.list_cursor = Some(0);
        }
    }

    /// One step back (drives `esc`): preview → list, then browse → form. Returns
    /// whether a step was consumed.
    pub fn ascend(&mut self) -> bool {
        if self.preview_focused {
            self.preview_focused = false;
            true
        } else if self.descended {
            self.descended = false;
            true
        } else {
            false
        }
    }

    pub fn is_browsing(&self) -> bool {
        self.descended
    }

    pub fn preview_focused(&self) -> bool {
        self.preview_focused
    }

    /// Focus the preview pane — only when a row is highlighted (not the action
    /// bar), else a no-op.
    pub fn focus_preview(&mut self) {
        if self.highlighted_row().is_some() {
            self.preview_focused = true;
        }
    }

    pub fn focus_list(&mut self) {
        self.preview_focused = false;
    }

    // ── selection / cursor ────────────────────────────────────────────────────

    /// Flip the checkbox on the row under the list cursor. No-op on the action bar.
    pub fn toggle_selected(&mut self) {
        if let Some(row) = self.list_cursor.and_then(|i| self.rows.get(i)) {
            let id = row.id;
            if !self.selected.insert(id) {
                self.selected.remove(&id);
            }
        }
    }

    /// The row under the list cursor.
    pub fn highlighted_row(&self) -> Option<&BrowseRow> {
        self.list_cursor.and_then(|i| self.rows.get(i))
    }

    pub fn list_cursor(&self) -> Option<usize> {
        self.list_cursor
    }

    pub fn is_selected(&self, id: u32) -> bool {
        self.selected.contains(&id)
    }

    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    /// Picked set ids in row order.
    pub fn selected_ids(&self) -> Vec<u32> {
        self.rows
            .iter()
            .filter(|r| self.selected.contains(&r.id))
            .map(|r| r.id)
            .collect()
    }

    // ── scroll ────────────────────────────────────────────────────────────────

    pub fn scroll_up(&mut self) {
        self.scroll_by(-1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_by(1);
    }

    pub fn page_up(&mut self) {
        self.scroll_by(-LIST_PAGE);
    }

    pub fn page_down(&mut self) {
        self.scroll_by(LIST_PAGE);
    }

    /// Jump the list cursor to the first (`top`) or last row (`gg` / `G`). The
    /// preview is a static detail of the highlighted row, so it has no cursor to
    /// jump.
    pub fn scroll_to_edge(&mut self, top: bool) {
        if self.preview_focused {
            return;
        }
        let len = self.rows.len();
        if len > 0 {
            self.list_cursor = Some(if top { 0 } else { len - 1 });
        }
    }

    fn scroll_by(&mut self, delta: i64) {
        if self.preview_focused {
            return;
        }
        scroll_list(&mut self.list_cursor, self.rows.len(), delta);
    }
}

/// Status of the current find run, shown inline on the active form. The two
/// backends report a different `Ready` shape (osu = server match total; nzbasic
/// = deduped set count + summed `SizeMap` bytes), so each render matches its own.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FindStatusMsg {
    /// No run yet.
    #[default]
    Idle,
    /// A query is in flight.
    Loading,
    /// osu results are in; `total` is the server's match count across all pages.
    ReadySearch { total: u64 },
    /// nzbasic results are in: the deduped set count and the summed `SizeMap` bytes.
    ReadyFilter { sets: usize, total_bytes: u64 },
    /// The query returned no results.
    Empty,
    /// The query failed; the string is user-facing.
    Error(String),
}

/// The compiled query for the resolved backend, produced by
/// [`FindSource::build_plan`].
#[derive(Debug, Clone)]
pub enum FindPlan {
    /// Route to the osu! API v2 search.
    Osu(SearchQuery),
    /// Route to the nzbasic BBD filter.
    Nzbasic(FilterQuery),
}

/// A curated sort preset for the sort chip: a display label plus how it maps onto
/// each backend. `osu: None` = nzbasic-only (forces nzbasic); `nzbasic: None` =
/// osu-only (forces osu). Both `Some` = expressible either route (no forcer).
struct SortOption {
    label: &'static str,
    osu: Option<(SortField, SortOrder)>,
    nzbasic: Option<(FilterSort, FilterDirection)>,
}

/// Union sort list, deduping the two backends' 1:1 overlaps. Index 0 is the
/// default — it MUST be an expressible-either entry, else an untouched form plus
/// any nzbasic-forcer would be an instant conflict (a preset can't pick a
/// backend on its own).
const SORT: &[SortOption] = &[
    SortOption {
        label: "ranked ↓",
        osu: Some((SortField::Ranked, SortOrder::Desc)),
        nzbasic: Some((FilterSort::ApprovedDate, FilterDirection::Desc)),
    },
    SortOption {
        label: "stars ↓",
        osu: Some((SortField::Difficulty, SortOrder::Desc)),
        nzbasic: Some((FilterSort::Stars, FilterDirection::Desc)),
    },
    SortOption {
        label: "stars ↑",
        osu: Some((SortField::Difficulty, SortOrder::Asc)),
        nzbasic: Some((FilterSort::Stars, FilterDirection::Asc)),
    },
    SortOption {
        label: "plays ↓",
        osu: Some((SortField::Plays, SortOrder::Desc)),
        nzbasic: Some((FilterSort::PlayCount, FilterDirection::Desc)),
    },
    SortOption {
        label: "favourites ↓",
        osu: Some((SortField::Favourites, SortOrder::Desc)),
        nzbasic: Some((FilterSort::FavouriteCount, FilterDirection::Desc)),
    },
    SortOption {
        label: "updated ↓",
        osu: Some((SortField::Updated, SortOrder::Desc)),
        nzbasic: Some((FilterSort::LastUpdate, FilterDirection::Desc)),
    },
    // osu-only (force osu): no nzbasic column expresses these.
    SortOption {
        label: "relevance",
        osu: Some((SortField::Relevance, SortOrder::Desc)),
        nzbasic: None,
    },
    SortOption {
        label: "title ↑",
        osu: Some((SortField::Title, SortOrder::Asc)),
        nzbasic: None,
    },
    SortOption {
        label: "artist ↑",
        osu: Some((SortField::Artist, SortOrder::Asc)),
        nzbasic: None,
    },
    // nzbasic-only (force nzbasic): osu's `sort` can't express these.
    SortOption {
        label: "bpm ↓",
        osu: None,
        nzbasic: Some((FilterSort::Bpm, FilterDirection::Desc)),
    },
    SortOption {
        label: "length ↑",
        osu: None,
        nzbasic: Some((FilterSort::TotalLength, FilterDirection::Asc)),
    },
];

/// Mode chip labels; `mode_idx` indexes this plus both per-backend value arrays
/// (the [`SearchMode::Fruits`] / [`FilterMode::Catch`] naming collision the plan
/// flags — same slot, different enum). Expressible either route.
const MODE_LABELS: &[&str] = &["any", "osu", "taiko", "catch", "mania"];
const MODE_OSU: &[Option<SearchMode>] = &[
    None,
    Some(SearchMode::Osu),
    Some(SearchMode::Taiko),
    Some(SearchMode::Fruits),
    Some(SearchMode::Mania),
];
const MODE_NZBASIC: &[Option<FilterMode>] = &[
    None,
    Some(FilterMode::Osu),
    Some(FilterMode::Taiko),
    Some(FilterMode::Catch),
    Some(FilterMode::Mania),
];

const _: () = assert!(MODE_LABELS.len() == MODE_OSU.len() && MODE_OSU.len() == MODE_NZBASIC.len());

/// Status chip labels; `status_idx` indexes this plus the two per-backend maps.
/// `default` position is `leaderboard` (index [`STATUS_DEFAULT_IDX`]) — it
/// matches both backends' historical default.
const STATUS_LABELS: &[&str] = &[
    "any",
    "leaderboard",
    "ranked",
    "approved",
    "qualified",
    "loved",
    "pending",
    "wip",
    "graveyard",
    "unranked",
];
/// osu mapping: `None` = osu-inexpressible (only `unranked`), which forces nzbasic.
const STATUS_OSU: &[Option<SearchStatus>] = &[
    Some(SearchStatus::Any),
    Some(SearchStatus::Leaderboard),
    Some(SearchStatus::Ranked),
    Some(SearchStatus::Approved),
    Some(SearchStatus::Qualified),
    Some(SearchStatus::Loved),
    Some(SearchStatus::Pending),
    Some(SearchStatus::Wip),
    Some(SearchStatus::Graveyard),
    None,
];
/// nzbasic mapping: outer `None` = nzbasic-inexpressible (only `qualified`),
/// which forces osu; outer `Some(inner)` carries the `FilterStatus` (`Some(None)`
/// = "any" / no status constraint).
const STATUS_NZBASIC: &[Option<Option<FilterStatus>>] = &[
    Some(None),
    Some(Some(FilterStatus::Leaderboard)),
    Some(Some(FilterStatus::Ranked)),
    Some(Some(FilterStatus::Approved)),
    None,
    Some(Some(FilterStatus::Loved)),
    Some(Some(FilterStatus::Pending)),
    Some(Some(FilterStatus::Wip)),
    Some(Some(FilterStatus::Graveyard)),
    Some(Some(FilterStatus::Unranked)),
];

const _: () =
    assert!(STATUS_LABELS.len() == STATUS_OSU.len() && STATUS_OSU.len() == STATUS_NZBASIC.len());

/// The default status chip position (`leaderboard`), matching both backends'
/// historical default so an untouched form never sweeps graveyard noise.
const STATUS_DEFAULT_IDX: usize = 1;

/// Special-tag chip labels; `special_idx` indexes this and [`SPECIAL_VALUES`].
/// These flags exist only in nzbasic's database — a non-`none` value forces nzbasic.
const SPECIAL_LABELS: &[&str] = &["none", "farm", "stream", "ranked mapper"];
const SPECIAL_VALUES: &[Option<FilterSpecial>] = &[
    None,
    Some(FilterSpecial::Farm),
    Some(FilterSpecial::Stream),
    Some(FilterSpecial::RankedMapper),
];

const _: () = assert!(SPECIAL_LABELS.len() == SPECIAL_VALUES.len());

/// Preset chip labels. A preset is a seed-macro: selecting one RESETS the union
/// criteria to defaults and seeds the fields below — every value stays visible
/// and editable, so there is no hidden query state. `none` is the plain reset.
const PRESET_LABELS: &[&str] = &["none", "all ranked", "loved", "farm", "stream", "7★+"];

/// The Get Maps `Find` source: one union criteria form plus its results browse.
/// Kept on `HomeTab` so its state survives source-strip / backend-chip switches
/// (keep-both).
pub struct FindSource {
    preset_idx: usize,
    special_idx: usize,
    mode_idx: usize,
    status_idx: usize,
    sort_idx: usize,
    /// osu! free-text query (`q`). osu-only — a non-empty value forces osu.
    pub query: InputField,
    pub stars: InputField,
    pub ar: InputField,
    pub cs: InputField,
    pub od: InputField,
    pub hp: InputField,
    pub bpm: InputField,
    pub length: InputField,
    pub artist: InputField,
    pub creator: InputField,
    pub title: InputField,
    /// nzbasic diff-row limit (default 500). nzbasic-route mechanic; ignored on
    /// the osu route (osu paging caps results instead), so it never conflicts.
    pub limit: InputField,
    /// osu-only criteria with NO UI row yet (phase 4 adds them). Read by
    /// [`build_plan`](Self::build_plan) for routing + `q` serialization, so not
    /// dead code — they force osu when set and ride into the emitted query.
    pub keys: InputField,
    pub favourites: InputField,
    pub ranked: InputField,
    pub status_msg: FindStatusMsg,
    /// Cursor for the next osu page (`load more`); `None` = first page not yet
    /// run, the last page reached, or the last run was nzbasic (no cursor).
    pub next_cursor: Option<String>,
    /// One-shot login-nudge gate: the guest-search nudge toast fires at most once
    /// per logged-out session.
    pub login_nudged: bool,
    /// Every matching nzbasic diff id, in server order — the `beatmapDetails`
    /// pager walks this via [`details_cursor`](Self::details_cursor).
    pub diff_ids: Vec<u32>,
    /// Offset into [`diff_ids`](Self::diff_ids) of the next unfetched details page.
    details_cursor: usize,
    /// Bytes per set id, from the nzbasic fetch response's `SizeMap`.
    pub size_map: HashMap<u32, u64>,
    /// Snapshot of the canonical inputs that produced the loaded rows, so the
    /// `view N maps` button can tell fresh results from stale ones.
    results_inputs: Option<String>,
    /// The backend that produced the currently-loaded results, for the download
    /// subdir prefix (`search-*` / `filter-*`). Set at fetch time; survives a
    /// later form edit (the rows stay from that backend until a re-fetch).
    results_backend: Option<FindBackend>,
    pub browse: SetBrowse,
}

impl FindSource {
    pub fn new() -> Self {
        Self {
            preset_idx: 0,
            special_idx: 0,
            mode_idx: 0,
            status_idx: STATUS_DEFAULT_IDX,
            sort_idx: 0,
            query: InputField::new("search", "", "artist, title, mapper, tags…"),
            stars: InputField::new("stars", "", "min-max, e.g. 5.5-7"),
            ar: InputField::new("ar", "", "min-max"),
            cs: InputField::new("cs", "", "min-max"),
            od: InputField::new("od", "", "min-max"),
            hp: InputField::new("hp", "", "min-max"),
            bpm: InputField::new("bpm", "", "min-max, e.g. 180-"),
            length: InputField::new("length", "", "seconds, e.g. 90-300"),
            artist: InputField::new("artist", "", "contains…"),
            creator: InputField::new("mapper", "", "contains…"),
            title: InputField::new("title", "", "contains…"),
            limit: InputField::new("limit", "", "500"),
            keys: InputField::new("keys", "", "min-max"),
            favourites: InputField::new("favourites", "", "min-max"),
            ranked: InputField::new("ranked", "", "yyyy or yyyy-mm-dd"),
            status_msg: FindStatusMsg::Idle,
            next_cursor: None,
            login_nudged: false,
            diff_ids: Vec::new(),
            details_cursor: 0,
            size_map: HashMap::new(),
            results_inputs: None,
            results_backend: None,
            browse: SetBrowse::new(),
        }
    }

    // ── presets ───────────────────────────────────────────────────────────────

    /// Cycle the preset chip and apply its seed: reset the criteria (and the
    /// sort) to defaults, then set the preset's fields. Limit is left untouched
    /// (it shapes the result size, not the criteria).
    pub fn cycle_preset(&mut self, forward: bool) {
        self.preset_idx = cycle_idx(self.preset_idx, PRESET_LABELS.len(), forward);
        self.apply_preset(self.preset_idx);
    }

    fn apply_preset(&mut self, idx: usize) {
        // Reset every criterion plus the sort, so a preset always yields a
        // clean, conflict-free state: a stray osu-forcer left set (free text, an
        // osu-only sort like relevance) would turn a nzbasic-forcing preset into
        // a routing conflict. Limit stays (result size, not criteria).
        self.special_idx = 0;
        self.mode_idx = 0;
        self.status_idx = STATUS_DEFAULT_IDX;
        self.sort_idx = 0;
        for field in [
            &mut self.query,
            &mut self.stars,
            &mut self.ar,
            &mut self.cs,
            &mut self.od,
            &mut self.hp,
            &mut self.bpm,
            &mut self.length,
            &mut self.artist,
            &mut self.creator,
            &mut self.title,
            &mut self.keys,
            &mut self.favourites,
            &mut self.ranked,
        ] {
            field.set_value("");
        }

        match PRESET_LABELS[idx] {
            "all ranked" => self.status_idx = status_idx_of("ranked"),
            "loved" => self.status_idx = status_idx_of("loved"),
            // BBD parity: its farm/stream presets pin mode to osu!standard.
            "farm" => {
                self.mode_idx = mode_idx_of("osu");
                self.special_idx = special_idx_of("farm");
            }
            "stream" => {
                self.mode_idx = mode_idx_of("osu");
                self.special_idx = special_idx_of("stream");
            }
            "7★+" => self.stars.set_value("7-"),
            _ => {} // "none" = the plain reset above
        }
    }

    // ── chips ─────────────────────────────────────────────────────────────────

    pub fn cycle_special(&mut self, forward: bool) {
        self.special_idx = cycle_idx(self.special_idx, SPECIAL_VALUES.len(), forward);
    }

    pub fn cycle_mode(&mut self, forward: bool) {
        self.mode_idx = cycle_idx(self.mode_idx, MODE_LABELS.len(), forward);
    }

    pub fn cycle_status(&mut self, forward: bool) {
        self.status_idx = cycle_idx(self.status_idx, STATUS_LABELS.len(), forward);
    }

    pub fn cycle_sort(&mut self, forward: bool) {
        self.sort_idx = cycle_idx(self.sort_idx, SORT.len(), forward);
    }

    pub fn preset_label(&self) -> &'static str {
        PRESET_LABELS[self.preset_idx]
    }

    pub fn special_label(&self) -> &'static str {
        SPECIAL_LABELS[self.special_idx]
    }

    pub fn mode_label(&self) -> &'static str {
        MODE_LABELS[self.mode_idx]
    }

    /// The game-mode chip index. Shared across the backend chip: the mode carries
    /// automatically now (one union value), so the backend switch does nothing
    /// special. Kept as an accessor for tests / phase-4 render.
    pub fn mode_idx(&self) -> usize {
        self.mode_idx
    }

    /// Set the game-mode chip index, clamped to the option count.
    pub fn set_mode_idx(&mut self, idx: usize) {
        self.mode_idx = idx.min(MODE_LABELS.len() - 1);
    }

    pub fn status_label(&self) -> &'static str {
        STATUS_LABELS[self.status_idx]
    }

    pub fn sort_label(&self) -> &'static str {
        SORT[self.sort_idx].label
    }

    pub fn preset_labels(&self) -> &'static [&'static str] {
        PRESET_LABELS
    }

    pub fn special_labels(&self) -> &'static [&'static str] {
        SPECIAL_LABELS
    }

    pub fn mode_labels(&self) -> &'static [&'static str] {
        MODE_LABELS
    }

    pub fn status_labels(&self) -> &'static [&'static str] {
        STATUS_LABELS
    }

    /// Every sort-preset label, in cycle order. Built from [`SORT`] so the two
    /// never drift.
    pub fn sort_labels(&self) -> Vec<&'static str> {
        SORT.iter().map(|opt| opt.label).collect()
    }

    // ── plan / routing ──────────────────────────────────────────────────────

    /// Resolve the form to a concrete backend query. Per-criterion backend
    /// requirements decide the route: a criterion the osu route can't express
    /// forces nzbasic and vice versa; a pair of conflicting forcers is an `Err`
    /// naming both fields (user-facing toast); otherwise osu is the default.
    /// `cursor` threads the osu paging cursor (ignored on the nzbasic route).
    pub fn build_plan(&self, cursor: Option<String>) -> Result<FindPlan, String> {
        let nz_reason = self.nzbasic_forcer();
        let osu_reason = self.osu_forcer();
        match (nz_reason, osu_reason) {
            (Some(nz), Some(osu)) => Err(format!("{nz} needs nzbasic, {osu} needs osu! api")),
            (Some(_), None) => Ok(FindPlan::Nzbasic(self.build_filter_query()?)),
            // osu-forcer only, or nothing → osu is the default route.
            (None, _) => Ok(FindPlan::Osu(self.build_search_query(cursor)?)),
        }
    }

    /// The backend [`build_plan`](Self::build_plan) routes to, ignoring parse
    /// errors and treating a conflict as osu. Used only as the download-source
    /// fallback when no fetch recorded a `results_backend`.
    pub fn planned_backend(&self) -> FindBackend {
        match (self.nzbasic_forcer(), self.osu_forcer()) {
            (Some(_), None) => FindBackend::Nzbasic,
            _ => FindBackend::Osu,
        }
    }

    /// The first criterion forcing the nzbasic route, or `None`. Priority:
    /// special → sort → status. The string is user-facing (conflict toast).
    fn nzbasic_forcer(&self) -> Option<String> {
        if self.special_idx != 0 {
            Some(SPECIAL_LABELS[self.special_idx].to_string())
        } else if SORT[self.sort_idx].osu.is_none() {
            Some(format!("sort {}", SORT[self.sort_idx].label))
        } else if STATUS_OSU[self.status_idx].is_none() {
            Some(format!("status {}", STATUS_LABELS[self.status_idx]))
        } else {
            None
        }
    }

    /// The first criterion forcing the osu route, or `None`. Priority: free text
    /// → sort → status → keys → ranked date → favourites.
    fn osu_forcer(&self) -> Option<String> {
        if !self.query.value.trim().is_empty() {
            Some("free text".to_string())
        } else if SORT[self.sort_idx].nzbasic.is_none() {
            Some(format!("sort {}", SORT[self.sort_idx].label))
        } else if STATUS_NZBASIC[self.status_idx].is_none() {
            Some(format!("status {}", STATUS_LABELS[self.status_idx]))
        } else if !self.keys.value.trim().is_empty() {
            Some("keys".to_string())
        } else if !self.ranked.value.trim().is_empty() {
            Some("ranked date".to_string())
        } else if !self.favourites.value.trim().is_empty() {
            Some("favourites".to_string())
        } else {
            None
        }
    }

    /// Build the osu! `SearchQuery` from the union fields (osu route).
    fn build_search_query(&self, cursor: Option<String>) -> Result<SearchQuery, String> {
        Ok(SearchQuery {
            text: self.query.value.trim().to_string(),
            mode: MODE_OSU[self.mode_idx],
            status: STATUS_OSU[self.status_idx],
            sort: SORT[self.sort_idx].osu,
            cursor,
            stars: osu_float_range(&self.stars)?,
            ar: osu_float_range(&self.ar)?,
            cs: osu_float_range(&self.cs)?,
            od: osu_float_range(&self.od)?,
            hp: osu_float_range(&self.hp)?,
            bpm: osu_float_range(&self.bpm)?,
            length: osu_int_range(&self.length)?,
            keys: osu_int_range(&self.keys)?,
            favourites: osu_int_range(&self.favourites)?,
            ranked: osu_date_range(&self.ranked),
            artist: text_opt(&self.artist),
            creator: text_opt(&self.creator),
            title: text_opt(&self.title),
        })
    }

    /// Build the nzbasic `FilterQuery` from the union fields (nzbasic route).
    fn build_filter_query(&self) -> Result<FilterQuery, String> {
        Ok(FilterQuery {
            mode: MODE_NZBASIC[self.mode_idx],
            status: STATUS_NZBASIC[self.status_idx].flatten(),
            special: SPECIAL_VALUES[self.special_idx],
            stars: parse_range(self.stars.label, &self.stars.value)?,
            ar: parse_range(self.ar.label, &self.ar.value)?,
            cs: parse_range(self.cs.label, &self.cs.value)?,
            od: parse_range(self.od.label, &self.od.value)?,
            hp: parse_range(self.hp.label, &self.hp.value)?,
            bpm: parse_range(self.bpm.label, &self.bpm.value)?,
            length: parse_range(self.length.label, &self.length.value)?,
            artist: self.artist.value.trim().to_string(),
            creator: self.creator.value.trim().to_string(),
            title: self.title.value.trim().to_string(),
            sort: SORT[self.sort_idx].nzbasic,
            limit: Some(parse_limit(&self.limit.value)?),
        })
    }

    // ── labels / tags ─────────────────────────────────────────────────────────

    /// Canonical string of the union CRITERIA only (no sort/limit): chip labels,
    /// trimmed texts, and numeric ranges re-rendered from their parsed bounds —
    /// so two runs filtering the same maps share a folder tag even when ordered,
    /// limited, or spelled differently (`6-7` vs `6.0-7.0`).
    fn criteria_string(&self) -> String {
        format!(
            "special={}|mode={}|status={}|q={}|stars={}|ar={}|cs={}|od={}|hp={}|bpm={}|len={}|keys={}|fav={}|ranked={}|artist={}|creator={}|title={}",
            self.special_label(),
            self.mode_label(),
            self.status_label(),
            self.query.value.trim(),
            canonical_range(&self.stars),
            canonical_range(&self.ar),
            canonical_range(&self.cs),
            canonical_range(&self.od),
            canonical_range(&self.hp),
            canonical_range(&self.bpm),
            canonical_range(&self.length),
            canonical_range(&self.keys),
            canonical_range(&self.favourites),
            self.ranked.value.trim(),
            self.artist.value.trim(),
            self.creator.value.trim(),
            self.title.value.trim(),
        )
    }

    /// Canonical string of ALL inputs (criteria + sort + limit) — the staleness
    /// key for the `view N maps` button.
    fn inputs_string(&self) -> String {
        format!(
            "{}|sort={}|limit={}",
            self.criteria_string(),
            self.sort_label(),
            self.limit.value.trim(),
        )
    }

    /// The per-run subdir tag (the source picks the `search-`/`filter-` prefix).
    /// Precedence: the preset label while its seed is untouched → the free text
    /// (so an osu free-text run lands in the same `search-<query>` dir as before)
    /// → the first text criterion → an 8-hex FNV-1a of the canonical criteria.
    /// Deterministic, so re-running lands in the same dir and different criteria
    /// never collide on the per-dir lock.
    pub fn folder_tag(&self) -> String {
        if self.preset_idx != 0 {
            let mut seeded = Self::new();
            seeded.apply_preset(self.preset_idx);
            if seeded.criteria_string() == self.criteria_string() {
                return PRESET_LABELS[self.preset_idx].to_string();
            }
        }
        let query = self.query.value.trim();
        if !query.is_empty() {
            return query.to_string();
        }
        for field in [&self.title, &self.artist, &self.creator] {
            let value = field.value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
        format!("{:08x}", fnv1a_32(&self.criteria_string()))
    }

    /// The per-run display label. Precedence: preset → free text → first text
    /// criterion → special/stars/status descriptor. Never empty.
    pub fn run_label(&self) -> String {
        if self.preset_idx != 0 && self.folder_tag() == PRESET_LABELS[self.preset_idx] {
            return PRESET_LABELS[self.preset_idx].to_string();
        }
        let query = self.query.value.trim();
        if !query.is_empty() {
            return query.to_string();
        }
        for field in [&self.title, &self.artist, &self.creator] {
            let value = field.value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
        if self.special_idx != 0 {
            return SPECIAL_LABELS[self.special_idx].to_string();
        }
        let stars = self.stars.value.trim();
        if !stars.is_empty() {
            return format!("stars {stars}");
        }
        if self.status_idx != STATUS_DEFAULT_IDX {
            return STATUS_LABELS[self.status_idx].to_string();
        }
        "results".to_string()
    }

    // ── results / staleness ───────────────────────────────────────────────────

    /// Whether the loaded results still match the current inputs (so the
    /// `view N maps` button offers the right results, not stale ones).
    pub fn results_current(&self) -> bool {
        self.results_inputs.as_ref() == Some(&self.inputs_string())
    }

    /// Record the current inputs as the ones the loaded results are for (called
    /// when fresh results land).
    pub fn mark_results_current(&mut self) {
        self.results_inputs = Some(self.inputs_string());
    }

    /// Drop the results snapshot (the loaded rows no longer apply). Leaves
    /// `results_backend` — the rows, while they exist, still came from it.
    pub fn clear_results_snapshot(&mut self) {
        self.results_inputs = None;
    }

    /// Record which backend produced the just-landed results (drives the
    /// download subdir prefix).
    pub fn note_results_backend(&mut self, backend: FindBackend) {
        self.results_backend = Some(backend);
    }

    /// The backend that produced the currently-loaded results, or `None` when no
    /// fetch has recorded one.
    pub fn results_backend(&self) -> Option<FindBackend> {
        self.results_backend
    }

    // ── nzbasic details pager ───────────────────────────────────────────────

    /// Adopt an nzbasic fetch response: remember the diff ids + sizes and rewind
    /// the details pager. Row population happens caller-side (the runtime handler
    /// owns the `BrowseRow` mapping).
    pub fn set_results(&mut self, diff_ids: Vec<u32>, size_map: HashMap<u32, u64>) {
        self.diff_ids = diff_ids;
        self.size_map = size_map;
        self.details_cursor = 0;
    }

    /// The next unfetched `beatmapDetails` page, advancing the pager. `None`
    /// once every diff id has been requested.
    pub fn next_details_page(&mut self) -> Option<Vec<u32>> {
        if self.details_cursor >= self.diff_ids.len() {
            return None;
        }
        let end = (self.details_cursor + DETAILS_PAGE).min(self.diff_ids.len());
        let page = self.diff_ids[self.details_cursor..end].to_vec();
        self.details_cursor = end;
        Some(page)
    }

    /// Rewind the pager to `cursor` (a failed page retries on the next `m`).
    pub fn rewind_details(&mut self, cursor: usize) {
        self.details_cursor = cursor.min(self.diff_ids.len());
    }

    /// The pager offset (captured before a fetch so a failure can rewind).
    pub fn details_cursor(&self) -> usize {
        self.details_cursor
    }

    /// Whether `m` still has details pages to load.
    pub fn has_more_details(&self) -> bool {
        self.details_cursor < self.diff_ids.len()
    }
}

impl Default for FindSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Step an index forward/backward within `len`, wrapping at both ends.
fn cycle_idx(idx: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if forward {
        (idx + 1) % len
    } else {
        (idx + len - 1) % len
    }
}

/// Union chip index for a status/mode/special label. The arrays are const-length
/// and every seeded label appears in them, so a miss falls back to a safe slot.
fn status_idx_of(label: &str) -> usize {
    STATUS_LABELS
        .iter()
        .position(|&l| l == label)
        .unwrap_or(STATUS_DEFAULT_IDX)
}

fn mode_idx_of(label: &str) -> usize {
    MODE_LABELS.iter().position(|&l| l == label).unwrap_or(0)
}

fn special_idx_of(label: &str) -> usize {
    SPECIAL_LABELS.iter().position(|&l| l == label).unwrap_or(0)
}

/// An exact-text osu criterion, or `None` when the field is blank.
fn text_opt(field: &InputField) -> Option<String> {
    let value = field.value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Convert a min-max input into an osu float criterion. Bounds come straight
/// from `str::parse` (never derived arithmetic) so the emitted `q` stays
/// byte-stable; `from_bounds` collapses "no bound" to a single representation.
fn osu_float_range(field: &InputField) -> Result<Option<QueryRange<f64>>, String> {
    let range = parse_range(field.label, &field.value)?;
    Ok(match (range.min, range.max) {
        // A bare value parses to equal bounds; emit `key=value` so the server
        // applies its tolerance band instead of a degenerate `>=`/`<=` pair.
        (Some(min), Some(max)) if min == max => Some(QueryRange::Exact(min)),
        (min, max) => QueryRange::from_bounds(min, max),
    })
}

/// Canonical form of a numeric min-max input for the criteria string: parsed
/// bounds re-rendered so equivalent spellings (`6-7` / `6.0-7.0`) share a folder
/// tag and never read as diverged. An unparseable (mid-edit) value falls back to
/// the raw string, so it correctly reads as diverged until it parses again.
fn canonical_range(field: &InputField) -> String {
    match parse_range(field.label, &field.value) {
        Ok(FilterRange {
            min: None,
            max: None,
        }) => String::new(),
        Ok(range) => {
            let bound = |b: Option<f64>| b.map(|v| v.to_string()).unwrap_or_default();
            format!("{}~{}", bound(range.min), bound(range.max))
        }
        Err(_) => field.value.trim().to_string(),
    }
}

/// Convert a min-max input into an osu integer criterion (`length` / `keys` /
/// `favourites`). A bare value emits `key=value` (`Exact`); a pair emits the
/// `>=`/`<=` range. Integer-only — a fractional bound is rejected.
fn osu_int_range(field: &InputField) -> Result<Option<QueryRange<u32>>, String> {
    let value = field.value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let label = field.label;
    let parse = |part: &str| -> Result<u32, String> {
        part.trim()
            .parse::<u32>()
            .map_err(|_| format!("{label}: \"{part}\" is not a whole number"))
    };
    match value.split_once('-') {
        Some((min, max)) => {
            let min = (!min.trim().is_empty()).then(|| parse(min)).transpose()?;
            let max = (!max.trim().is_empty()).then(|| parse(max)).transpose()?;
            if let (Some(a), Some(b)) = (min, max)
                && a > b
            {
                return Err(format!("{label}: min {a} is greater than max {b}"));
            }
            Ok(QueryRange::from_bounds(min, max))
        }
        None => Ok(Some(QueryRange::Exact(parse(value)?))),
    }
}

/// Convert a date input into an osu `ranked` criterion. No UI row yet (phase 4),
/// so this stays minimal: a non-empty value is an exact `ranked=<value>` term
/// (the `-` in `yyyy-mm-dd` rules out reusing the numeric range separator —
/// phase 4 wires the row and a proper date-range syntax).
fn osu_date_range(field: &InputField) -> Option<QueryRange<String>> {
    let value = field.value.trim();
    (!value.is_empty()).then(|| QueryRange::Exact(value.to_string()))
}

/// Parse a min-max pair for the nzbasic route: empty = unconstrained, `a-b` =
/// both bounds, `a-` = min only, `-b` = max only, a bare `a` = exactly that value.
fn parse_range(label: &str, value: &str) -> Result<FilterRange, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(FilterRange::default());
    }
    let parse = |part: &str| -> Result<f64, String> {
        part.trim()
            .parse::<f64>()
            .ok()
            // `f64::parse` accepts "nan"/"inf"; neither is a usable bound and
            // NaN would slip past the min>max guard, so reject at the boundary.
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("{label}: \"{part}\" is not a number"))
    };
    let range = match value.split_once('-') {
        Some((min, max)) => FilterRange {
            min: (!min.trim().is_empty()).then(|| parse(min)).transpose()?,
            max: (!max.trim().is_empty()).then(|| parse(max)).transpose()?,
        },
        None => {
            let exact = parse(value)?;
            FilterRange {
                min: Some(exact),
                max: Some(exact),
            }
        }
    };
    if let (Some(min), Some(max)) = (range.min, range.max)
        && min > max
    {
        return Err(format!("{label}: min {min} is greater than max {max}"));
    }
    Ok(range)
}

/// Default 500; the cap protects the free nzbasic instance from megaqueries.
fn parse_limit(value: &str) -> Result<u32, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(500);
    }
    let limit: u32 = value
        .parse()
        .map_err(|_| format!("limit: \"{value}\" is not a number"))?;
    if !(1..=10_000).contains(&limit) {
        return Err("limit: must be between 1 and 10000".to_string());
    }
    Ok(limit)
}

/// FNV-1a 32-bit — a stable, dependency-free hash for the folder tag.
fn fnv1a_32(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

#[cfg(test)]
#[path = "../../tests/unit/app_find_source.rs"]
mod tests;
