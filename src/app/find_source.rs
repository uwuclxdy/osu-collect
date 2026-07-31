//! State for the Get Maps `Find` source: ONE union criteria form that compiles
//! to either an osu! API v2 search or an nzbasic BBD attribute filter. Which
//! backend runs is an implementation detail resolved by [`FindSource::build_plan`]
//! — every criterion carries per-backend expressibility, some criteria force a
//! backend, and a pair of conflicting forcers is a hard build error. The routing
//! contract lives in `docs/plan/find-backend-merge.md`.
//!
//! One rendered form (`src/tui/find_source.rs`) edits this union state; a
//! read-only indicator ([`FindSource::resolved_route`]) shows which backend the
//! criteria resolve to before the CTA fires.
//!
//! A fetch is CTA-triggered — nothing here fires on keystroke.
//! The osu `next_cursor` pager lives on this struct; id-only rows (nzbasic
//! results) are backfilled by the shared osu-batch enrichment pager that every
//! [`SetBrowse`] carries.

use super::home::{FindBackend, InputField};
use super::update_source::{LIST_PAGE, PAGE_ROWS, scroll_list, scroll_list_clamped};
use osu_downloader::filter::{
    BeatmapDetails, FilterDirection, FilterMode, FilterQuery, FilterRange, FilterSort,
    FilterSpecial, FilterStatus,
};
use osu_downloader::search::{
    Beatmap, BeatmapSetMeta, QueryRange, RangeBound, SearchMode, SearchQuery, SearchStatus,
    SortField, SortOrder,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

/// Diff ids per enrichment page. The first page auto-fetches when id-only results
/// land / the browse descends; `m` loads the next. The osu-batch service chunks
/// each page into <=50-id calls internally (a page of 250 = 5 calls).
pub const ENRICH_PAGE: usize = 250;

/// Which id-only browse an enrichment page targets, so one shared osu-batch pager
/// service can fold set-level metadata into the right [`SetBrowse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrichTarget {
    /// The find source's results browse ([`FindSource::browse`]) — nzbasic-routed
    /// runs land id-only here (osu-routed runs arrive with metadata already).
    Find,
    /// The collection browse&pick surface (`HomeTab::collection_browse`).
    Collection,
    /// The update source's missing-set preview (`UpdateSource`) — the scan lands
    /// set ids only; the batch backfills each set's title/artist.
    Update,
}

/// Lazy osu-batch enrichment cursor for one browse: walks a list of diff ids in
/// [`ENRICH_PAGE`]-sized pages, feeding metadata backfill. `generation` bumps on
/// every reseed / clear so a page from a superseded run (a new find run, a fresh
/// collection open) is dropped by the handler instead of folding into the new
/// rows. Advancing on dispatch + rewinding a failed page keeps the "`m` retries a
/// failed page" contract the old `beatmapDetails` pager had. `in_flight` counts
/// outstanding scheduled pages so the browse can render an explicit loading cue
/// (`is_enriching()` = a page is still pending). It is a counter, not a flag:
/// a landed page's event is processed AFTER the next `m` may have scheduled
/// another, so a plain bool would let the older event clear the cue while a newer
/// fetch is still pending (the cue vanishing mid-fetch). Each dispatched page
/// bumps it; each same-generation land/fail event decrements it (a stale-gen
/// event drops before decrementing); a reseed zeros it.
///
/// Shared by every id-only browse — the flat [`SetBrowse`] and the update
/// source's missing-set preview — through the [`EnrichSink`] trait.
#[derive(Debug, Clone, Default)]
pub(crate) struct EnrichPager {
    diff_ids: Vec<u32>,
    cursor: usize,
    generation: u64,
    in_flight: u32,
}

impl EnrichPager {
    /// Adopt a fresh diff-id list (new results / a new collection open), homing
    /// the cursor and invalidating any in-flight page.
    pub(crate) fn seed(&mut self, diff_ids: Vec<u32>) {
        self.diff_ids = diff_ids;
        self.cursor = 0;
        self.in_flight = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Drop the pager (the rows it enriched are gone), invalidating any in-flight
    /// page.
    pub(crate) fn clear(&mut self) {
        self.diff_ids = Vec::new();
        self.cursor = 0;
        self.in_flight = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    /// The next unfetched page, advancing the cursor past it. `None` once every
    /// diff id has been requested. Holes (ids the server omits) don't stall the
    /// cursor — it advances by the requested count regardless.
    pub(crate) fn next_page(&mut self) -> Option<Vec<u32>> {
        if self.cursor >= self.diff_ids.len() {
            return None;
        }
        let end = (self.cursor + ENRICH_PAGE).min(self.diff_ids.len());
        let page = self.diff_ids[self.cursor..end].to_vec();
        self.cursor = end;
        Some(page)
    }

    /// Rewind to `cursor` (a failed page retries on the next `m`).
    pub(crate) fn rewind(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.diff_ids.len());
    }

    pub(crate) fn has_more(&self) -> bool {
        self.cursor < self.diff_ids.len()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    /// A page was dispatched — count it as outstanding (drives the loading cue).
    pub(crate) fn mark_dispatched(&mut self) {
        self.in_flight = self.in_flight.saturating_add(1);
    }

    /// A page's result (success or failure) landed — one fewer outstanding.
    pub(crate) fn mark_settled(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
    }

    pub(crate) fn is_enriching(&self) -> bool {
        self.in_flight > 0
    }
}

/// An id-only browse that the shared osu-batch enrichment pager backfills. Lets
/// the runtime drive [`SetBrowse`] and the update source's missing-set preview
/// through one code path (`src/app/runtime/enrich.rs`): the generic pager methods
/// plus a `fold_meta` that each consumer routes into its own row/cache shape and
/// an `enriching` flag that gates the loading cue.
pub trait EnrichSink {
    fn enrich_generation(&self) -> u64;
    fn enrich_cursor(&self) -> usize;
    fn next_enrich_page(&mut self) -> Option<Vec<u32>>;
    fn rewind_enrichment(&mut self, cursor: usize);
    fn has_more_enrichment(&self) -> bool;
    /// Count a page as dispatched (drives the browse's loading cue).
    fn mark_enrichment_dispatched(&mut self);
    /// Count a page as settled (its result just landed). Gen-guarded by the
    /// caller, so this only fires for a page this sink actually dispatched.
    fn mark_enrichment_settled(&mut self);
    fn is_enriching(&self) -> bool;
    /// Fold a landed batch page's set-level metadata into this browse.
    fn fold_meta(&mut self, meta_by_set: HashMap<u32, BeatmapSetMeta>);
}

/// One row in a [`SetBrowse`]: a beatmapset id plus optional metadata for the
/// preview. osu-routed find rows carry full [`BeatmapSetMeta`] straight from the
/// search response; id-only rows (nzbasic results + collection browse&pick) land
/// with `meta: None` and get it backfilled by the enrichment pager.
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
/// Sort order for the difficulty spread in the preview pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffSort {
    /// Stars ascending — easiest difficulties first (default).
    #[default]
    StarsAsc,
    /// Stars descending — hardest first.
    StarsDesc,
}

impl DiffSort {
    const ALL: &[Self] = &[Self::StarsAsc, Self::StarsDesc];

    pub fn label(&self) -> &'static str {
        match self {
            Self::StarsAsc => "stars ↑",
            Self::StarsDesc => "stars ↓",
        }
    }

    fn cycle(&mut self) {
        let idx = Self::ALL.iter().position(|s| s == self).unwrap_or(0);
        *self = Self::ALL[(idx + 1) % Self::ALL.len()];
    }
}

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
    /// Top row of the preview pane. The page keys raise it while the preview
    /// owns focus; the render clamps it against the row count and the pane's
    /// height and writes the resolved value back, so the bottom is settled in
    /// the one place that knows where it is.
    pub preview_offset: Cell<usize>,
    /// The bottom of the preview at the pane's last-rendered size, reported by
    /// the render. `G` reads it instead of parking a value past the end, which a
    /// following page key would subtract from and land right back on the bottom.
    pub preview_max_offset: Cell<usize>,
    /// Sort order for the difficulty spread in the preview pane. Defaults to
    /// [`DiffSort::StarsAsc`] so easier diffs list first.
    pub diff_sort: DiffSort,
    /// Cursor into the highlighted row's `beatmaps[]`: which difficulty the
    /// preview's detail block renders. `None` resolves to the hardest diff
    /// (first-seen on ties, matching [`Self::record_details`]'s strict-`>` fold).
    /// Reset to `None` whenever the list cursor moves to another row, since the
    /// cursor is only meaningful relative to one row's spread.
    diff_cursor: Option<usize>,
    /// osu-batch metadata backfill for this browse's id-only rows. Empty (inert)
    /// for a browse whose rows already carry `meta` (osu search results).
    enrich: EnrichPager,
    /// nzbasic-only per-set extra columns (tags/source/genre/language/dates plus
    /// one representative diff's combo/drain/passes/hash), keyed by set id. Filled
    /// by the nzbasic details path (`src/app/runtime/details.rs`), which pages the
    /// same diff ids as `enrich` under the same generation. Empty and inert for
    /// osu-routed find results and collection browse&pick; cleared with the rows.
    details: HashMap<u32, BeatmapDetails>,
}

impl SetBrowse {
    pub fn new() -> Self {
        Self::default()
    }

    // ── row population ────────────────────────────────────────────────────────

    /// Replace the rows (a fresh find / a fresh collection pick), homing the
    /// cursor and dropping selections for ids no longer present. Clears the
    /// enrichment pager — new rows are a new identity, so any in-flight page is
    /// stale (the caller reseeds it afterwards for an id-only result set). Any
    /// id-only row whose set is already in the session `cache` hydrates straight
    /// away, so a reopen never refetches a title the app already fetched.
    pub fn set_rows(&mut self, mut rows: Vec<BrowseRow>, cache: &HashMap<u32, BeatmapSetMeta>) {
        for row in &mut rows {
            if row.meta.is_none()
                && let Some(meta) = cache.get(&row.id)
            {
                row.meta = Some(meta.clone());
            }
        }
        let present: HashSet<u32> = rows.iter().map(|r| r.id).collect();
        self.selected.retain(|id| present.contains(id));
        self.rows = rows;
        self.list_cursor = Some(0);
        self.reset_preview();
        self.enrich.clear();
        // New identity → any prior nzbasic details are stale; the details path
        // reseeds itself off the fresh enrichment pages under the new generation.
        self.details.clear();
    }

    /// Fold a landed nzbasic details page: the HARDEST diff (highest `stars`)
    /// seen for a set wins — set-level columns are identical across a set's
    /// diffs, and the per-diff stats (AR/CS/OD/HP, combo, drain) are most
    /// informative on the top difficulty. A recorded set is replaced only when
    /// a strictly harder diff arrives: the first-seen diff at the top rating
    /// wins ties (strict `>`). The preview's representative-diff picker
    /// (`diff_section_rows`) matches this tie direction.
    pub(crate) fn record_details(&mut self, rows: Vec<BeatmapDetails>) {
        for row in rows {
            self.details
                .entry(row.set_id)
                .and_modify(|existing| {
                    if row.stars > existing.stars {
                        *existing = row.clone();
                    }
                })
                .or_insert(row);
        }
    }

    /// The nzbasic details recorded for a set, if the details path has fetched
    /// them. Always `None` for osu-routed results and collection browse&pick.
    pub fn details_for(&self, set_id: u32) -> Option<&BeatmapDetails> {
        self.details.get(&set_id)
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

    /// Cycle the difficulty spread sort order (`s` in the preview). Resets the
    /// diff cursor so the detail block follows the hardest diff to its new position.
    pub fn cycle_diff_sort(&mut self) {
        self.diff_sort.cycle();
        self.diff_cursor = None;
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

    /// Page the focused pane by [`PAGE_ROWS`] rows: the list cursor, clamped at
    /// the ends (paging never wraps — unlike a single
    /// [`scroll_up`](Self::scroll_up) step), or the preview's scroll while it
    /// holds focus. `↑`/`↓` keep cycling the highlighted difficulty there, so
    /// the page keys are what reach a preview taller than its pane.
    pub fn page_up(&mut self) {
        if self.preview_focused {
            self.preview_offset
                .set(self.preview_offset.get().saturating_sub(PAGE_ROWS));
            return;
        }
        self.reset_preview();
        scroll_list_clamped(&mut self.list_cursor, self.rows.len(), -LIST_PAGE);
    }

    pub fn page_down(&mut self) {
        if self.preview_focused {
            self.preview_offset
                .set(self.preview_offset.get().saturating_add(PAGE_ROWS));
            return;
        }
        self.reset_preview();
        scroll_list_clamped(&mut self.list_cursor, self.rows.len(), LIST_PAGE);
    }

    /// Jump to the first (`top`) or last row of the focused pane (`gg` / `G`):
    /// the list cursor, or the preview's scroll. It is the pane that jumps, never
    /// the difficulty cursor — `j`/`k` and the arrows move that. A list jump
    /// resets the diff cursor to hardest.
    pub fn scroll_to_edge(&mut self, top: bool) {
        if self.preview_focused {
            // The bottom comes from the last frame (`preview_max_offset`), not
            // from a value past the end: the clamp reads those as the bottom but
            // a page key does not, so `G` then `PageUp` inside one coalesced
            // event batch — no frame between them to resolve it — would subtract
            // a page from the sentinel and stay put.
            self.preview_offset.set(if top {
                0
            } else {
                self.preview_max_offset.get()
            });
            return;
        }
        self.reset_preview();
        let len = self.rows.len();
        if len > 0 {
            self.list_cursor = Some(if top { 0 } else { len - 1 });
        }
    }

    fn scroll_by(&mut self, delta: i64) {
        if self.preview_focused {
            // The preview owns ↑/↓: step the difficulty cursor within the
            // highlighted row's spread (no-op when it has none).
            self.move_diff_cursor(delta);
            return;
        }
        self.reset_preview();
        scroll_list(&mut self.list_cursor, self.rows.len(), delta);
    }

    /// Drop what is read RELATIVE to the highlighted row: the difficulty cursor
    /// (back to hardest) and the preview's scroll. Every list-cursor move lands
    /// on another row, and another row is another preview, read from its top.
    fn reset_preview(&mut self) {
        self.diff_cursor = None;
        self.preview_offset.set(0);
    }

    // ── difficulty cursor ─────────────────────────────────────────────────────

    /// Index into the highlighted row's `beatmaps[]` of the difficulty the
    /// preview's detail block renders: the cursor if set (clamped to the
    /// spread), else the hardest diff (first-seen on ties). `None` when the row
    /// has no metadata or no difficulty spread — the render then falls back to a
    /// recorded [`BeatmapDetails`] single diff, or nothing.
    pub fn focused_diff_index(&self) -> Option<usize> {
        let beatmaps = &self.highlighted_row()?.meta.as_ref()?.beatmaps;
        if beatmaps.is_empty() {
            return None;
        }
        Some(
            self.diff_cursor
                .map(|i| i.min(beatmaps.len() - 1))
                .unwrap_or_else(|| hardest_beatmap_index(beatmaps)),
        )
    }

    /// Step the difficulty cursor by `delta` in the order the preview LISTS the
    /// spread ([`Self::diff_sort`]), wrapping at both ends like the row list. A
    /// no-op when the highlighted row has no spread. Only called while the
    /// preview owns focus.
    fn move_diff_cursor(&mut self, delta: i64) {
        let sort = self.diff_sort;
        let next = self.highlighted_row().and_then(|row| {
            let beatmaps = &row.meta.as_ref()?.beatmaps;
            if beatmaps.is_empty() {
                return None;
            }
            let current = self
                .diff_cursor
                .map(|i| i.min(beatmaps.len() - 1))
                .unwrap_or_else(|| hardest_beatmap_index(beatmaps));
            let order = diff_order(beatmaps, sort);
            let pos = order.iter().position(|&i| i == current).unwrap_or(0);
            let stepped = (pos as i64 + delta).rem_euclid(order.len() as i64) as usize;
            Some(order[stepped])
        });
        if let Some(next) = next {
            self.diff_cursor = Some(next);
        }
    }

    // ── enrichment pager ──────────────────────────────────────────────────────

    /// Seed the enrichment pager from `(diff_id, set_id)` seeds (one diff per set
    /// is enough — the batch response nests each row's set metadata). A seed whose
    /// set is already in the session `cache` is pruned, so the pager only pages
    /// sets whose title the app hasn't fetched yet. Callers with no set pairing
    /// (find / nzbasic results) pass `None` set ids, which are never pruned. The
    /// runtime auto-fetches the first page after this.
    pub fn seed_enrichment(
        &mut self,
        seeds: Vec<(u32, Option<u32>)>,
        cache: &HashMap<u32, BeatmapSetMeta>,
    ) {
        self.enrich.seed(pruned_diff_ids(seeds, cache));
    }
}

/// The spread's original-array indices in the order the preview lists them.
/// Ordering by `sort` here rather than in the render is what lets the difficulty
/// cursor step by what is on screen: the API's `beatmaps[]` arrives in neither
/// star order, so an index step walks the spread in an order nobody can see.
/// `sort_by` is stable, so ties keep the array's own order — first-seen, the
/// same tie-break [`hardest_beatmap_index`] takes.
pub(crate) fn diff_order(beatmaps: &[Beatmap], sort: DiffSort) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..beatmaps.len()).collect();
    indices.sort_by(|&a, &b| {
        let (a, b) = match sort {
            DiffSort::StarsAsc => (a, b),
            DiffSort::StarsDesc => (b, a),
        };
        beatmaps[a]
            .difficulty_rating
            .partial_cmp(&beatmaps[b].difficulty_rating)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    indices
}

/// Index of the hardest diff (highest star rating) in `beatmaps`, first-seen
/// winning ties — matching [`SetBrowse::record_details`]'s strict-`>` fold, so
/// the preview's default focused diff agrees with the recorded representative.
/// `rev()` before `max_by` so the last maximal element is the earliest index.
pub(crate) fn hardest_beatmap_index(beatmaps: &[Beatmap]) -> usize {
    beatmaps
        .iter()
        .enumerate()
        .rev()
        .max_by(|(_, a), (_, b)| {
            a.difficulty_rating
                .partial_cmp(&b.difficulty_rating)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Diff ids the enrichment pager must actually fetch: drop any seed whose set is
/// already in `cache` (its metadata is known, so the batch would refetch it for
/// nothing). Seeds with no set pairing (`None`) are always kept. Shared by every
/// cache-aware seeder (the flat browse and the update source's missing preview).
pub(crate) fn pruned_diff_ids(
    seeds: Vec<(u32, Option<u32>)>,
    cache: &HashMap<u32, BeatmapSetMeta>,
) -> Vec<u32> {
    seeds
        .into_iter()
        .filter(|&(_, set_id)| !set_id.is_some_and(|s| cache.contains_key(&s)))
        .map(|(diff, _)| diff)
        .collect()
}

impl EnrichSink for SetBrowse {
    /// The pager generation, bumped on every reseed / clear. The runtime tags each
    /// scheduled page with it and drops a returned page whose generation no longer
    /// matches (a superseding run reseeded meanwhile).
    fn enrich_generation(&self) -> u64 {
        self.enrich.generation
    }

    /// The pager offset (captured before a fetch so a failure can rewind to it).
    fn enrich_cursor(&self) -> usize {
        self.enrich.cursor
    }

    /// Pull the next unfetched enrichment page, advancing the cursor.
    fn next_enrich_page(&mut self) -> Option<Vec<u32>> {
        self.enrich.next_page()
    }

    /// Rewind the pager to `cursor` (a failed page retries on the next `m`).
    fn rewind_enrichment(&mut self, cursor: usize) {
        self.enrich.rewind(cursor);
    }

    /// Whether `m` still has enrichment pages to load.
    fn has_more_enrichment(&self) -> bool {
        self.enrich.has_more()
    }

    fn mark_enrichment_dispatched(&mut self) {
        self.enrich.mark_dispatched();
    }

    fn mark_enrichment_settled(&mut self) {
        self.enrich.mark_settled();
    }

    fn is_enriching(&self) -> bool {
        self.enrich.is_enriching()
    }

    /// Fold set-level metadata into the id-only rows. `meta_by_set` is keyed by
    /// beatmapset id (the caller dedupes per set, first diff wins); only rows
    /// still missing `meta` are filled, so a re-fetch never clobbers a preview.
    fn fold_meta(&mut self, mut meta_by_set: HashMap<u32, BeatmapSetMeta>) {
        for row in &mut self.rows {
            if row.meta.is_none()
                && let Some(meta) = meta_by_set.remove(&row.id)
            {
                row.meta = Some(meta);
            }
        }
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

/// Which backend the current criteria resolve to, for the form's read-only
/// resolved-backend indicator ([`FindSource::resolved_route`]). Routing only —
/// mirrors [`FindSource::build_plan`]'s decision but ignores parse errors, so a
/// mid-edit bad range still shows the route it would take. A conflict carries
/// both offending field names for the inline warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindRoute {
    /// osu! API v2 search — the default when nothing forces nzbasic.
    Osu,
    /// nzbasic BBD filter — an nzbasic-forcing criterion is set.
    Nzbasic,
    /// Conflicting forcers: `nzbasic` names the field forcing nzbasic, `osu` the
    /// one forcing osu.
    Conflict { nzbasic: String, osu: String },
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

/// Session-lived download-size state for one result set. Filled by both routes —
/// the osu route probes nekoha lazily, the nzbasic route folds its response's
/// free `SizeMap` in — so a state here is never route-specific.
#[derive(Debug, Clone, Copy)]
enum SizeState {
    /// A probe is in flight — dedupes a rapid re-trigger (another toggle). A
    /// probe that fails to reach the mirror clears this rather than settling it,
    /// so the id is retried; only the mirror's own answer settles a set.
    Pending,
    /// The mirror's download size in bytes.
    Known(u64),
    /// The mirror answered that it has no size for this set; never re-probed.
    Missing,
}

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
    /// osu-only criteria (mania key count, favourite count, ranked date). Each
    /// forces osu when set and rides into the emitted `q`; `ranked` takes a
    /// `..`-separated date range (see [`osu_date_range`]).
    pub keys: InputField,
    pub favourites: InputField,
    pub ranked: InputField,
    pub status_msg: FindStatusMsg,
    /// Whether the `advanced filters` disclosure is expanded, showing the 13
    /// per-attribute range inputs. Collapsed by default so the primary form
    /// (query + chips + find CTA) fits on one screen.
    pub advanced_filters_open: bool,
    /// Cursor for the next osu page (`load more`); `None` = first page not yet
    /// run, the last page reached, or the last run was nzbasic (no cursor).
    pub next_cursor: Option<String>,
    /// One-shot login-nudge gate: the guest-search nudge toast fires at most once
    /// per logged-out session.
    pub login_nudged: bool,
    /// Per-set download-size cache, session-lived (a set id's size is stable, so
    /// it outlives a re-search). `Known` feeds the download button's `· ~X` size
    /// suffix and seeds a run so the pipeline re-fetches nothing already here.
    ///
    /// ONE cache, both routes: the osu route backfills it by lazy nekoha probe
    /// (`runtime/size.rs`), the nzbasic route folds in its response's exact
    /// `SizeMap` for free (`runtime/filter.rs`). Only the probe is osu-only.
    /// The collection browse never populates this.
    size_cache: HashMap<u32, SizeState>,
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
            query: InputField::new("query", "", "artist, title, mapper, tags…"),
            stars: InputField::new("stars", "", "e.g. 7+  5.5..7  <9"),
            ar: InputField::new("ar", "", "e.g. 9  8+"),
            cs: InputField::new("cs", "", "e.g. 4+"),
            od: InputField::new("od", "", "e.g. 8+"),
            hp: InputField::new("hp", "", "e.g. 6+"),
            bpm: InputField::new("bpm", "", "e.g. 180+  170..190"),
            length: InputField::new("length", "", "seconds, e.g. 90+  60..300"),
            artist: InputField::new("artist", "", "contains…"),
            creator: InputField::new("mapper", "", "contains…"),
            title: InputField::new("title", "", "contains…"),
            limit: InputField::new("limit", "", "500"),
            keys: InputField::new("keys", "", "e.g. 4  4..7"),
            favourites: InputField::new("favourites", "", "e.g. 10000+"),
            ranked: InputField::new("ranked", "", "yyyy or 2020..2024"),
            status_msg: FindStatusMsg::Idle,
            advanced_filters_open: false,
            next_cursor: None,
            login_nudged: false,
            size_cache: HashMap::new(),
            results_inputs: None,
            results_backend: None,
            browse: SetBrowse::new(),
        }
    }

    /// Toggle the `advanced filters` disclosure that gates the 13 per-attribute
    /// range inputs. Navigation skips the collapsed inputs so the focus path
    /// stays on the primary form.
    pub fn toggle_advanced_filters(&mut self) {
        self.advanced_filters_open = !self.advanced_filters_open;
    }

    /// Whether the advanced-filter section should render: the user has toggled it
    /// open, or at least one advanced input has a non-empty value. This
    /// auto-expands the section when inputs carry data so a live value is never
    /// silently hidden.
    pub fn show_advanced_filters(&self) -> bool {
        self.advanced_filters_open || self.has_any_advanced_input()
    }

    /// Whether any of the 13 per-attribute range/text inputs has a non-empty value.
    fn has_any_advanced_input(&self) -> bool {
        !self.stars.value.is_empty()
            || !self.ar.value.is_empty()
            || !self.cs.value.is_empty()
            || !self.od.value.is_empty()
            || !self.hp.value.is_empty()
            || !self.bpm.value.is_empty()
            || !self.length.value.is_empty()
            || !self.keys.value.is_empty()
            || !self.favourites.value.is_empty()
            || !self.ranked.value.is_empty()
            || !self.artist.value.is_empty()
            || !self.creator.value.is_empty()
            || !self.title.value.is_empty()
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
            "7★+" => self.stars.set_value("7+"),
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
            (Some(nz), Some(osu)) => Err(format!("{nz} needs nzbasic · {osu} needs osu! api")),
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

    /// The resolved-backend indicator's view of the current criteria: the route
    /// [`build_plan`](Self::build_plan) would take (osu / nzbasic), or the
    /// conflict it would reject. Ignores parse errors — routing is decided by the
    /// forcers alone — so the indicator stays live even mid-edit.
    pub fn resolved_route(&self) -> FindRoute {
        match (self.nzbasic_forcer(), self.osu_forcer()) {
            (Some(nzbasic), Some(osu)) => FindRoute::Conflict { nzbasic, osu },
            (Some(_), None) => FindRoute::Nzbasic,
            (None, _) => FindRoute::Osu,
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
    ///
    /// The osu-only range fields route off their PARSED criterion, not their raw
    /// text — see [`emits_criterion`].
    fn osu_forcer(&self) -> Option<String> {
        if !self.query.value.trim().is_empty() {
            Some("free text".to_string())
        } else if SORT[self.sort_idx].nzbasic.is_none() {
            Some(format!("sort {}", SORT[self.sort_idx].label))
        } else if STATUS_NZBASIC[self.status_idx].is_none() {
            Some(format!("status {}", STATUS_LABELS[self.status_idx]))
        } else if emits_criterion(osu_int_range(&self.keys)) {
            Some("keys".to_string())
        } else if emits_criterion(osu_date_range(&self.ranked)) {
            Some("ranked date".to_string())
        } else if emits_criterion(osu_int_range(&self.favourites)) {
            Some("favourites".to_string())
        } else {
            None
        }
    }

    // ── live validation ───────────────────────────────────────────────────────

    /// The `ranked` field's parse error, or `None` when it is blank or valid.
    /// Backs the on-focus hint: the date grammar is the field's own, so
    /// [`describe_range`] (the numeric-range reader) can't speak for it, and
    /// without this a bad date stays silent until the run is attempted.
    pub fn ranked_error(&self) -> Option<String> {
        osu_date_range(&self.ranked).err()
    }

    /// The `limit` field's parse error, or `None` when it is blank or valid.
    /// Same contract as [`ranked_error`](Self::ranked_error).
    pub fn limit_error(&self) -> Option<String> {
        parse_limit(&self.limit.value).err()
    }

    /// Build the osu! `SearchQuery` from the union fields (osu route).
    fn build_search_query(&self, cursor: Option<String>) -> Result<SearchQuery, String> {
        Ok(SearchQuery {
            text: self.query.value.trim().to_string(),
            mode: MODE_OSU[self.mode_idx],
            status: STATUS_OSU[self.status_idx],
            genre: None,
            language: None,
            extra: Default::default(),
            nsfw: None,
            rank: Default::default(),
            played: None,
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
            ranked: osu_date_range(&self.ranked)?,
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
            stars: filter_range(&self.stars)?,
            ar: filter_range(&self.ar)?,
            cs: filter_range(&self.cs)?,
            od: filter_range(&self.od)?,
            hp: filter_range(&self.hp)?,
            bpm: filter_range(&self.bpm)?,
            length: filter_range(&self.length)?,
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

    // ── size backfill ─────────────────────────────────────────────────────────

    /// Claim the checked results still needing a nekoha size probe, marking each
    /// `Pending` so a rapid re-trigger (another toggle) never double-fetches. Only
    /// checked sets are claimed, and an already-cached id (pending / known /
    /// missing) is skipped — a set is probed at most once per session unless its
    /// probe fails to reach the mirror ([`release_size_probe`]). The runtime
    /// spawns the concurrency-capped probe for the returned ids; this is only
    /// called on the osu route (the caller gates it), so nzbasic ids never land
    /// here.
    ///
    /// [`release_size_probe`]: Self::release_size_probe
    pub fn claim_size_probes(&mut self) -> Vec<u32> {
        use std::collections::hash_map::Entry;
        let mut claimed = Vec::new();
        for id in self.browse.selected_ids() {
            if let Entry::Vacant(slot) = self.size_cache.entry(id) {
                slot.insert(SizeState::Pending);
                claimed.push(id);
            }
        }
        claimed
    }

    /// Fold a landed size probe into the cache: `Some` records the mirror's byte
    /// count, `None` records the set as sizeless. Both are the mirror's own
    /// answer, so neither is re-probed. A probe that could not reach the mirror
    /// says nothing about the set — that goes to [`release_size_probe`] instead.
    ///
    /// [`release_size_probe`]: Self::release_size_probe
    pub fn record_size(&mut self, id: u32, size: Option<u64>) {
        self.size_cache
            .insert(id, size.map_or(SizeState::Missing, SizeState::Known));
    }

    /// Drop a `Pending` claim whose probe failed to reach the mirror, so the id
    /// is claimable again and the next selection change retries it. Only a
    /// `Pending` entry is released: a `Known` / `Missing` answer that landed
    /// meanwhile is the mirror's, and outranks a failure.
    pub fn release_size_probe(&mut self, id: u32) {
        if matches!(self.size_cache.get(&id), Some(SizeState::Pending)) {
            self.size_cache.remove(&id);
        }
    }

    /// Sum of the known nekoha sizes among the currently-checked results, for the
    /// download button's `· ~X` suffix. Pending / missing / un-probed sets add 0,
    /// so this is a partial (hence approximate) total that grows as probes land;
    /// zero while nothing is known, which drops the suffix entirely.
    pub fn checked_known_bytes(&self) -> u64 {
        self.browse
            .selected_ids()
            .iter()
            .filter_map(|id| match self.size_cache.get(id) {
                Some(SizeState::Known(bytes)) => Some(*bytes),
                _ => None,
            })
            .sum()
    }

    /// The already-known nekoha sizes among `ids`, seeding a run's size
    /// estimate so a fully-cached selection needs no probe at all. Both routes
    /// feed this cache: nzbasic folds its free `SizeMap` in at fetch time
    /// (`record_size`), osu backfills lazily via the probe in `runtime/size.rs`.
    pub fn known_sizes_for(&self, ids: &[u32]) -> HashMap<u32, u64> {
        ids.iter()
            .filter_map(|id| match self.size_cache.get(id) {
                Some(SizeState::Known(bytes)) => Some((*id, *bytes)),
                _ => None,
            })
            .collect()
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

/// Whether a range field carries a criterion its backend must honour — the test
/// [`FindSource::osu_forcer`] routes on, applied to that field's own builder so
/// the two can't drift.
///
/// `Ok(None)` is the only non-forcing case: the field is blank, or degenerate
/// (`-` / `..`, a range with neither bound), and emits no query term — nothing to
/// route for. `Err` forces too: the value is broken rather than absent, so the
/// user typed a criterion, and the route it belongs to is the only one that can
/// report the parse error. Routing past it would drop the value onto a backend
/// with no column for it, silently.
fn emits_criterion<T>(parsed: Result<Option<T>, String>) -> bool {
    !matches!(parsed, Ok(None))
}

/// Convert a range input into an osu float criterion (`stars`/`ar`/…). `Exact`
/// (a bare value) emits `key=value` so the server applies its tolerance band;
/// bounds carry their `>`/`>=`/`<`/`<=` operator through to the `q` string.
fn osu_float_range(field: &InputField) -> Result<Option<QueryRange<f64>>, String> {
    Ok(parse_range_criterion(field.label, &field.value)?.map(|c| c.to_query(|v| v)))
}

/// Convert a range input into an osu integer criterion (`length`/`keys`/
/// `favourites`). Integer-only — a fractional bound is rejected. Operators carry
/// through as in [`osu_float_range`].
fn osu_int_range(field: &InputField) -> Result<Option<QueryRange<u32>>, String> {
    let Some(criterion) = parse_range_criterion(field.label, &field.value)? else {
        return Ok(None);
    };
    let label = field.label;
    let to_u32 = |value: f64| -> Result<u32, String> {
        if value.fract() != 0.0 {
            return Err(format!(
                "{label}: \"{}\" is not a whole number",
                fmt_num(value)
            ));
        }
        Ok(value as u32)
    };
    Ok(Some(criterion.try_to_query(to_u32)?))
}

/// The nzbasic inclusive [`FilterRange`] for a field. nzbasic has no strict
/// bound, so `>`/`<` collapse to their inclusive form; a bare value pins both
/// bounds. A blank field is the empty range.
fn filter_range(field: &InputField) -> Result<FilterRange, String> {
    let Some(criterion) = parse_range_criterion(field.label, &field.value)? else {
        return Ok(FilterRange::default());
    };
    Ok(match criterion {
        RangeCriterion::Exact(value) => FilterRange {
            min: Some(value),
            max: Some(value),
        },
        RangeCriterion::Bounds { lower, upper } => FilterRange {
            min: lower.map(|b| b.value),
            max: upper.map(|b| b.value),
        },
    })
}

/// Canonical form of a range input for the criteria string: the parsed criterion
/// re-rendered so equivalent spellings (`7>`/`>7`, `6..7`/`6.0..7.0`) share a
/// folder tag and never read as diverged. An unparseable (mid-edit) value falls
/// back to the raw string, so it correctly reads as diverged until it parses.
fn canonical_range(field: &InputField) -> String {
    match parse_range_criterion(field.label, &field.value) {
        Ok(None) => String::new(),
        Ok(Some(criterion)) => canonical_criterion(&criterion),
        Err(_) => field.value.trim().to_string(),
    }
}

fn canonical_criterion(criterion: &RangeCriterion) -> String {
    match criterion {
        RangeCriterion::Exact(value) => format!("={}", fmt_num(*value)),
        RangeCriterion::Bounds { lower, upper } => {
            let mut out = String::new();
            if let Some(bound) = lower {
                let op = if bound.inclusive { ">=" } else { ">" };
                out.push_str(&format!("{op}{}", fmt_num(bound.value)));
            }
            if let Some(bound) = upper {
                if !out.is_empty() {
                    out.push(' ');
                }
                let op = if bound.inclusive { "<=" } else { "<" };
                out.push_str(&format!("{op}{}", fmt_num(bound.value)));
            }
            out
        }
    }
}

/// Convert a date input into an osu `ranked` criterion. A single token is an
/// exact `ranked=<token>`; a `..`-separated pair emits a `>=`/`<=` range
/// (`a..b` / `a..` / `..b`). The range separator is `..`, not `-`, because a
/// date token itself uses `-` (`yyyy-mm-dd`). Each token must be `yyyy`,
/// `yyyy-mm`, or `yyyy-mm-dd`; the server does the real calendar validation. A
/// pair is rejected when inverted at their *comparable* precision (year, then
/// month, then day — only as deep as both tokens specify), matching the
/// min>max guard every sibling range parser has; differing precision
/// (`2020..2020-06`) has no comparable month, so it is never inverted.
fn osu_date_range(field: &InputField) -> Result<Option<QueryRange<String>>, String> {
    let value = field.value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let label = field.label;
    let token = |part: &str| -> Result<String, String> {
        let part = part.trim();
        if is_date_token(part) {
            Ok(part.to_string())
        } else {
            Err(format!(
                "{label}: \"{part}\" is not a yyyy / yyyy-mm / yyyy-mm-dd date (range separator is `..`)"
            ))
        }
    };
    match value.split_once("..") {
        Some((min_raw, max_raw)) => {
            let min = (!min_raw.trim().is_empty())
                .then(|| token(min_raw))
                .transpose()?;
            let max = (!max_raw.trim().is_empty())
                .then(|| token(max_raw))
                .transpose()?;
            if let (Some(min_s), Some(max_s)) = (&min, &max) {
                // Both tokens already passed `token`, so parsing here is infallible.
                let min_tuple = parse_date_token(min_s).expect("token already validated");
                let max_tuple = parse_date_token(max_s).expect("token already validated");
                if date_token_inverted(min_tuple, max_tuple) {
                    return Err(format!("{label}: min {min_s} is greater than max {max_s}"));
                }
            }
            Ok(QueryRange::from_bounds(min, max))
        }
        None => Ok(Some(QueryRange::Exact(token(value)?))),
    }
}

/// Parses a `yyyy` / `yyyy-mm` / `yyyy-mm-dd` date token into `(year, month,
/// day)`, `month`/`day` present only at the token's own precision. `None` when
/// `s` isn't 1–3 `-`-separated numeric segments of width 4/2/2 (loose — no
/// calendar range check; the server rejects an impossible month/day).
fn parse_date_token(s: &str) -> Option<(u32, Option<u32>, Option<u32>)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut fields: [Option<u32>; 3] = [None; 3];
    for ((part, width), field) in parts.iter().zip([4usize, 2, 2]).zip(fields.iter_mut()) {
        if part.len() != width || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *field = part.parse::<u32>().ok();
    }
    Some((fields[0]?, fields[1], fields[2]))
}

fn is_date_token(s: &str) -> bool {
    parse_date_token(s).is_some()
}

/// Whether `min`'s date tuple is strictly greater than `max`'s at their shared
/// (comparable) precision: year first; month only when both sides carry one;
/// day only when both carry one. A side missing a component makes that depth
/// incomparable, so `2020` (year-only) vs `2020-06` reads as equal, not inverted.
fn date_token_inverted(
    min: (u32, Option<u32>, Option<u32>),
    max: (u32, Option<u32>, Option<u32>),
) -> bool {
    if min.0 != max.0 {
        return min.0 > max.0;
    }
    let (min_month, max_month) = match (min.1, max.1) {
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };
    if min_month != max_month {
        return min_month > max_month;
    }
    match (min.2, max.2) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// A parsed numeric range criterion from one field. `Exact` is the bare-value
/// form (`=N`), kept distinct from a bound because osu widens `=` to a tolerance
/// band server-side. `Bounds` carries one or two sides, each with its own strict
/// / inclusive flag. Blank fields never reach here — callers short-circuit first.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RangeCriterion {
    /// Bare `N` → `=N`.
    Exact(f64),
    /// One- or two-sided bounds; at least one side is set.
    Bounds {
        /// Lower bound (`>N` / `>=N`), if set.
        lower: Option<NumBound>,
        /// Upper bound (`<N` / `<=N`), if set.
        upper: Option<NumBound>,
    },
}

/// One side of a [`RangeCriterion::Bounds`].
#[derive(Debug, Clone, Copy, PartialEq)]
struct NumBound {
    value: f64,
    /// `true` → inclusive (`>=`/`<=`); `false` → strict (`>`/`<`).
    inclusive: bool,
}

impl RangeCriterion {
    /// Map onto an [`QueryRange`] with an infallible value conversion (float).
    fn to_query<T>(self, conv: impl Fn(f64) -> T) -> QueryRange<T> {
        match self {
            RangeCriterion::Exact(value) => QueryRange::Exact(conv(value)),
            RangeCriterion::Bounds { lower, upper } => QueryRange::Range {
                min: lower.map(|b| RangeBound {
                    value: conv(b.value),
                    inclusive: b.inclusive,
                }),
                max: upper.map(|b| RangeBound {
                    value: conv(b.value),
                    inclusive: b.inclusive,
                }),
            },
        }
    }

    /// Map onto an [`QueryRange`] with a fallible value conversion (integer keys
    /// reject a fractional bound).
    fn try_to_query<T, E>(self, conv: impl Fn(f64) -> Result<T, E>) -> Result<QueryRange<T>, E> {
        let bound = |b: NumBound| -> Result<RangeBound<T>, E> {
            Ok(RangeBound {
                value: conv(b.value)?,
                inclusive: b.inclusive,
            })
        };
        Ok(match self {
            RangeCriterion::Exact(value) => QueryRange::Exact(conv(value)?),
            RangeCriterion::Bounds { lower, upper } => QueryRange::Range {
                min: lower.map(bound).transpose()?,
                max: upper.map(bound).transpose()?,
            },
        })
    }
}

/// Comparison operator parsed off a single-value range token.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RangeOp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
}

/// Render a parsed numeric bound back to a stable, minimal string (`7`, `6.5`).
fn fmt_num(value: f64) -> String {
    value.to_string()
}

/// The find route's numeric range grammar (osu-native comparison concept),
/// accepting the operator as a prefix or suffix:
/// - `7>` / `>7` → `>7`, `7<` / `<7` → `<7` (strict)
/// - `7+` / `>=7` / `7>=` → `≥7`, `<=7` / `7<=` → `≤7` (inclusive)
/// - `2..3` / `2-3` (and open `2..` / `..3` / `2-` / `-3`) → an inclusive range;
///   `..` and `-` are interchangeable separators (a value is never negative)
/// - a bare `6` → `=6` (the server widens it to a tolerance band)
///
/// Returns `None` for a blank field. Values must be finite and non-negative.
fn parse_range_criterion(label: &str, value: &str) -> Result<Option<RangeCriterion>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    // A range separator (`..` or `-`) denotes an inclusive range. Numbers here are
    // never negative, so any `-` is a separator, not a sign; comparison operators
    // (`>` `<` `>=` `<=` `+` `=`) never contain `-`, so this never shadows one.
    // Each side is optional and always inclusive (no strict range form).
    if let Some((low, high)) = value.split_once("..").or_else(|| value.split_once('-')) {
        let lower = parse_opt_bound(label, low)?;
        let upper = parse_opt_bound(label, high)?;
        if lower.is_none() && upper.is_none() {
            return Ok(None);
        }
        if let (Some(lo), Some(hi)) = (lower, upper)
            && lo.value > hi.value
        {
            return Err(format!(
                "{label}: min {} is greater than max {}",
                fmt_num(lo.value),
                fmt_num(hi.value)
            ));
        }
        return Ok(Some(RangeCriterion::Bounds { lower, upper }));
    }
    let (op, number) = split_range_op(value);
    let parsed = parse_num(label, number)?;
    Ok(Some(match op {
        RangeOp::Eq => RangeCriterion::Exact(parsed),
        RangeOp::Gt => RangeCriterion::Bounds {
            lower: Some(NumBound {
                value: parsed,
                inclusive: false,
            }),
            upper: None,
        },
        RangeOp::Ge => RangeCriterion::Bounds {
            lower: Some(NumBound {
                value: parsed,
                inclusive: true,
            }),
            upper: None,
        },
        RangeOp::Lt => RangeCriterion::Bounds {
            lower: None,
            upper: Some(NumBound {
                value: parsed,
                inclusive: false,
            }),
        },
        RangeOp::Le => RangeCriterion::Bounds {
            lower: None,
            upper: Some(NumBound {
                value: parsed,
                inclusive: true,
            }),
        },
    }))
}

/// Split a single-value token into its comparison operator and the numeric rest.
/// Operators are accepted as a prefix (`>7`) or suffix (`7>`); `+` is the
/// inclusive at-least suffix (`7+` ≡ `>=7`). A bare token is `=`. `-` never
/// reaches here — it is a range separator, split off before this is called.
/// Two-char operators are matched before one-char so `>=` never reads as `>`.
fn split_range_op(value: &str) -> (RangeOp, &str) {
    const PREFIX: &[(&str, RangeOp)] = &[
        (">=", RangeOp::Ge),
        ("<=", RangeOp::Le),
        (">", RangeOp::Gt),
        ("<", RangeOp::Lt),
        ("=", RangeOp::Eq),
    ];
    const SUFFIX: &[(&str, RangeOp)] = &[
        (">=", RangeOp::Ge),
        ("<=", RangeOp::Le),
        ("+", RangeOp::Ge),
        (">", RangeOp::Gt),
        ("<", RangeOp::Lt),
        ("=", RangeOp::Eq),
    ];
    for (token, op) in PREFIX {
        if let Some(rest) = value.strip_prefix(token) {
            return (*op, rest);
        }
    }
    for (token, op) in SUFFIX {
        if let Some(rest) = value.strip_suffix(token) {
            return (*op, rest);
        }
    }
    (RangeOp::Eq, value)
}

/// Parse one range side (`..` or `-`): blank = open (`None`), else inclusive.
fn parse_opt_bound(label: &str, part: &str) -> Result<Option<NumBound>, String> {
    let part = part.trim();
    if part.is_empty() {
        return Ok(None);
    }
    Ok(Some(NumBound {
        value: parse_num(label, part)?,
        inclusive: true,
    }))
}

/// Parse a bound value: finite, non-negative (every stat here is `≥ 0`).
fn parse_num(label: &str, part: &str) -> Result<f64, String> {
    let part = part.trim();
    if part.is_empty() {
        return Err(format!("{label}: a comparison needs a number"));
    }
    let value = part
        .parse::<f64>()
        .ok()
        // `f64::parse` accepts "nan"/"inf"; neither is a usable bound and NaN
        // would slip past the min>max guard, so reject at the boundary.
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("{label}: \"{part}\" is not a number"))?;
    if value < 0.0 {
        return Err(format!("{label}: \"{part}\" must be 0 or more"));
    }
    Ok(value)
}

/// Live plain-english reading of a range field, for the on-focus hint.
#[derive(Debug, Clone, PartialEq)]
pub enum RangeHint {
    /// The field is blank.
    Empty,
    /// The value parses; the string is its reading (`maps with 7 stars or
    /// higher`), example numbers wrapped in `[…]` for the highlight pass.
    Valid(String),
    /// The value fails to parse; the string is the reason.
    Invalid(String),
}

/// Interpret a range field's current value against the numeric grammar.
pub fn describe_range(label: &str, value: &str) -> RangeHint {
    match parse_range_criterion(label, value) {
        Ok(None) => RangeHint::Empty,
        Ok(Some(criterion)) => RangeHint::Valid(describe_criterion(label, &criterion)),
        Err(reason) => RangeHint::Invalid(reason),
    }
}

/// Plain-english reading of a parsed criterion; example numbers are wrapped in
/// `[…]` so the hint renderer can highlight them.
fn describe_criterion(label: &str, criterion: &RangeCriterion) -> String {
    // `length` values are seconds; every other field reads naturally as its label.
    let unit = if label == "length" { "seconds" } else { label };
    let n = |value: f64| format!("[{}]", fmt_num(value));
    match criterion {
        RangeCriterion::Exact(value) => format!("maps with exactly {} {unit}", n(*value)),
        RangeCriterion::Bounds {
            lower: Some(lo),
            upper: None,
        } if lo.inclusive => format!("maps with {} {unit} or higher", n(lo.value)),
        RangeCriterion::Bounds {
            lower: Some(lo),
            upper: None,
        } => format!("maps above {} {unit}", n(lo.value)),
        RangeCriterion::Bounds {
            lower: None,
            upper: Some(hi),
        } if hi.inclusive => format!("maps with {} {unit} or lower", n(hi.value)),
        RangeCriterion::Bounds {
            lower: None,
            upper: Some(hi),
        } => format!("maps below {} {unit}", n(hi.value)),
        RangeCriterion::Bounds {
            lower: Some(lo),
            upper: Some(hi),
        } => format!("maps between {} and {} {unit}", n(lo.value), n(hi.value)),
        // A `Bounds` with neither side is never produced (parse returns `None`).
        RangeCriterion::Bounds {
            lower: None,
            upper: None,
        } => String::new(),
    }
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
