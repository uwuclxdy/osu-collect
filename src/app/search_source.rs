//! State for the Get Maps `Search` source: the query form (free-text +
//! mode/status/sort chips) plus the results browse. The osu! API v2 contract and
//! token plan live in `docs/plan/get-maps-rework.md`; this module holds the UI
//! state the source drives. Search is CTA-triggered — nothing here fires a query
//! on keystroke; the app dispatches [`AppCommand::RunSearch`] on the `search`
//! button and on `load more`.
//!
//! [`AppCommand::RunSearch`]: crate::app::AppCommand::RunSearch

use super::home::InputField;
use super::update_source::scroll_list;
use crate::app::update_source::LIST_PAGE;
use osu_downloader::search::{
    BeatmapSetMeta, SearchMode, SearchQuery, SearchStatus, SortField, SortOrder,
};
use std::cell::Cell;
use std::collections::HashSet;

/// One row in a [`SetBrowse`]: a beatmapset id plus optional metadata for the
/// preview. Search rows carry full [`BeatmapSetMeta`]; collection browse&pick
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
/// Shared by the search-results browse ([`SearchSource::browse`]) and the
/// collection browse&pick surface (`HomeTab::collection_browse`); each consumer
/// owns its own instance so keep-both persistence holds across source switches.
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

    /// Replace the rows (a fresh search / a fresh collection pick), homing the
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

/// Status of the current search, shown inline below the query field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SearchStatusMsg {
    /// No search run yet.
    #[default]
    Idle,
    /// A query is in flight.
    Loading,
    /// Results are in; `total` is the server's match count across all pages.
    Ready { total: u64 },
    /// The query returned no results.
    Empty,
    /// The query failed; the string is user-facing.
    Error(String),
}

/// A curated sort preset for the sort chip: a display label plus the osu! sort
/// field + order it maps to. osu! exposes 11 fields x 2 orders; the chip cycles
/// this hand-picked subset instead of all 22 combinations.
struct SortPreset {
    label: &'static str,
    field: SortField,
    order: SortOrder,
}

const SORT_PRESETS: &[SortPreset] = &[
    SortPreset {
        label: "relevance",
        field: SortField::Relevance,
        order: SortOrder::Desc,
    },
    SortPreset {
        label: "ranked ↓",
        field: SortField::Ranked,
        order: SortOrder::Desc,
    },
    SortPreset {
        label: "plays ↓",
        field: SortField::Plays,
        order: SortOrder::Desc,
    },
    SortPreset {
        label: "favourites ↓",
        field: SortField::Favourites,
        order: SortOrder::Desc,
    },
    SortPreset {
        label: "updated ↓",
        field: SortField::Updated,
        order: SortOrder::Desc,
    },
    SortPreset {
        label: "difficulty ↓",
        field: SortField::Difficulty,
        order: SortOrder::Desc,
    },
    SortPreset {
        label: "title ↑",
        field: SortField::Title,
        order: SortOrder::Asc,
    },
    SortPreset {
        label: "artist ↑",
        field: SortField::Artist,
        order: SortOrder::Asc,
    },
];

/// Mode chip labels; `mode_idx` indexes both this and [`MODE_VALUES`].
const MODE_LABELS: &[&str] = &["any", "osu", "taiko", "catch", "mania"];
const MODE_VALUES: &[Option<SearchMode>] = &[
    None,
    Some(SearchMode::Osu),
    Some(SearchMode::Taiko),
    Some(SearchMode::Fruits),
    Some(SearchMode::Mania),
];

// Label/value arrays are indexed by a shared `mode_idx`; a length drift would
// panic `mode_label`. Catch it at compile time instead.
const _: () = assert!(MODE_LABELS.len() == MODE_VALUES.len());

/// Status chip labels; `status_idx` indexes both this and [`STATUS_VALUES`].
/// `default` = the server default (has-leaderboard); `any` = every category.
const STATUS_LABELS: &[&str] = &[
    "default",
    "any",
    "leaderboard",
    "ranked",
    "qualified",
    "loved",
    "pending",
    "wip",
    "graveyard",
];
const STATUS_VALUES: &[Option<SearchStatus>] = &[
    None,
    Some(SearchStatus::Any),
    Some(SearchStatus::Leaderboard),
    Some(SearchStatus::Ranked),
    Some(SearchStatus::Qualified),
    Some(SearchStatus::Loved),
    Some(SearchStatus::Pending),
    Some(SearchStatus::Wip),
    Some(SearchStatus::Graveyard),
];

const _: () = assert!(STATUS_LABELS.len() == STATUS_VALUES.len());

/// The Get Maps `Search` source: the query form plus its results browse. Kept on
/// `HomeTab` so its state survives source-strip switches (keep-both).
pub struct SearchSource {
    pub query: InputField,
    mode_idx: usize,
    status_idx: usize,
    sort_idx: usize,
    pub status_msg: SearchStatusMsg,
    /// Cursor for the next page (`load more`); `None` = first page not yet run,
    /// or the last page reached.
    pub next_cursor: Option<String>,
    /// One-shot login-nudge gate: the guest-search nudge toast fires at most once
    /// per logged-out session.
    pub login_nudged: bool,
    /// Snapshot of the inputs `(query, mode, status, sort)` that produced the
    /// loaded `browse` rows, so the `view N maps` button can tell fresh results
    /// from stale ones left over after an input edit. `None` until results land.
    results_inputs: Option<(String, usize, usize, usize)>,
    pub browse: SetBrowse,
}

impl SearchSource {
    pub fn new() -> Self {
        Self {
            query: InputField::new("search", "", "artist, title, mapper, tags…"),
            mode_idx: 0,
            status_idx: 0,
            sort_idx: 0,
            status_msg: SearchStatusMsg::Idle,
            next_cursor: None,
            login_nudged: false,
            results_inputs: None,
            browse: SetBrowse::new(),
        }
    }

    /// The current search inputs as a comparable key `(query, mode, status, sort)`.
    fn current_inputs(&self) -> (String, usize, usize, usize) {
        (
            self.query.value.clone(),
            self.mode_idx,
            self.status_idx,
            self.sort_idx,
        )
    }

    /// Whether the loaded results still match the current inputs (so the
    /// `view N maps` button offers the right results, not stale ones).
    pub fn results_current(&self) -> bool {
        self.results_inputs.as_ref() == Some(&self.current_inputs())
    }

    /// Record the current inputs as the ones the loaded results are for (called
    /// when fresh results land).
    pub fn mark_results_current(&mut self) {
        self.results_inputs = Some(self.current_inputs());
    }

    /// Drop the results snapshot (the loaded rows no longer apply).
    pub fn clear_results_snapshot(&mut self) {
        self.results_inputs = None;
    }

    // ── chips ─────────────────────────────────────────────────────────────────

    pub fn cycle_mode(&mut self, forward: bool) {
        self.mode_idx = cycle_idx(self.mode_idx, MODE_VALUES.len(), forward);
    }

    pub fn cycle_status(&mut self, forward: bool) {
        self.status_idx = cycle_idx(self.status_idx, STATUS_VALUES.len(), forward);
    }

    pub fn cycle_sort(&mut self, forward: bool) {
        self.sort_idx = cycle_idx(self.sort_idx, SORT_PRESETS.len(), forward);
    }

    pub fn mode_label(&self) -> &'static str {
        MODE_LABELS[self.mode_idx]
    }

    /// The game-mode chip index, for carrying the selection across a find-backend
    /// switch (both backends share the `["any", "osu", …]` order).
    pub fn mode_idx(&self) -> usize {
        self.mode_idx
    }

    /// Set the game-mode chip index, clamped to the option count.
    pub fn set_mode_idx(&mut self, idx: usize) {
        self.mode_idx = idx.min(MODE_VALUES.len() - 1);
    }

    pub fn status_label(&self) -> &'static str {
        STATUS_LABELS[self.status_idx]
    }

    pub fn sort_label(&self) -> &'static str {
        SORT_PRESETS[self.sort_idx].label
    }

    /// Every mode option label, in cycle order — for a full cloudy cycle row that
    /// shows all options with the active one bracketed.
    pub fn mode_labels(&self) -> &'static [&'static str] {
        MODE_LABELS
    }

    /// Every status option label, in cycle order (see [`mode_labels`](Self::mode_labels)).
    pub fn status_labels(&self) -> &'static [&'static str] {
        STATUS_LABELS
    }

    /// Every sort-preset label, in cycle order (see [`mode_labels`](Self::mode_labels)).
    /// Built from [`SORT_PRESETS`] so the two never drift.
    pub fn sort_labels(&self) -> Vec<&'static str> {
        SORT_PRESETS.iter().map(|preset| preset.label).collect()
    }

    // ── query ─────────────────────────────────────────────────────────────────

    /// The osu! API query for the current form plus an optional paging cursor.
    pub fn build_query(&self, cursor: Option<String>) -> SearchQuery {
        let preset = &SORT_PRESETS[self.sort_idx];
        SearchQuery {
            text: self.query.value.trim().to_string(),
            mode: MODE_VALUES[self.mode_idx],
            status: STATUS_VALUES[self.status_idx],
            sort: Some((preset.field, preset.order)),
            cursor,
            // Typed q-DSL criteria land in phase 2; the current form emits none.
            ..SearchQuery::default()
        }
    }

    /// The per-run download label: the trimmed query, or a filter descriptor when
    /// the query is empty, so the output subdir is always recognizable
    /// (`search-<label>`). Never empty.
    pub fn run_label(&self) -> String {
        let query = self.query.value.trim();
        if !query.is_empty() {
            return query.to_string();
        }
        // Empty query: fall back to the active status, else the mode, else a
        // generic tag so the folder is never a bare `search-`.
        if self.status_idx != 0 {
            self.status_label().to_string()
        } else if self.mode_idx != 0 {
            self.mode_label().to_string()
        } else {
            "results".to_string()
        }
    }
}

impl Default for SearchSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Step an index forward/backward within `len`, wrapping at both ends. Shared
/// with the filter source's chips.
pub(crate) fn cycle_idx(idx: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if forward {
        (idx + 1) % len
    } else {
        (idx + len - 1) % len
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app_search_source.rs"]
mod tests;
