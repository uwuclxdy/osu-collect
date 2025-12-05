use super::{
    home::InputField,
    messages::{AppMessage, MessageKind},
};
use crate::osu_db::{LocalBeatmapset, LocalCollection, OsuClient};
use ratatui::widgets::ListState;
use std::collections::{HashMap, HashSet};
use tracing::{debug, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatesField {
    ClientType,
    OsuPath,
    Collections,
    BeatmapList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatesAction {
    None,
    Download,
    RefreshAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanStatus {
    #[default]
    Idle,
    ReadingDatabase,
    FetchingCollection,
    Ready,
    Error,
}

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
}

#[derive(Debug, Clone)]
pub struct CollectionEntry {
    pub name: String,
    pub collection_id: Option<u64>,
    pub beatmap_count: usize,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub enum BeatmapDisplayItem {
    CollectionHeader { collection_id: u32 },
    Beatmap { cache_index: usize },
}

#[derive(Debug, Clone)]
pub struct PathState {
    pub client_type: OsuClient,
    pub osu_path: InputField,
}

impl PathState {
    fn new(client_type: OsuClient) -> Self {
        let default_path = Self::detect_default_path(client_type);
        Self {
            client_type,
            osu_path: InputField {
                label: "osu! path",
                value: default_path.clone(),
                placeholder: default_path,
            },
        }
    }

    fn detect_default_path(client: OsuClient) -> String {
        use crate::osu_db::{BeatmapReader, LazerReader, StableReader};

        match client {
            OsuClient::Stable => StableReader::default_path(),
            OsuClient::Lazer => LazerReader::default_path(),
        }
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct ScanState {
    pub local_beatmapsets: HashMap<u32, LocalBeatmapset>,
    pub all_local_checksums: HashSet<String>,
    pub scan_status: ScanStatus,
    pub scan_generation: u64,
}

impl ScanState {
    fn new() -> Self {
        Self {
            local_beatmapsets: HashMap::new(),
            all_local_checksums: HashSet::new(),
            scan_status: ScanStatus::Idle,
            scan_generation: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SelectionState {
    pub local_collections: Vec<CollectionEntry>,
    pub cached_missing_sets: Vec<MissingBeatmapset>,
    pub visible_missing: Vec<usize>,
    pub display_items: Vec<BeatmapDisplayItem>,
    pub collections_state: ListState,
    pub beatmaps_state: ListState,
    pub in_collection_list: bool,
    pub in_beatmap_list: bool,
    pub focus: UpdatesField,
}

impl SelectionState {
    fn new() -> Self {
        Self {
            local_collections: Vec::new(),
            cached_missing_sets: Vec::new(),
            visible_missing: Vec::new(),
            display_items: Vec::new(),
            collections_state: ListState::default(),
            beatmaps_state: ListState::default(),
            in_collection_list: false,
            in_beatmap_list: false,
            focus: UpdatesField::ClientType,
        }
    }
}

pub struct UpdatesTab {
    pub path: PathState,
    pub scan: ScanState,
    pub selection: SelectionState,
    pub message: Option<AppMessage>,
}

impl UpdatesTab {
    pub fn new() -> Self {
        let client_type = OsuClient::default();
        Self {
            path: PathState::new(client_type),
            scan: ScanState::new(),
            selection: SelectionState::new(),
            message: None,
        }
    }

    pub fn next_field(&mut self) {
        if self.selection.in_collection_list || self.selection.in_beatmap_list {
            return;
        }

        self.selection.focus = match self.selection.focus {
            UpdatesField::ClientType => UpdatesField::OsuPath,
            UpdatesField::OsuPath => UpdatesField::Collections,
            UpdatesField::Collections => UpdatesField::BeatmapList,
            UpdatesField::BeatmapList => UpdatesField::ClientType,
        };
    }

    pub fn prev_field(&mut self) {
        if self.selection.in_collection_list || self.selection.in_beatmap_list {
            return;
        }

        self.selection.focus = match self.selection.focus {
            UpdatesField::ClientType => UpdatesField::BeatmapList,
            UpdatesField::OsuPath => UpdatesField::ClientType,
            UpdatesField::Collections => UpdatesField::OsuPath,
            UpdatesField::BeatmapList => UpdatesField::Collections,
        };
    }

    pub fn handle_char(&mut self, ch: char) {
        if self.selection.in_collection_list || self.selection.in_beatmap_list {
            match ch {
                'a' => self.select_all(),
                'd' => self.deselect_all(),
                _ => {}
            }
            return;
        }

        if self.selection.focus == UpdatesField::OsuPath {
            self.path.osu_path.value.push(ch);
        }
    }

    pub fn backspace(&mut self) {
        if self.selection.focus == UpdatesField::OsuPath
            && !self.selection.in_collection_list
            && !self.selection.in_beatmap_list
        {
            self.path.osu_path.value.pop();
        }
    }

    pub fn toggle_current(&mut self) -> UpdatesAction {
        match self.selection.focus {
            UpdatesField::ClientType => {
                self.path.client_type.toggle();
                let new_path = PathState::detect_default_path(self.path.client_type);
                if self.path.osu_path.value.is_empty()
                    || self.path.osu_path.value == self.path.osu_path.placeholder
                {
                    self.path.osu_path.value = new_path.clone();
                }
                self.path.osu_path.placeholder = new_path;
                // Clear current data and trigger full rescan
                // Increment generation to invalidate any in-flight fetch tasks
                self.scan.scan_generation = self.scan.scan_generation.wrapping_add(1);
                self.selection.local_collections.clear();
                self.scan.all_local_checksums.clear();
                self.scan.local_beatmapsets.clear();
                self.selection.cached_missing_sets.clear();
                self.selection.visible_missing.clear();
                self.selection.display_items.clear();
                self.selection.collections_state = ListState::default();
                self.selection.beatmaps_state = ListState::default();
                self.selection.in_collection_list = false;
                self.selection.in_beatmap_list = false;
                self.scan.scan_status = ScanStatus::Idle;
                UpdatesAction::RefreshAll
            }
            UpdatesField::Collections => {
                if self.selection.in_collection_list {
                    self.toggle_collection_at_scroll();
                    UpdatesAction::None
                } else {
                    self.selection.in_collection_list = true;
                    UpdatesAction::None
                }
            }
            UpdatesField::BeatmapList => {
                if self.selection.in_beatmap_list {
                    self.toggle_beatmap_at_scroll();
                } else {
                    if !self.is_scan_ready() {
                        return UpdatesAction::None;
                    }
                    self.selection.in_beatmap_list = true;
                }
                UpdatesAction::None
            }
            _ => UpdatesAction::None,
        }
    }

    pub fn handle_enter(&mut self) -> UpdatesAction {
        if self.selection.in_collection_list {
            self.selection.in_collection_list = false;
            // Filter cached beatmaps based on newly selected collections
            self.filter_missing_from_cache();
            return UpdatesAction::None;
        }

        if self.selection.in_beatmap_list {
            self.selection.in_beatmap_list = false;
            return UpdatesAction::None;
        }

        // Pressing Enter anywhere (except exiting lists) triggers download
        if self.selected_beatmap_count() == 0 {
            UpdatesAction::None
        } else {
            UpdatesAction::Download
        }
    }

    pub fn handle_escape(&mut self) -> Option<UpdatesAction> {
        if self.selection.in_collection_list {
            self.selection.in_collection_list = false;
            // Filter cached beatmaps based on newly selected collections
            self.filter_missing_from_cache();
            return Some(UpdatesAction::None);
        }

        if self.selection.in_beatmap_list {
            self.selection.in_beatmap_list = false;
            return Some(UpdatesAction::None);
        }

        None
    }

    pub fn scroll_up(&mut self) {
        if self.selection.in_collection_list {
            let i = self.selection.collections_state.selected().unwrap_or(0);
            if i > 0 {
                self.selection.collections_state.select(Some(i - 1));
            }
        } else if self.selection.in_beatmap_list {
            let i = self.selection.beatmaps_state.selected().unwrap_or(0);
            if i > 0 {
                self.selection.beatmaps_state.select(Some(i - 1));
            }
        }
    }

    pub fn scroll_down(&mut self) {
        if self.selection.in_collection_list {
            let max = self.selection.local_collections.len().saturating_sub(1);
            let i = self.selection.collections_state.selected().unwrap_or(0);
            if i < max {
                self.selection.collections_state.select(Some(i + 1));
            }
        } else if self.selection.in_beatmap_list {
            let max = self.selection.display_items.len().saturating_sub(1);
            let i = self.selection.beatmaps_state.selected().unwrap_or(0);
            if i < max {
                self.selection.beatmaps_state.select(Some(i + 1));
            }
        }
    }

    fn toggle_collection_at_scroll(&mut self) {
        if let Some(idx) = self.selection.collections_state.selected()
            && let Some(collection) = self.selection.local_collections.get_mut(idx)
        {
            collection.selected = !collection.selected;
        }
    }

    fn toggle_beatmap_at_scroll(&mut self) {
        let Some(idx) = self.selection.beatmaps_state.selected() else {
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

    pub fn select_all(&mut self) {
        if self.selection.in_collection_list {
            for collection in &mut self.selection.local_collections {
                collection.selected = true;
            }
        } else if self.selection.in_beatmap_list {
            for &cache_idx in &self.selection.visible_missing {
                if let Some(beatmap) = self.selection.cached_missing_sets.get_mut(cache_idx) {
                    beatmap.selected = true;
                }
            }
        }
    }

    pub fn deselect_all(&mut self) {
        if self.selection.in_collection_list {
            for collection in &mut self.selection.local_collections {
                collection.selected = false;
            }
        } else if self.selection.in_beatmap_list {
            for &cache_idx in &self.selection.visible_missing {
                if let Some(beatmap) = self.selection.cached_missing_sets.get_mut(cache_idx) {
                    beatmap.selected = false;
                }
            }
        }
    }

    pub fn set_collections(&mut self, collections: Vec<LocalCollection>) {
        info!(
            total_collections = collections.len(),
            "Processing local collections for updatable IDs"
        );

        // Only keep collections that have a recognizable osu!collector ID
        self.selection.local_collections = collections
            .into_iter()
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
                        name: c.name,
                        collection_id,
                        beatmap_count: c.beatmap_checksums.len(),
                        selected: true,
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

        self.selection.collections_state.select(Some(0));
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.scan.scan_status = ScanStatus::Error;
        self.message = Some(AppMessage {
            kind: MessageKind::Error,
            text: message.into(),
        });
    }

    pub fn set_info(&mut self, message: impl Into<String>) {
        self.message = Some(AppMessage {
            kind: MessageKind::Info,
            text: message.into(),
        });
    }

    pub fn set_loading(&mut self, message: impl Into<String>) {
        self.message = Some(AppMessage {
            kind: MessageKind::Loading,
            text: message.into(),
        });
    }

    pub fn clear_message(&mut self) {
        self.message = None;
    }

    pub fn is_path_auto_detected(&self) -> bool {
        self.path.osu_path.value == self.path.osu_path.placeholder
    }

    pub fn selected_collection_count(&self) -> usize {
        self.selection
            .local_collections
            .iter()
            .filter(|c| c.selected)
            .count()
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

    pub fn osu_path(&self) -> &str {
        &self.path.osu_path.value
    }

    pub fn set_local_beatmapsets(&mut self, beatmapsets: Vec<LocalBeatmapset>) {
        self.scan.local_beatmapsets = beatmapsets.into_iter().map(|bs| (bs.id, bs)).collect();
    }

    pub fn set_all_checksums(&mut self, checksums: Vec<String>) {
        self.scan.all_local_checksums = checksums.into_iter().collect();
    }

    pub fn is_scan_ready(&self) -> bool {
        matches!(
            self.scan.scan_status,
            ScanStatus::Ready | ScanStatus::Idle | ScanStatus::Error
        )
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
                beatmap.selected = if had_selection {
                    previously_selected.contains(&beatmap.id)
                } else {
                    true
                };
                beatmap
            })
            .collect();
        self.filter_missing_from_cache();
    }

    pub fn filter_missing_from_cache(&mut self) {
        let selected_ids: HashSet<u64> = self
            .selection
            .local_collections
            .iter()
            .filter_map(|c| if c.selected { c.collection_id } else { None })
            .collect();

        self.selection.visible_missing = self
            .selection
            .cached_missing_sets
            .iter()
            .enumerate()
            .filter(|(_, beatmap)| selected_ids.contains(&(beatmap.collection_id as u64)))
            .map(|(idx, _)| idx)
            .collect();

        self.rebuild_display_items();
        self.selection.beatmaps_state.select(Some(0));
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
}

fn extract_collection_id(name: &str) -> Option<u64> {
    // Look for patterns like:
    // - "Collection Name-12345" (name-id format)
    // - "Collection Name - 12345" (name - id format)
    // - "#12345 - Collection Name" (legacy format)
    // - Any number with 2+ digits at the end after a separator
    let patterns = [
        regex_lite::Regex::new(r"[-–—]\s*(\d{2,})\s*$").ok()?, // trailing: name-id or name - id
        regex_lite::Regex::new(r"^\s*#?(\d{2,})\s*[-–—]").ok()?, // leading: #id - name
        regex_lite::Regex::new(r"\((\d{2,})\)\s*$").ok()?,     // trailing: name (id)
        regex_lite::Regex::new(r"\[(\d{2,})\]\s*$").ok()?,     // trailing: name [id]
    ];

    for pattern in &patterns {
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
