use super::messages::{AppMessage, clear_app_message};

/// Rows a page key moves (`Ctrl+d` / `Ctrl+u`, `PageDown` / `PageUp`): the step
/// for a cursor list, and the same distance for a pane that scrolls by offset.
pub(crate) const PAGE_ROWS: usize = 10;
/// [`PAGE_ROWS`] as the signed step the cursor helpers take.
pub(crate) const LIST_PAGE: i64 = PAGE_ROWS as i64;
use super::find_source::{EnrichPager, EnrichSink, pruned_diff_ids};
use crate::osu_db::{LocalBeatmapset, LocalCollection, Md5};
use osu_downloader::search::BeatmapSetMeta;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use tracing::{debug, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanStatus {
    #[default]
    Idle,
    ReadingDatabase,
    FetchingCollection,
    CheckingFailedMaps,
    Ready,
    Error,
}

/// What activating the scan CTA on the update form should do, derived from the
/// scan status and the pending new-update count. Drives both the button label
/// ([`UpdateSource::scan_cta_label`]) and the enter dispatch in the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanCta {
    /// Start (or restart) a local scan.
    Scan,
    /// A scan is in flight; the button is inert.
    Busy,
}

/// Sort order for the collection list.
///
/// Cycles: `Default` → `Name` → `Size` → `Default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollectionSort {
    /// Original insertion order from the osu! database.
    #[default]
    Default,
    /// Case-insensitive alphabetical by collection name.
    Name,
    /// Largest beatmap count first.
    Size,
}

impl CollectionSort {
    /// Advance to the next sort mode.
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::Name,
            Self::Name => Self::Size,
            Self::Size => Self::Default,
        }
    }

    /// Short label shown in the section header.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => SORT_LABEL_DEFAULT,
            Self::Name => SORT_LABEL_NAME,
            Self::Size => SORT_LABEL_SIZE,
        }
    }
}

/// Sort order for the preview (missing-beatmap) list.
///
/// Cycles: `Default` → `Name` → `Status` → `Default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BeatmapSort {
    /// Original scan order.
    #[default]
    Default,
    /// Case-insensitive alphabetical by collection name.
    Name,
    /// Previously-deleted entries last.
    Status,
}

impl BeatmapSort {
    /// Advance to the next sort mode.
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::Name,
            Self::Name => Self::Status,
            Self::Status => Self::Default,
        }
    }

    /// Short label shown in the section header.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => SORT_LABEL_DEFAULT,
            Self::Name => SORT_LABEL_NAME,
            Self::Status => SORT_LABEL_STATUS,
        }
    }
}

const SORT_LABEL_DEFAULT: &str = "default";
const SORT_LABEL_NAME: &str = "name ↑";
const SORT_LABEL_SIZE: &str = "size ↓";
const SORT_LABEL_STATUS: &str = "status";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingStatus {
    NotInstalled,
}

#[derive(Debug, Clone)]
pub struct MissingBeatmapset {
    pub id: u32,
    pub status: MissingStatus,
    pub collection_id: u32,
    pub collection_name: String,
    /// Vestigial: selection is now whole-collection, so the download set is
    /// derived from the collection's `selected` flag, not this field. Kept so
    /// the scan-side construction in `runtime/scan.rs` still compiles.
    pub selected: bool,
    pub previously_deleted: bool,
    /// One diff (beatmap) id from this set, captured at scan time, to seed the
    /// osu-batch enrichment pager (`GET /beatmaps?ids[]=` keys on diff ids, not
    /// set ids). `None` when the upstream set carried no listable diff.
    pub enrich_diff_id: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct CollectionEntry {
    pub name: String,
    pub collection_id: Option<u64>,
    pub beatmap_count: usize,
    /// Whether this whole collection is included in the update download.
    pub selected: bool,
    /// Beatmaps present in the local snapshot but absent from the upstream collection.
    pub removed_count: usize,
}

#[derive(Debug, Clone)]
pub struct ScanState {
    pub local_collections_raw: Vec<LocalCollection>,
    pub local_beatmapsets: Vec<LocalBeatmapset>,
    pub all_local_checksums: HashSet<Md5>,
    pub scan_status: ScanStatus,
    pub scan_generation: u64,
    pub failed_beatmapset_count: usize,
}

impl ScanState {
    fn new() -> Self {
        Self {
            local_collections_raw: Vec::new(),
            local_beatmapsets: Vec::new(),
            all_local_checksums: HashSet::new(),
            scan_status: ScanStatus::Idle,
            scan_generation: 0,
            failed_beatmapset_count: 0,
        }
    }
}

/// A preview row's backing store, so the cursor can address both the live
/// missing list and the manually "marked installed" list in one combined pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewEntry {
    /// Index into `cached_missing_sets`.
    Missing(usize),
    /// Index into `marked_installed`.
    Marked(usize),
}

#[derive(Debug, Clone)]
pub struct SelectionState {
    /// The user's osu!collector collections; `.selected` = include in download.
    pub local_collections: Vec<CollectionEntry>,
    /// Snapshot of `local_collections` in insertion order; used to restore `Default` sort.
    pub collections_default_order: Vec<CollectionEntry>,
    /// Every missing beatmapset across all collections.
    pub cached_missing_sets: Vec<MissingBeatmapset>,
    /// Beatmapsets the user manually marked installed (hidden from the missing
    /// list) but can still reverse here — see `mark_installed_sets` /
    /// `unmark_installed_sets`. Mirrors `ignored-beatmapsets.json`, kept in sync
    /// at scan-land by `sync_marked_installed` so a reload restores the rows.
    pub marked_installed: Vec<MissingBeatmapset>,
    /// Cursor in the left collections list (an index into `local_collections`).
    pub collections_cursor: Option<usize>,
    /// Cursor within the highlighted collection's preview list.
    pub preview_cursor: Option<usize>,
    /// `false` = the update form; `true` = the two-pane browse.
    pub descended: bool,
    /// `false` = collections list focused; `true` = preview pane focused.
    pub preview_focused: bool,
    pub collection_sort: CollectionSort,
    /// Applies to the preview (highlighted collection's missing sets).
    pub beatmap_sort: BeatmapSort,
}

impl SelectionState {
    fn new() -> Self {
        Self {
            local_collections: Vec::new(),
            collections_default_order: Vec::new(),
            cached_missing_sets: Vec::new(),
            marked_installed: Vec::new(),
            collections_cursor: None,
            preview_cursor: None,
            descended: false,
            preview_focused: false,
            collection_sort: CollectionSort::Default,
            beatmap_sort: BeatmapSort::Default,
        }
    }
}

pub struct UpdateSource {
    pub scan: ScanState,
    pub selection: SelectionState,
    pub message: Option<AppMessage>,
    /// Persisted collections-list scroll offset (see [`widgets::render_list`]).
    pub list_offset: std::cell::Cell<usize>,
    /// Persisted preview-list scroll offset.
    pub preview_offset: std::cell::Cell<usize>,
    /// osu-batch backfill for the missing-set preview: the pager walks one diff
    /// id per missing set, folding the returned set metadata into `meta` (keyed by
    /// set id) so the preview rows read `artist - title` instead of a bare `#id`.
    enrich: EnrichPager,
    meta: HashMap<u32, BeatmapSetMeta>,
}

impl UpdateSource {
    pub fn new() -> Self {
        Self {
            scan: ScanState::new(),
            selection: SelectionState::new(),
            message: None,
            list_offset: std::cell::Cell::new(0),
            preview_offset: std::cell::Cell::new(0),
            enrich: EnrichPager::default(),
            meta: HashMap::new(),
        }
    }

    // ── missing-set enrichment ────────────────────────────────────────────────

    /// Seed the enrichment pager for the current missing sets (deduped by set id,
    /// first diff wins), rebuilding the preview `meta` from the session `cache` and
    /// paging only the sets it doesn't already know. Called at scan-land
    /// ([`set_missing_beatmaps`](Self::set_missing_beatmaps)) so the visible titles
    /// fill in without a keystroke and a re-enter / rescan never refetches a
    /// cached set. The runtime auto-fetches the first page after this.
    fn seed_enrichment(&mut self, cache: &HashMap<u32, BeatmapSetMeta>) {
        self.meta.clear();
        let mut seen: HashSet<u32> = HashSet::new();
        let mut seeds: Vec<(u32, Option<u32>)> = Vec::new();
        for set in &self.selection.cached_missing_sets {
            if !seen.insert(set.id) {
                continue;
            }
            if let Some(meta) = cache.get(&set.id) {
                self.meta.insert(set.id, meta.clone());
            }
            if let Some(diff) = set.enrich_diff_id {
                seeds.push((diff, Some(set.id)));
            }
        }
        self.enrich.seed(pruned_diff_ids(seeds, cache));
    }

    /// Folded set metadata for a missing set, once its enrichment page has landed.
    pub fn set_meta(&self, set_id: u32) -> Option<&BeatmapSetMeta> {
        self.meta.get(&set_id)
    }

    // ── browse descend / ascend ───────────────────────────────────────────────

    /// Descend from the form into the two-pane browse: focus the collections
    /// list with both cursors homed. A pure descend — the enrichment pager was
    /// seeded at scan-land ([`set_missing_beatmaps`](Self::set_missing_beatmaps)),
    /// so re-entering never reseeds or refetches. The app self-heals a missed
    /// prefetch by re-kicking page 1 only when nothing was ever fetched.
    pub fn descend(&mut self) {
        self.selection.descended = true;
        self.selection.preview_focused = false;
        self.selection.collections_cursor = Some(0);
        self.selection.preview_cursor = Some(0);
    }

    /// One step back out of the browse (drives `esc`): preview → collections
    /// list, then browse → form. Returns whether a step was consumed.
    pub fn ascend(&mut self) -> bool {
        if self.selection.preview_focused {
            self.selection.preview_focused = false;
            true
        } else if self.selection.descended {
            self.selection.descended = false;
            true
        } else {
            false
        }
    }

    /// Whether the browse (two-pane) view is active.
    pub fn is_browsing(&self) -> bool {
        self.selection.descended
    }

    /// Whether the preview pane currently holds focus (else the collections list).
    pub fn preview_focused(&self) -> bool {
        self.selection.preview_focused
    }

    /// Focus the preview pane — only when the highlighted collection has ≥1
    /// missing set to preview, else a no-op.
    pub fn focus_preview(&mut self) {
        if self.preview_len() > 0 {
            self.selection.preview_focused = true;
            if self.selection.preview_cursor.is_none() {
                self.selection.preview_cursor = Some(0);
            }
        }
    }

    /// Focus the collections list pane.
    pub fn focus_list(&mut self) {
        self.selection.preview_focused = false;
    }

    // ── scan CTA state machine ────────────────────────────────────────────────

    /// The action the scan CTA should take right now. The scan button only ever
    /// scans (or is inert while one runs); opening the browse is the separate
    /// `UpdateBrowse` button's job.
    pub fn scan_cta(&self) -> ScanCta {
        match self.scan.scan_status {
            ScanStatus::ReadingDatabase
            | ScanStatus::FetchingCollection
            | ScanStatus::CheckingFailedMaps => ScanCta::Busy,
            _ => ScanCta::Scan,
        }
    }

    /// Label for the scan CTA button, matching [`scan_cta`](Self::scan_cta).
    pub fn scan_cta_label(&self) -> String {
        match self.scan_cta() {
            ScanCta::Busy => "scanning…".to_string(),
            // A completed scan offers to re-scan; a fresh / errored one is the
            // first scan.
            ScanCta::Scan if self.scan.scan_status == ScanStatus::Ready => "rescan".to_string(),
            ScanCta::Scan => "scan for updates".to_string(),
        }
    }

    // ── collection selection ──────────────────────────────────────────────────

    /// Flip the checkbox on the collection under the list cursor. No-op when the
    /// cursor is parked on the action bar or on an inert (no-update) collection —
    /// a collection with nothing to download can't be selected.
    pub fn toggle_selected_collection(&mut self) {
        let Some(idx) = self.selection.collections_cursor else {
            return;
        };
        let with_new = self.collections_with_new_ids();
        if let Some(collection) = self.selection.local_collections.get_mut(idx)
            && entry_has_new(collection, &with_new)
        {
            collection.selected = !collection.selected;
        }
    }

    /// Select-all (`value == true`) only ticks collections that have updates;
    /// inert no-update ones stay deselected. Clear (`value == false`) drops all.
    pub fn set_all_collections_selected(&mut self, value: bool) {
        let with_new = self.collections_with_new_ids();
        for collection in &mut self.selection.local_collections {
            collection.selected = value && entry_has_new(collection, &with_new);
        }
    }

    // ── sorts ─────────────────────────────────────────────────────────────────

    /// Advance the collection sort mode and re-sort `local_collections` in place.
    pub fn cycle_collection_sort(&mut self) {
        self.selection.collection_sort = self.selection.collection_sort.next();
        self.apply_collection_sort();
    }

    fn apply_collection_sort(&mut self) {
        // No-update collections are inert, so they sink below the rest in every
        // mode; the active mode then orders within each partition. Precompute the
        // has-new set once — the borrow ends before the in-place sort.
        let with_new = self.collections_with_new_ids();
        let has_new = |c: &CollectionEntry| entry_has_new(c, &with_new);
        match self.selection.collection_sort {
            CollectionSort::Default => {
                // Reorder in place rather than cloning the snapshot: a clone would
                // reset every live `selected`/`removed_count` to its scan-time
                // default, silently re-including collections the user deselected.
                let order: std::collections::HashMap<(Option<u64>, String), usize> = self
                    .selection
                    .collections_default_order
                    .iter()
                    .enumerate()
                    .map(|(idx, c)| ((c.collection_id, c.name.clone()), idx))
                    .collect();
                self.selection.local_collections.sort_by(|a, b| {
                    has_new(b).cmp(&has_new(a)).then_with(|| {
                        let ka = order
                            .get(&(a.collection_id, a.name.clone()))
                            .copied()
                            .unwrap_or(usize::MAX);
                        let kb = order
                            .get(&(b.collection_id, b.name.clone()))
                            .copied()
                            .unwrap_or(usize::MAX);
                        ka.cmp(&kb)
                    })
                });
            }
            CollectionSort::Name => {
                self.selection.local_collections.sort_by(|a, b| {
                    has_new(b)
                        .cmp(&has_new(a))
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
            }
            CollectionSort::Size => {
                self.selection.local_collections.sort_by(|a, b| {
                    has_new(b)
                        .cmp(&has_new(a))
                        .then_with(|| b.beatmap_count.cmp(&a.beatmap_count))
                });
            }
        }
    }

    /// Advance the preview sort mode. The preview derivation
    /// ([`preview_missing_indices`](Self::preview_missing_indices)) applies it.
    pub fn cycle_preview_sort(&mut self) {
        self.selection.beatmap_sort = self.selection.beatmap_sort.next();
    }

    // ── highlighted collection / preview derivation ───────────────────────────

    /// The collection under the list cursor, or `None` when the list is empty.
    pub fn highlighted_collection(&self) -> Option<&CollectionEntry> {
        self.selection
            .collections_cursor
            .and_then(|idx| self.selection.local_collections.get(idx))
    }

    fn highlighted_collection_id_u32(&self) -> Option<u32> {
        self.highlighted_collection()
            .and_then(|c| c.collection_id)
            .and_then(|id| u32::try_from(id).ok())
    }

    /// Indices into `cached_missing_sets` for the highlighted collection's
    /// missing sets, ordered by the active preview sort.
    pub fn preview_missing_indices(&self) -> Vec<usize> {
        let Some(collection_id) = self.highlighted_collection_id_u32() else {
            return Vec::new();
        };

        let mut indices: Vec<usize> = self
            .selection
            .cached_missing_sets
            .iter()
            .enumerate()
            .filter(|(_, set)| set.collection_id == collection_id)
            .map(|(idx, _)| idx)
            .collect();

        self.sort_preview_indices(&mut indices, &self.selection.cached_missing_sets);
        indices
    }

    /// Combined preview rows for the highlighted collection: manually-marked-installed
    /// sets first (so they're immediately visible and restorable), then its missing
    /// sets (sorted). Orphan marked sets (no collection, e.g. from a prior
    /// session's file) are shown under every collection so they stay reachable
    /// and reversible.
    pub fn preview_entries(&self) -> Vec<PreviewEntry> {
        let collection_id = self.highlighted_collection_id_u32();
        let mut marked: Vec<usize> = self
            .selection
            .marked_installed
            .iter()
            .enumerate()
            .filter(|(_, set)| {
                collection_id.is_none_or(|id| set.collection_id == id || set.collection_id == 0)
            })
            .map(|(idx, _)| idx)
            .collect();
        self.sort_preview_indices(&mut marked, &self.selection.marked_installed);

        let mut missing: Vec<usize> = match collection_id {
            Some(id) => self
                .selection
                .cached_missing_sets
                .iter()
                .enumerate()
                .filter(|(_, set)| set.collection_id == id)
                .map(|(idx, _)| idx)
                .collect(),
            None => Vec::new(),
        };
        self.sort_preview_indices(&mut missing, &self.selection.cached_missing_sets);

        let mut entries = marked
            .into_iter()
            .map(PreviewEntry::Marked)
            .collect::<Vec<_>>();
        entries.extend(missing.into_iter().map(PreviewEntry::Missing));
        entries
    }

    fn sort_preview_indices(&self, indices: &mut [usize], sets: &[MissingBeatmapset]) {
        match self.selection.beatmap_sort {
            BeatmapSort::Default => {}
            BeatmapSort::Name => indices.sort_by(|&a, &b| {
                let name_a = sets
                    .get(a)
                    .map(|m| m.collection_name.as_str())
                    .unwrap_or("");
                let name_b = sets
                    .get(b)
                    .map(|m| m.collection_name.as_str())
                    .unwrap_or("");
                name_a.to_lowercase().cmp(&name_b.to_lowercase())
            }),
            BeatmapSort::Status => indices.sort_by_key(|&idx| {
                sets.get(idx)
                    .map(|m| m.previously_deleted as u8)
                    .unwrap_or(0)
            }),
        }
    }

    /// Number of preview rows (missing + marked) for the highlighted collection.
    pub fn preview_len(&self) -> usize {
        self.preview_entries().len()
    }

    /// Count of missing sets belonging to `collection_id` (the per-collection "N new" badge).
    pub fn new_count_for(&self, collection_id: u64) -> usize {
        self.selection
            .cached_missing_sets
            .iter()
            .filter(|set| u64::from(set.collection_id) == collection_id)
            .count()
    }

    /// Total missing sets across all collections.
    pub fn total_new_count(&self) -> usize {
        self.selection.cached_missing_sets.len()
    }

    /// Missing sets whose collection is selected (the download-button count).
    pub fn selected_new_count(&self) -> usize {
        let selected = self.selected_collection_id_set();
        self.selection
            .cached_missing_sets
            .iter()
            .filter(|set| selected.contains(&u64::from(set.collection_id)))
            .count()
    }

    /// Ids (as `u32`, matching the API) of collections with ≥1 pending missing
    /// set. Precomputed for the sort / selection guards so a per-entry check
    /// avoids re-scanning `cached_missing_sets` each time.
    fn collections_with_new_ids(&self) -> HashSet<u32> {
        self.selection
            .cached_missing_sets
            .iter()
            .map(|set| set.collection_id)
            .collect()
    }

    /// Collections that have at least one missing set.
    pub fn collections_with_new_count(&self) -> usize {
        self.selection
            .local_collections
            .iter()
            .filter(|c| {
                c.collection_id
                    .map(|id| self.new_count_for(id) > 0)
                    .unwrap_or(false)
            })
            .count()
    }

    // ── download id lists ─────────────────────────────────────────────────────

    fn selected_collection_id_set(&self) -> HashSet<u64> {
        self.selection
            .local_collections
            .iter()
            .filter_map(|c| if c.selected { c.collection_id } else { None })
            .collect()
    }

    pub fn selected_collection_ids(&self) -> Vec<u64> {
        self.selection
            .local_collections
            .iter()
            .filter_map(|c| if c.selected { c.collection_id } else { None })
            .collect()
    }

    /// Beatmapset ids of every missing set belonging to a selected collection
    /// (whole-collection download semantics).
    pub fn selected_beatmapset_ids(&self) -> Vec<u32> {
        let selected = self.selected_collection_id_set();
        self.selection
            .cached_missing_sets
            .iter()
            .filter(|set| selected.contains(&u64::from(set.collection_id)))
            .map(|set| set.id)
            .collect()
    }

    // ── ignored-maps (mark / unmark installed) ─────────────────────────────────

    /// The single missing-set id under the preview cursor (for `i` / `u`).
    pub fn preview_focused_id(&self) -> Vec<u32> {
        match self.preview_focused_entry() {
            Some(PreviewEntry::Missing(i)) => self
                .selection
                .cached_missing_sets
                .get(i)
                .map(|set| vec![set.id])
                .unwrap_or_default(),
            Some(PreviewEntry::Marked(i)) => self
                .selection
                .marked_installed
                .get(i)
                .map(|set| vec![set.id])
                .unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// The preview row under the cursor, addressed against the combined list.
    pub fn preview_focused_entry(&self) -> Option<PreviewEntry> {
        self.selection
            .preview_cursor
            .and_then(|cursor| self.preview_entries().get(cursor).copied())
    }

    pub fn preview_focused_is_marked(&self) -> bool {
        matches!(self.preview_focused_entry(), Some(PreviewEntry::Marked(_)))
    }

    /// Every missing-set id of the highlighted collection (for `I`).
    pub fn highlighted_collection_missing_ids(&self) -> Vec<u32> {
        self.preview_missing_indices()
            .iter()
            .filter_map(|&idx| self.selection.cached_missing_sets.get(idx))
            .map(|set| set.id)
            .collect()
    }

    /// Every marked-installed id of the highlighted collection (for `U`), plus
    /// any orphan marked sets (no collection) so they stay reversible anywhere.
    pub fn highlighted_collection_marked_ids(&self) -> Vec<u32> {
        let collection_id = self.highlighted_collection_id_u32();
        self.selection
            .marked_installed
            .iter()
            .filter(|set| {
                collection_id.is_none_or(|id| set.collection_id == id || set.collection_id == 0)
            })
            .map(|set| set.id)
            .collect()
    }

    /// Move the given missing sets into the marked-installed list (the manual
    /// "mark installed" action). The file write lives in the caller; this only
    /// reshapes the in-memory view so the rows re-home into the preview's marked
    /// group immediately.
    pub fn mark_installed_sets(&mut self, ids: &HashSet<u32>) {
        if ids.is_empty() {
            return;
        }
        let moved: Vec<MissingBeatmapset> = self
            .selection
            .cached_missing_sets
            .iter()
            .filter(|set| ids.contains(&set.id))
            .cloned()
            .collect();
        if moved.is_empty() {
            return;
        }
        self.selection
            .cached_missing_sets
            .retain(|set| !ids.contains(&set.id));
        self.selection.marked_installed.extend(moved);
        // The marked group now leads the combined preview, so the row the user
        // just acted on shifts position — keep the cursor on it.
        self.rehome_preview_cursor_to(ids);
        self.clamp_preview_cursor();
    }

    /// Reverse of [`mark_installed_sets`]: move the given marked sets back into the
    /// missing list so they reappear at once (no rescan needed). The file prune
    /// lives in the caller. In-memory moves keep full data; id-only file entries
    /// (prior session, `collection_id == 0`) have no `MissingBeatmapset` to
    /// restore, so they drop from view and reappear as missing on the next scan.
    /// Orphan placeholders (`collection_id == 0`) are skipped entirely to avoid
    /// inflating `total_new_count()`.
    pub fn unmark_installed_sets(&mut self, ids: &HashSet<u32>) {
        if ids.is_empty() {
            return;
        }
        let moved: Vec<MissingBeatmapset> = self
            .selection
            .marked_installed
            .iter()
            .filter(|set| ids.contains(&set.id) && set.collection_id != 0)
            .cloned()
            .collect();
        self.selection
            .marked_installed
            .retain(|set| !ids.contains(&set.id));
        if !moved.is_empty() {
            self.selection.cached_missing_sets.extend(moved);
        }
        // Re-home the cursor onto a restored row (marked-first ordering moves it);
        // an orphan-only unmark leaves nothing in the preview, so the cursor just
        // clamps.
        self.rehome_preview_cursor_to(ids);
        self.clamp_preview_cursor();
    }

    /// After moving rows between the missing/marked groups, point `preview_cursor`
    /// at the first moved set still present in the combined preview, so the
    /// keyboard cursor keeps tracking the row the user just acted on (its index
    /// shifts because the marked group now leads the preview).
    fn rehome_preview_cursor_to(&mut self, ids: &HashSet<u32>) {
        let entries = self.preview_entries();
        let pos = entries.iter().position(|e| {
            let id = match e {
                PreviewEntry::Missing(i) => {
                    self.selection.cached_missing_sets.get(*i).map(|s| s.id)
                }
                PreviewEntry::Marked(i) => self.selection.marked_installed.get(*i).map(|s| s.id),
            };
            id.is_some_and(|id| ids.contains(&id))
        });
        if let Some(pos) = pos {
            self.selection.preview_cursor = Some(pos);
        }
    }

    /// Reconcile the in-memory marked list with the on-disk ignored set after a
    /// scan: drop rows the file no longer lists (genuine install / external
    /// prune) and add id-only placeholders for file entries not held in memory
    /// (prior session), so they remain visible and reversible.
    pub fn sync_marked_installed(&mut self, still_ignored: &HashSet<u32>) {
        self.selection
            .marked_installed
            .retain(|set| still_ignored.contains(&set.id));
        let held: HashSet<u32> = self
            .selection
            .marked_installed
            .iter()
            .map(|s| s.id)
            .collect();
        for id in still_ignored {
            if held.contains(id) {
                continue;
            }
            self.selection.marked_installed.push(MissingBeatmapset {
                id: *id,
                status: MissingStatus::NotInstalled,
                collection_id: 0,
                collection_name: String::new(),
                selected: false,
                previously_deleted: false,
                enrich_diff_id: None,
            });
        }
    }

    fn clamp_preview_cursor(&mut self) {
        let len = self.preview_len();
        match self.selection.preview_cursor {
            Some(c) if c >= len => {
                self.selection.preview_cursor = if len == 0 { None } else { Some(len - 1) };
            }
            _ => {}
        }
    }

    // ── scroll ────────────────────────────────────────────────────────────────

    pub fn scroll_up(&mut self) {
        self.scroll_by(-1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_by(1);
    }

    /// Page the focused pane up/down by [`LIST_PAGE`] rows (`Ctrl+u` / `Ctrl+d`),
    /// clamped at the ends (paging never wraps — unlike a single step).
    pub fn page_up(&mut self) {
        self.page_by(-LIST_PAGE);
    }

    pub fn page_down(&mut self) {
        self.page_by(LIST_PAGE);
    }

    /// Jump the focused pane's cursor to the first (`top`) or last row (`gg` / `G`).
    pub fn scroll_to_edge(&mut self, top: bool) {
        if self.selection.preview_focused {
            let len = self.preview_len();
            if len > 0 {
                self.selection.preview_cursor = Some(if top { 0 } else { len - 1 });
            }
        } else {
            let len = self.list_nav_len();
            if len > 0 {
                self.selection.collections_cursor = Some(if top { 0 } else { len - 1 });
                self.selection.preview_cursor = Some(0);
            }
        }
    }

    /// Number of navigable rows in the collections list.
    fn list_nav_len(&self) -> usize {
        self.selection.local_collections.len()
    }

    fn scroll_by(&mut self, delta: i64) {
        if self.selection.preview_focused {
            let len = self.preview_len();
            scroll_list(&mut self.selection.preview_cursor, len, delta);
        } else {
            let len = self.list_nav_len();
            scroll_list(&mut self.selection.collections_cursor, len, delta);
            // A new highlighted collection resets the preview to its top.
            self.selection.preview_cursor = Some(0);
        }
    }

    /// Like [`scroll_by`](Self::scroll_by) but clamps at the ends (paging).
    fn page_by(&mut self, delta: i64) {
        if self.selection.preview_focused {
            let len = self.preview_len();
            scroll_list_clamped(&mut self.selection.preview_cursor, len, delta);
        } else {
            let len = self.list_nav_len();
            scroll_list_clamped(&mut self.selection.collections_cursor, len, delta);
            self.selection.preview_cursor = Some(0);
        }
    }

    // ── scan pipeline setters ─────────────────────────────────────────────────

    pub fn set_collections(&mut self, collections: Vec<LocalCollection>) {
        info!(
            total_collections = collections.len(),
            "Processing local collections for updatable IDs"
        );

        self.scan.local_collections_raw = collections;

        // Only keep collections that have a recognizable osu!collector ID
        self.selection.local_collections = self
            .scan
            .local_collections_raw
            .iter()
            .filter_map(|c| {
                let collection_id = extract_collection_id(&c.name);
                if collection_id.is_some() {
                    debug!(
                        name = %c.name,
                        extracted_id = ?collection_id,
                        beatmap_count = c.beatmap_checksums.len(),
                        "Included updatable collection"
                    );
                    Some(CollectionEntry {
                        name: c.name.clone(),
                        collection_id,
                        beatmap_count: c.beatmap_checksums.len(),
                        selected: true,
                        removed_count: 0,
                    })
                } else {
                    debug!(name = %c.name, "Skipped collection without ID");
                    None
                }
            })
            .collect();

        info!(
            updatable = self.selection.local_collections.len(),
            "Finished filtering updatable collections"
        );

        // Snapshot the insertion order so we can restore it when cycling back to Default.
        self.selection.collections_default_order = self.selection.local_collections.clone();
        self.apply_collection_sort();
        self.selection.collections_cursor = Some(0);
    }

    /// Mark the scan as errored and clear any in-flight loading status. The
    /// reason is surfaced as a toast by the caller ([`App::report_scan_error`]).
    ///
    /// [`App::report_scan_error`]: crate::app::App::report_scan_error
    pub fn mark_scan_error(&mut self) {
        self.scan.scan_status = ScanStatus::Error;
        clear_app_message(&mut self.message);
    }

    pub fn set_local_beatmapsets(&mut self, beatmapsets: Vec<LocalBeatmapset>) {
        self.scan.local_beatmapsets = beatmapsets;
    }

    pub fn set_all_checksums(&mut self, checksums: Vec<Md5>) {
        self.scan.all_local_checksums = checksums.into_iter().collect();
    }

    pub fn set_failed_beatmapset_count(&mut self, count: usize) {
        self.scan.failed_beatmapset_count = count;
    }

    /// Apply per-collection removed-beatmap counts to the collection list.
    ///
    /// `counts` maps collection_id (as `u32`, matching the API) to the number of local
    /// checksums absent from the upstream collection at the time of the scan.
    pub fn set_removed_counts(&mut self, counts: &std::collections::HashMap<u32, usize>) {
        for entry in &mut self.selection.local_collections {
            if let Some(cid) = entry.collection_id.and_then(|id| u32::try_from(id).ok()) {
                entry.removed_count = counts.get(&cid).copied().unwrap_or(0);
            }
        }
        // Keep the default-order snapshot in sync so cycling back to Default restores the counts.
        for entry in &mut self.selection.collections_default_order {
            if let Some(cid) = entry.collection_id.and_then(|id| u32::try_from(id).ok()) {
                entry.removed_count = counts.get(&cid).copied().unwrap_or(0);
            }
        }
    }

    pub fn can_recheck_failed_maps(&self) -> bool {
        self.scan.failed_beatmapset_count > 0 && self.is_scan_ready()
    }

    pub fn is_scan_ready(&self) -> bool {
        matches!(
            self.scan.scan_status,
            ScanStatus::Ready | ScanStatus::Idle | ScanStatus::Error
        )
    }

    /// Whether a first scan is still owed (nothing cached, none in flight).
    pub fn needs_initial_scan(&self) -> bool {
        matches!(self.scan.scan_status, ScanStatus::Idle | ScanStatus::Error)
    }

    /// Store the freshly scanned missing sets. Selection is whole-collection, so
    /// there is no per-map selection to preserve — the collection checkboxes
    /// (defaulted to selected by [`set_collections`](Self::set_collections))
    /// decide the download set. Seeds the missing-set enrichment pager here (at
    /// scan-land, the earliest known intent) against the session `cache`, so the
    /// preview titles are already loading before the user opens the browse.
    pub fn set_missing_beatmaps(
        &mut self,
        missing: Vec<MissingBeatmapset>,
        cache: &HashMap<u32, BeatmapSetMeta>,
    ) {
        self.selection.cached_missing_sets = missing;
        // No-update collections are inert: force them deselected (nothing to
        // download) so a default-selected empty collection can't ride along, then
        // re-sort so they sink below the rest.
        let with_new = self.collections_with_new_ids();
        for collection in &mut self.selection.local_collections {
            if !entry_has_new(collection, &with_new) {
                collection.selected = false;
            }
        }
        self.apply_collection_sort();
        self.selection.preview_cursor = Some(0);
        self.seed_enrichment(cache);
    }

    /// Drop the given set ids from the cached missing list (the "mark installed"
    /// action) and re-home the preview cursor.
    pub fn hide_missing(&mut self, ids: &HashSet<u32>) {
        self.selection
            .cached_missing_sets
            .retain(|set| !ids.contains(&set.id));
        self.selection.preview_cursor = Some(0);
    }

    /// Reset the scan for a fresh library after the app-global client switch
    /// (the client kind + path change lives on [`LibraryState::switch_client`]).
    /// Clears the prior client's scan data but does NOT auto-scan — the user
    /// scans manually from the update form.
    ///
    /// [`LibraryState::switch_client`]: super::LibraryState::switch_client
    pub fn reset_for_client_switch(&mut self) {
        // Increment generation to invalidate any in-flight fetch tasks.
        self.scan.scan_generation = self.scan.scan_generation.wrapping_add(1);
        self.selection.local_collections.clear();
        self.selection.collections_default_order.clear();
        self.scan.all_local_checksums.clear();
        self.scan.local_beatmapsets.clear();
        self.selection.cached_missing_sets.clear();
        self.selection.marked_installed.clear();
        self.selection.collections_cursor = None;
        self.selection.preview_cursor = None;
        self.selection.descended = false;
        self.selection.preview_focused = false;
        self.scan.scan_status = ScanStatus::Idle;
        // Drop the prior client's enrichment: no live pager or stale titles carry
        // into the next scan (seeding now happens at scan-land, not on descend).
        self.enrich.clear();
        self.meta.clear();
    }
}

/// Whether `entry` has any pending update, given a precomputed set of collection
/// ids that do (see [`UpdateSource::collections_with_new_ids`]). A no-update
/// collection is inert: unselectable and sorted to the bottom.
fn entry_has_new(entry: &CollectionEntry, with_new: &HashSet<u32>) -> bool {
    entry
        .collection_id
        .and_then(|id| u32::try_from(id).ok())
        .map(|id| with_new.contains(&id))
        .unwrap_or(false)
}

/// Moves the list cursor by `delta`, wrapping at both ends: stepping down past
/// the last item lands on the first, stepping up past the first lands on the
/// last (index arithmetic modulo `len`). Shared with [`SetBrowse`] scrolling.
///
/// [`SetBrowse`]: super::find_source::SetBrowse
pub(crate) fn scroll_list(state: &mut Option<usize>, len: usize, delta: i64) {
    if len == 0 {
        return;
    }
    let len_i = len as i64;
    let i = state.unwrap_or(0) as i64;
    let next = (i + delta).rem_euclid(len_i) as usize;
    *state = Some(next);
}

/// Moves the list cursor by `delta`, clamping at both ends (no wrap). Paging is a
/// "jump toward an end" gesture, so it stops at the first / last row rather than
/// wrapping like a single [`scroll_list`] step. Shared with [`SetBrowse`] paging.
///
/// [`SetBrowse`]: super::find_source::SetBrowse
pub(crate) fn scroll_list_clamped(state: &mut Option<usize>, len: usize, delta: i64) {
    if len == 0 {
        return;
    }
    let last = (len - 1) as i64;
    let i = state.unwrap_or(0) as i64;
    let next = (i + delta).clamp(0, last) as usize;
    *state = Some(next);
}

fn collection_id_patterns() -> &'static [regex_lite::Regex; 4] {
    static PATTERNS: OnceLock<[regex_lite::Regex; 4]> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            regex_lite::Regex::new(r"[-–—]\s*(\d{2,})\s*$").expect("valid regex"),
            regex_lite::Regex::new(r"^\s*#?(\d{2,})\s*[-–—]").expect("valid regex"),
            regex_lite::Regex::new(r"\((\d{2,})\)\s*$").expect("valid regex"),
            regex_lite::Regex::new(r"\[(\d{2,})\]\s*$").expect("valid regex"),
        ]
    })
}

pub fn extract_collection_id(name: &str) -> Option<u64> {
    for pattern in collection_id_patterns() {
        if let Some(caps) = pattern.captures(name)
            && let Some(m) = caps.get(1)
            && let Ok(id) = m.as_str().parse()
        {
            return Some(id);
        }
    }

    None
}

impl EnrichSink for UpdateSource {
    fn enrich_generation(&self) -> u64 {
        self.enrich.generation()
    }

    fn enrich_cursor(&self) -> usize {
        self.enrich.cursor()
    }

    fn next_enrich_page(&mut self) -> Option<Vec<u32>> {
        self.enrich.next_page()
    }

    fn rewind_enrichment(&mut self, cursor: usize) {
        self.enrich.rewind(cursor);
    }

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

    /// Fold set-level metadata into the missing-set cache (keyed by set id). Only
    /// sets not already known are inserted, so a re-fetch never clobbers a title.
    fn fold_meta(&mut self, meta_by_set: HashMap<u32, BeatmapSetMeta>) {
        for (set_id, meta) in meta_by_set {
            self.meta.entry(set_id).or_insert(meta);
        }
    }
}

impl Default for UpdateSource {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app_update_source_mod.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/app_update_source.rs"]
mod integration;
