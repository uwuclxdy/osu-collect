use super::{
    first_field, last_field,
    messages::{AppMessage, clear_app_message},
    next_field, prev_field,
};

/// Cursor step for a list page-scroll (`Ctrl+d` / `Ctrl+u`).
pub(crate) const LIST_PAGE: i64 = 10;
use crate::osu_db::{LocalBeatmapset, LocalCollection, Md5};
use std::collections::HashSet;
use std::sync::OnceLock;
use tracing::{debug, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatesField {
    OsuPath,
    Collections,
    BeatmapList,
    /// The "download selected" button; activated with `enter`.
    Download,
}

const UPDATE_FIELDS: &[UpdatesField] = &[
    UpdatesField::OsuPath,
    UpdatesField::Collections,
    UpdatesField::BeatmapList,
    UpdatesField::Download,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatesAction {
    None,
    Download,
    RefreshAll,
    RecheckFailedMaps,
}

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

/// Sort order for the missing-beatmap list.
///
/// Cycles: `Default` → `Name` → `Status` → `Default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BeatmapSort {
    /// Original order from the failed-map check pass.
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
    pub selected: bool,
    pub previously_deleted: bool,
}

#[derive(Debug, Clone)]
pub struct CollectionEntry {
    pub name: String,
    pub collection_id: Option<u64>,
    pub beatmap_count: usize,
    pub selected: bool,
    /// Beatmaps present in the local snapshot but absent from the upstream collection.
    pub removed_count: usize,
}

#[derive(Debug, Clone)]
pub enum BeatmapDisplayItem {
    CollectionHeader { collection_id: u32 },
    Beatmap { cache_index: usize },
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

#[derive(Debug, Clone)]
pub struct SelectionState {
    pub local_collections: Vec<CollectionEntry>,
    /// Snapshot of `local_collections` in insertion order; used to restore `Default` sort.
    pub collections_default_order: Vec<CollectionEntry>,
    pub cached_missing_sets: Vec<MissingBeatmapset>,
    pub visible_missing: Vec<usize>,
    /// Snapshot of `visible_missing` in insertion order; used to restore `Default` sort.
    pub visible_missing_default_order: Vec<usize>,
    pub display_items: Vec<BeatmapDisplayItem>,
    pub collections_state: Option<usize>,
    pub beatmaps_state: Option<usize>,
    pub in_collection_list: bool,
    pub in_beatmap_list: bool,
    pub focus: UpdatesField,
    pub collection_sort: CollectionSort,
    pub beatmap_sort: BeatmapSort,
}

impl SelectionState {
    fn new() -> Self {
        Self {
            local_collections: Vec::new(),
            collections_default_order: Vec::new(),
            cached_missing_sets: Vec::new(),
            visible_missing: Vec::new(),
            visible_missing_default_order: Vec::new(),
            display_items: Vec::new(),
            collections_state: None,
            beatmaps_state: None,
            in_collection_list: false,
            in_beatmap_list: false,
            focus: UpdatesField::OsuPath,
            collection_sort: CollectionSort::Default,
            beatmap_sort: BeatmapSort::Default,
        }
    }
}

pub struct UpdatesTab {
    pub scan: ScanState,
    pub selection: SelectionState,
    pub message: Option<AppMessage>,
    /// Persisted list scroll offset so the focused row isn't re-pinned to the
    /// panel's bottom edge every frame (see [`widgets::render_list`]).
    pub list_offset: std::cell::Cell<usize>,
}

impl UpdatesTab {
    pub fn new() -> Self {
        Self {
            scan: ScanState::new(),
            selection: SelectionState::new(),
            message: None,
            list_offset: std::cell::Cell::new(0),
        }
    }

    pub fn next_field(&mut self) {
        if self.selection.in_collection_list || self.selection.in_beatmap_list {
            return;
        }

        self.selection.focus = next_field(UPDATE_FIELDS, self.selection.focus);
    }

    pub fn prev_field(&mut self) {
        if self.selection.in_collection_list || self.selection.in_beatmap_list {
            return;
        }

        self.selection.focus = prev_field(UPDATE_FIELDS, self.selection.focus);
    }

    pub fn first_field(&mut self) {
        if self.selection.in_collection_list || self.selection.in_beatmap_list {
            return;
        }
        self.selection.focus = first_field(UPDATE_FIELDS, self.selection.focus);
    }

    pub fn last_field(&mut self) {
        if self.selection.in_collection_list || self.selection.in_beatmap_list {
            return;
        }
        self.selection.focus = last_field(UPDATE_FIELDS, self.selection.focus);
    }

    /// Whether a beatmap/collection list is open (so list-scroll keys apply
    /// instead of field navigation).
    pub fn list_open(&self) -> bool {
        self.selection.in_collection_list || self.selection.in_beatmap_list
    }

    pub fn handle_char(&mut self, ch: char) {
        if self.selection.in_collection_list {
            match ch {
                'a' => self.set_all_selected(true),
                'd' => self.set_all_selected(false),
                's' => self.cycle_collection_sort(),
                _ => {}
            }
            return;
        }

        if self.selection.in_beatmap_list {
            match ch {
                'a' => self.set_all_selected(true),
                'd' => self.set_all_selected(false),
                's' => self.cycle_beatmap_sort(),
                _ => {}
            }
        }
    }

    /// Advance the collection sort mode and re-sort `local_collections` in place.
    pub fn cycle_collection_sort(&mut self) {
        self.selection.collection_sort = self.selection.collection_sort.next();
        self.apply_collection_sort();
    }

    fn apply_collection_sort(&mut self) {
        match self.selection.collection_sort {
            CollectionSort::Default => {
                self.selection.local_collections = self.selection.collections_default_order.clone();
            }
            CollectionSort::Name => {
                self.selection
                    .local_collections
                    .sort_by_key(|a| a.name.to_lowercase());
            }
            CollectionSort::Size => {
                self.selection
                    .local_collections
                    .sort_by_key(|c| std::cmp::Reverse(c.beatmap_count));
            }
        }
    }

    /// Advance the beatmap sort mode and re-sort `visible_missing` in place.
    pub fn cycle_beatmap_sort(&mut self) {
        self.selection.beatmap_sort = self.selection.beatmap_sort.next();
        self.apply_beatmap_sort();
        self.rebuild_display_items();
    }

    fn apply_beatmap_sort(&mut self) {
        match self.selection.beatmap_sort {
            BeatmapSort::Default => {
                self.selection.visible_missing =
                    self.selection.visible_missing_default_order.clone();
            }
            BeatmapSort::Name => {
                let cached = &self.selection.cached_missing_sets;
                self.selection.visible_missing.sort_by(|&a, &b| {
                    let name_a = cached
                        .get(a)
                        .map(|m| m.collection_name.as_str())
                        .unwrap_or("");
                    let name_b = cached
                        .get(b)
                        .map(|m| m.collection_name.as_str())
                        .unwrap_or("");
                    name_a.to_lowercase().cmp(&name_b.to_lowercase())
                });
            }
            BeatmapSort::Status => {
                let cached = &self.selection.cached_missing_sets;
                self.selection.visible_missing.sort_by_key(|&idx| {
                    cached
                        .get(idx)
                        .map(|m| m.previously_deleted as u8)
                        .unwrap_or(0)
                });
            }
        }
    }

    /// Whether the osu! path text field currently accepts edits — focused and
    /// no list panel is open.
    pub fn osu_path_editable(&self) -> bool {
        self.selection.focus == UpdatesField::OsuPath
            && !self.selection.in_collection_list
            && !self.selection.in_beatmap_list
    }

    /// Reset the scan for a fresh library after the app-global client switch
    /// (the client kind + path change lives on [`LibraryState::switch_client`]).
    /// Clears the prior client's scan data so the next scan rebuilds against the
    /// new library. Returns `RefreshAll` so the caller kicks off
    /// `ScanLocalDatabase`.
    ///
    /// [`LibraryState::switch_client`]: super::LibraryState::switch_client
    pub fn reset_for_client_switch(&mut self) -> UpdatesAction {
        // Increment generation to invalidate any in-flight fetch tasks.
        self.scan.scan_generation = self.scan.scan_generation.wrapping_add(1);
        self.selection.local_collections.clear();
        self.scan.all_local_checksums.clear();
        self.scan.local_beatmapsets.clear();
        self.selection.cached_missing_sets.clear();
        self.selection.visible_missing.clear();
        self.selection.display_items.clear();
        self.selection.collections_state = None;
        self.selection.beatmaps_state = None;
        self.selection.in_collection_list = false;
        self.selection.in_beatmap_list = false;
        self.scan.scan_status = ScanStatus::Idle;
        UpdatesAction::RefreshAll
    }

    /// Toggle the item under the scroll cursor in whichever list is currently
    /// open, independent of `selection.focus`. No-op when neither list is active.
    pub fn toggle_list_item(&mut self) {
        if self.selection.in_collection_list {
            self.toggle_collection_at_scroll();
        } else if self.selection.in_beatmap_list {
            self.toggle_beatmap_at_scroll();
        }
    }

    /// Returns `true` when the enter key should open a list panel rather than
    /// trigger a download. Mutates state to enter the panel as a side effect.
    /// An empty list is not expandable, so enter on it is a no-op.
    pub fn enter_opens_list(&mut self) -> bool {
        match self.selection.focus {
            UpdatesField::Collections if !self.selection.local_collections.is_empty() => {
                self.selection.in_collection_list = true;
                true
            }
            UpdatesField::BeatmapList if self.is_scan_ready() && self.total_missing_count() > 0 => {
                self.selection.in_beatmap_list = true;
                true
            }
            _ => false,
        }
    }

    pub fn handle_enter(&mut self) -> UpdatesAction {
        if self.selected_beatmap_count() == 0 {
            UpdatesAction::None
        } else {
            UpdatesAction::Download
        }
    }

    /// Returns `true` when the focused field accepts character input (i.e. the
    /// osu! path text box), meaning letter keybinds must be suppressed.
    pub fn is_typing(&self) -> bool {
        self.selection.focus == UpdatesField::OsuPath
    }

    pub fn handle_escape(&mut self) -> Option<UpdatesAction> {
        if self.selection.in_collection_list {
            self.selection.in_collection_list = false;
            // Filter cached beatmaps based on newly selected collections
            self.filter_cached();
            return Some(UpdatesAction::None);
        }

        if self.selection.in_beatmap_list {
            self.selection.in_beatmap_list = false;
            return Some(UpdatesAction::None);
        }

        None
    }

    pub fn scroll_up(&mut self) {
        self.scroll_by(-1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_by(1);
    }

    /// Page the open list up/down by [`LIST_PAGE`] rows (`Ctrl+u` / `Ctrl+d`).
    pub fn page_up(&mut self) {
        self.scroll_by(-LIST_PAGE);
    }

    pub fn page_down(&mut self) {
        self.scroll_by(LIST_PAGE);
    }

    /// Jump the open list cursor to the first (`top`) or last row (`gg` / `G`).
    pub fn scroll_to_edge(&mut self, top: bool) {
        let (state, len) = if self.selection.in_collection_list {
            (
                &mut self.selection.collections_state,
                self.selection.local_collections.len(),
            )
        } else if self.selection.in_beatmap_list {
            (
                &mut self.selection.beatmaps_state,
                self.selection.display_items.len(),
            )
        } else {
            return;
        };
        if len == 0 {
            return;
        }
        *state = Some(if top { 0 } else { len - 1 });
    }

    fn scroll_by(&mut self, delta: i64) {
        if self.selection.in_collection_list {
            scroll_list(
                &mut self.selection.collections_state,
                self.selection.local_collections.len(),
                delta,
            );
        } else if self.selection.in_beatmap_list {
            scroll_list(
                &mut self.selection.beatmaps_state,
                self.selection.display_items.len(),
                delta,
            );
        }
    }

    fn toggle_collection_at_scroll(&mut self) {
        if let Some(idx) = self.selection.collections_state
            && let Some(collection) = self.selection.local_collections.get_mut(idx)
        {
            collection.selected = !collection.selected;
        }
    }

    fn toggle_beatmap_at_scroll(&mut self) {
        let Some(idx) = self.selection.beatmaps_state else {
            return;
        };
        let Some(item) = self.selection.display_items.get(idx) else {
            return;
        };

        match item {
            BeatmapDisplayItem::CollectionHeader { collection_id } => {
                let matching: Vec<usize> = self
                    .selection
                    .visible_missing
                    .iter()
                    .copied()
                    .filter(|&cache_idx| {
                        self.selection
                            .cached_missing_sets
                            .get(cache_idx)
                            .map(|beatmap| beatmap.collection_id == *collection_id)
                            .unwrap_or(false)
                    })
                    .collect();

                let all_selected = matching.iter().all(|&idx| {
                    self.selection
                        .cached_missing_sets
                        .get(idx)
                        .map(|beatmap| beatmap.selected)
                        .unwrap_or(false)
                });

                for cache_idx in matching {
                    if let Some(beatmap) = self.selection.cached_missing_sets.get_mut(cache_idx) {
                        beatmap.selected = !all_selected;
                    }
                }
            }
            BeatmapDisplayItem::Beatmap { cache_index } => {
                if let Some(beatmap) = self.selection.cached_missing_sets.get_mut(*cache_index) {
                    beatmap.selected = !beatmap.selected;
                }
            }
        }
    }

    fn set_all_selected(&mut self, value: bool) {
        if self.selection.in_collection_list {
            for collection in &mut self.selection.local_collections {
                collection.selected = value;
            }
        } else if self.selection.in_beatmap_list {
            for &cache_idx in &self.selection.visible_missing {
                if let Some(beatmap) = self.selection.cached_missing_sets.get_mut(cache_idx) {
                    beatmap.selected = value;
                }
            }
        }
    }

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
        self.selection.collections_state = Some(0);
    }

    /// Mark the scan as errored and clear any in-flight loading status. The
    /// reason is surfaced as a toast by the caller ([`App::report_scan_error`]).
    ///
    /// [`App::report_scan_error`]: crate::app::App::report_scan_error
    pub fn mark_scan_error(&mut self) {
        self.scan.scan_status = ScanStatus::Error;
        clear_app_message(&mut self.message);
    }

    pub fn selected_beatmap_count(&self) -> usize {
        self.selection
            .visible_missing
            .iter()
            .filter(|&&cache_idx| {
                self.selection
                    .cached_missing_sets
                    .get(cache_idx)
                    .map(|beatmap| beatmap.selected)
                    .unwrap_or(false)
            })
            .count()
    }

    pub fn total_missing_count(&self) -> usize {
        self.selection.visible_missing.len()
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

    /// Whether entering the Updates tab should kick off a scan. Returns `false` once results
    /// are cached in memory or a scan is already in flight, so tab switching reuses prior
    /// results instead of redundantly re-checking.
    pub fn needs_initial_scan(&self) -> bool {
        matches!(self.scan.scan_status, ScanStatus::Idle | ScanStatus::Error)
    }

    pub fn set_missing_beatmaps(&mut self, missing: Vec<MissingBeatmapset>) {
        let previously_selected: HashSet<u32> = self
            .selection
            .cached_missing_sets
            .iter()
            .filter(|beatmap| beatmap.selected)
            .map(|beatmap| beatmap.id)
            .collect();

        let had_selection = !previously_selected.is_empty();

        self.selection.cached_missing_sets = missing
            .into_iter()
            .map(|mut beatmap| {
                // Previously-deleted items arrive with selected=false; preserve that unless
                // the user had explicitly re-selected them in a prior in-session refresh.
                if !beatmap.previously_deleted {
                    beatmap.selected = if had_selection {
                        previously_selected.contains(&beatmap.id)
                    } else {
                        true
                    };
                } else if had_selection && previously_selected.contains(&beatmap.id) {
                    beatmap.selected = true;
                }
                beatmap
            })
            .collect();
        self.filter_cached();
    }

    fn selected_collection_id_set(&self) -> HashSet<u64> {
        self.selection
            .local_collections
            .iter()
            .filter_map(|c| if c.selected { c.collection_id } else { None })
            .collect()
    }

    pub fn filter_cached(&mut self) {
        let selected_ids = self.selected_collection_id_set();

        self.selection.visible_missing = self
            .selection
            .cached_missing_sets
            .iter()
            .enumerate()
            .filter(|(_, beatmap)| selected_ids.contains(&(beatmap.collection_id as u64)))
            .map(|(idx, _)| idx)
            .collect();

        // Snapshot the natural filter order before any sort is applied.
        self.selection.visible_missing_default_order = self.selection.visible_missing.clone();
        self.apply_beatmap_sort();
        self.rebuild_display_items();
        self.selection.beatmaps_state = Some(0);
    }

    fn rebuild_display_items(&mut self) {
        self.selection.display_items.clear();
        let mut current_collection_id: Option<u32> = None;

        for &cache_idx in &self.selection.visible_missing {
            let Some(beatmap) = self.selection.cached_missing_sets.get(cache_idx) else {
                continue;
            };

            if current_collection_id != Some(beatmap.collection_id) {
                current_collection_id = Some(beatmap.collection_id);
                self.selection
                    .display_items
                    .push(BeatmapDisplayItem::CollectionHeader {
                        collection_id: beatmap.collection_id,
                    });
            }
            self.selection
                .display_items
                .push(BeatmapDisplayItem::Beatmap {
                    cache_index: cache_idx,
                });
        }
    }

    pub fn selected_collection_ids(&self) -> Vec<u64> {
        self.selection
            .local_collections
            .iter()
            .filter_map(|c| if c.selected { c.collection_id } else { None })
            .collect()
    }

    pub fn selected_beatmapset_ids(&self) -> Vec<u32> {
        self.selection
            .visible_missing
            .iter()
            .filter_map(|&cache_idx| {
                self.selection
                    .cached_missing_sets
                    .get(cache_idx)
                    .filter(|beatmap| beatmap.selected)
                    .map(|beatmap| beatmap.id)
            })
            .collect()
    }

    /// Beatmapset ids of every currently-visible missing row.
    pub fn all_visible_missing_ids(&self) -> Vec<u32> {
        self.selection
            .visible_missing
            .iter()
            .filter_map(|&idx| self.selection.cached_missing_sets.get(idx).map(|b| b.id))
            .collect()
    }

    /// Beatmapset ids targeted by an action on the focused beatmap-list row: the
    /// single set under a beatmap row, or every visible set in the collection
    /// under a header row. Empty when the list has no focused row.
    pub fn focused_missing_ids(&self) -> Vec<u32> {
        let Some(idx) = self.selection.beatmaps_state else {
            return Vec::new();
        };
        match self.selection.display_items.get(idx) {
            Some(BeatmapDisplayItem::Beatmap { cache_index }) => self
                .selection
                .cached_missing_sets
                .get(*cache_index)
                .map(|b| vec![b.id])
                .unwrap_or_default(),
            Some(BeatmapDisplayItem::CollectionHeader { collection_id }) => self
                .selection
                .visible_missing
                .iter()
                .filter_map(|&i| self.selection.cached_missing_sets.get(i))
                .filter(|b| b.collection_id == *collection_id)
                .map(|b| b.id)
                .collect(),
            None => Vec::new(),
        }
    }

    /// Drop the given set ids from the cached missing list and re-derive the
    /// visible rows so they disappear immediately (the "mark installed" action).
    pub fn hide_missing(&mut self, ids: &HashSet<u32>) {
        self.selection
            .cached_missing_sets
            .retain(|beatmap| !ids.contains(&beatmap.id));
        self.filter_cached();
    }
}

/// Moves the list cursor by `delta`, wrapping at both ends: stepping down past
/// the last item lands on the first, stepping up past the first lands on the
/// last (index arithmetic modulo `len`).
fn scroll_list(state: &mut Option<usize>, len: usize, delta: i64) {
    if len == 0 {
        return;
    }
    let len_i = len as i64;
    let i = state.unwrap_or(0) as i64;
    let next = (i + delta).rem_euclid(len_i) as usize;
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

impl Default for UpdatesTab {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app_updates_mod.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/app_updates.rs"]
mod integration;
