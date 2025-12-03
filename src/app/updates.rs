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
    Beatmap { beatmap_idx: usize },
}

pub struct UpdatesTab {
    pub client_type: OsuClient,
    pub osu_path: InputField,
    pub local_collections: Vec<CollectionEntry>,
    pub local_beatmapsets: HashMap<u32, LocalBeatmapset>,
    pub all_local_checksums: HashSet<String>,
    /// All missing beatmaps from all fetched collections (cache)
    pub cached_missing_sets: Vec<MissingBeatmapset>,
    /// Filtered missing beatmaps based on selected collections
    pub missing_sets: Vec<MissingBeatmapset>,
    pub selected_missing: HashSet<u32>,
    pub display_items: Vec<BeatmapDisplayItem>,
    pub scan_status: ScanStatus,
    pub focus: UpdatesField,
    pub collections_state: ListState,
    pub beatmaps_state: ListState,
    pub in_collection_list: bool,
    pub in_beatmap_list: bool,
    pub message: Option<AppMessage>,
    pub needs_scan: bool,
    /// Generation counter to invalidate stale fetch results
    pub scan_generation: u64,
}

impl UpdatesTab {
    pub fn new() -> Self {
        let default_path = Self::detect_default_path(OsuClient::default());

        Self {
            client_type: OsuClient::default(),
            osu_path: InputField {
                label: "osu! path",
                value: default_path.clone(),
                placeholder: default_path,
            },
            local_collections: Vec::new(),
            local_beatmapsets: HashMap::new(),
            all_local_checksums: HashSet::new(),
            cached_missing_sets: Vec::new(),
            missing_sets: Vec::new(),
            selected_missing: HashSet::new(),
            display_items: Vec::new(),
            scan_status: ScanStatus::Idle,
            focus: UpdatesField::ClientType,
            collections_state: ListState::default(),
            beatmaps_state: ListState::default(),
            in_collection_list: false,
            in_beatmap_list: false,
            message: None,
            needs_scan: true,
            scan_generation: 0,
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

    pub fn next_field(&mut self) {
        if self.in_collection_list || self.in_beatmap_list {
            return;
        }

        self.focus = match self.focus {
            UpdatesField::ClientType => UpdatesField::OsuPath,
            UpdatesField::OsuPath => UpdatesField::Collections,
            UpdatesField::Collections => UpdatesField::BeatmapList,
            UpdatesField::BeatmapList => UpdatesField::ClientType,
        };
    }

    pub fn prev_field(&mut self) {
        if self.in_collection_list || self.in_beatmap_list {
            return;
        }

        self.focus = match self.focus {
            UpdatesField::ClientType => UpdatesField::BeatmapList,
            UpdatesField::OsuPath => UpdatesField::ClientType,
            UpdatesField::Collections => UpdatesField::OsuPath,
            UpdatesField::BeatmapList => UpdatesField::Collections,
        };
    }

    pub fn handle_char(&mut self, ch: char) {
        if self.in_collection_list || self.in_beatmap_list {
            match ch {
                'a' => self.select_all(),
                'd' => self.deselect_all(),
                _ => {}
            }
            return;
        }

        if self.focus == UpdatesField::OsuPath {
            self.osu_path.value.push(ch);
        }
    }

    pub fn backspace(&mut self) {
        if self.focus == UpdatesField::OsuPath && !self.in_collection_list && !self.in_beatmap_list
        {
            self.osu_path.value.pop();
        }
    }

    pub fn toggle_current(&mut self) -> UpdatesAction {
        match self.focus {
            UpdatesField::ClientType => {
                self.client_type.toggle();
                let new_path = Self::detect_default_path(self.client_type);
                if self.osu_path.value.is_empty()
                    || self.osu_path.value == self.osu_path.placeholder
                {
                    self.osu_path.value = new_path.clone();
                }
                self.osu_path.placeholder = new_path;
                // Clear current data and trigger full rescan
                // Increment generation to invalidate any in-flight fetch tasks
                self.scan_generation = self.scan_generation.wrapping_add(1);
                self.local_collections.clear();
                self.all_local_checksums.clear();
                self.cached_missing_sets.clear();
                self.missing_sets.clear();
                self.selected_missing.clear();
                self.display_items.clear();
                self.scan_status = ScanStatus::Idle;
                UpdatesAction::RefreshAll
            }
            UpdatesField::Collections => {
                if self.in_collection_list {
                    self.toggle_collection_at_scroll();
                    UpdatesAction::None
                } else {
                    self.in_collection_list = true;
                    UpdatesAction::None
                }
            }
            UpdatesField::BeatmapList => {
                if self.in_beatmap_list {
                    self.toggle_beatmap_at_scroll();
                } else {
                    self.in_beatmap_list = true;
                }
                UpdatesAction::None
            }
            _ => UpdatesAction::None,
        }
    }

    pub fn handle_enter(&mut self) -> UpdatesAction {
        if self.in_collection_list {
            self.in_collection_list = false;
            // Filter cached beatmaps based on newly selected collections
            self.filter_missing_from_cache();
            return UpdatesAction::None;
        }

        if self.in_beatmap_list {
            self.in_beatmap_list = false;
            return UpdatesAction::None;
        }

        // Pressing Enter anywhere (except exiting lists) triggers download
        if self.selected_missing.is_empty() {
            UpdatesAction::None
        } else {
            UpdatesAction::Download
        }
    }

    pub fn handle_escape(&mut self) -> Option<UpdatesAction> {
        if self.in_collection_list {
            self.in_collection_list = false;
            // Filter cached beatmaps based on newly selected collections
            self.filter_missing_from_cache();
            return Some(UpdatesAction::None);
        }

        if self.in_beatmap_list {
            self.in_beatmap_list = false;
            return Some(UpdatesAction::None);
        }

        None
    }

    pub fn scroll_up(&mut self) {
        if self.in_collection_list {
            let i = self.collections_state.selected().unwrap_or(0);
            if i > 0 {
                self.collections_state.select(Some(i - 1));
            }
        } else if self.in_beatmap_list {
            let i = self.beatmaps_state.selected().unwrap_or(0);
            if i > 0 {
                self.beatmaps_state.select(Some(i - 1));
            }
        }
    }

    pub fn scroll_down(&mut self) {
        if self.in_collection_list {
            let max = self.local_collections.len().saturating_sub(1);
            let i = self.collections_state.selected().unwrap_or(0);
            if i < max {
                self.collections_state.select(Some(i + 1));
            }
        } else if self.in_beatmap_list {
            let max = self.display_items.len().saturating_sub(1);
            let i = self.beatmaps_state.selected().unwrap_or(0);
            if i < max {
                self.beatmaps_state.select(Some(i + 1));
            }
        }
    }

    fn toggle_collection_at_scroll(&mut self) {
        if let Some(idx) = self.collections_state.selected()
            && let Some(collection) = self.local_collections.get_mut(idx)
        {
            collection.selected = !collection.selected;
        }
    }

    fn toggle_beatmap_at_scroll(&mut self) {
        let Some(idx) = self.beatmaps_state.selected() else {
            return;
        };
        let Some(item) = self.display_items.get(idx) else {
            return;
        };

        match item {
            BeatmapDisplayItem::CollectionHeader { collection_id } => {
                let collection_id = *collection_id;
                let beatmap_ids: Vec<u32> = self
                    .missing_sets
                    .iter()
                    .filter(|b| b.collection_id == collection_id)
                    .map(|b| b.id)
                    .collect();

                let all_selected = beatmap_ids
                    .iter()
                    .all(|id| self.selected_missing.contains(id));
                if all_selected {
                    for id in beatmap_ids {
                        self.selected_missing.remove(&id);
                    }
                } else {
                    for id in beatmap_ids {
                        self.selected_missing.insert(id);
                    }
                }
            }
            BeatmapDisplayItem::Beatmap { beatmap_idx } => {
                if let Some(beatmap) = self.missing_sets.get(*beatmap_idx) {
                    let id = beatmap.id;
                    if self.selected_missing.contains(&id) {
                        self.selected_missing.remove(&id);
                    } else {
                        self.selected_missing.insert(id);
                    }
                }
            }
        }
    }

    pub fn select_all(&mut self) {
        if self.in_collection_list {
            for collection in &mut self.local_collections {
                collection.selected = true;
            }
        } else if self.in_beatmap_list {
            for beatmap in &self.missing_sets {
                self.selected_missing.insert(beatmap.id);
            }
        }
    }

    pub fn deselect_all(&mut self) {
        if self.in_collection_list {
            for collection in &mut self.local_collections {
                collection.selected = false;
            }
        } else if self.in_beatmap_list {
            self.selected_missing.clear();
        }
    }

    pub fn set_collections(&mut self, collections: Vec<LocalCollection>) {
        info!(
            total_collections = collections.len(),
            "Processing local collections for updatable IDs"
        );

        // Only keep collections that have a recognizable osu!collector ID
        self.local_collections = collections
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
            updatable = self.local_collections.len(),
            "Finished filtering updatable collections"
        );

        self.collections_state.select(Some(0));
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.scan_status = ScanStatus::Error;
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
        self.osu_path.value == self.osu_path.placeholder
    }

    pub fn selected_collection_count(&self) -> usize {
        self.local_collections.iter().filter(|c| c.selected).count()
    }

    pub fn selected_beatmap_count(&self) -> usize {
        self.selected_missing.len()
    }

    pub fn total_missing_count(&self) -> usize {
        self.missing_sets.len()
    }

    pub fn osu_path(&self) -> &str {
        &self.osu_path.value
    }

    pub fn set_local_beatmapsets(&mut self, beatmapsets: Vec<LocalBeatmapset>) {
        self.local_beatmapsets = beatmapsets.into_iter().map(|bs| (bs.id, bs)).collect();
    }

    pub fn set_all_checksums(&mut self, checksums: Vec<String>) {
        self.all_local_checksums = checksums.into_iter().collect();
    }

    pub fn has_local_data(&self) -> bool {
        !self.local_beatmapsets.is_empty() || !self.all_local_checksums.is_empty()
    }

    pub fn is_scan_ready(&self) -> bool {
        matches!(
            self.scan_status,
            ScanStatus::Ready | ScanStatus::Idle | ScanStatus::Error
        )
    }

    pub fn set_missing_beatmaps(&mut self, missing: Vec<MissingBeatmapset>) {
        // Store in cache and filter based on current selection
        self.cached_missing_sets = missing;
        self.filter_missing_from_cache();
    }

    pub fn filter_missing_from_cache(&mut self) {
        let selected_ids: HashSet<u64> = self
            .local_collections
            .iter()
            .filter_map(|c| if c.selected { c.collection_id } else { None })
            .collect();

        self.missing_sets = self
            .cached_missing_sets
            .iter()
            .filter(|m| selected_ids.contains(&(m.collection_id as u64)))
            .cloned()
            .collect();

        self.selected_missing = self.missing_sets.iter().map(|m| m.id).collect();
        self.rebuild_display_items();
        self.beatmaps_state.select(Some(0));

        let count = self.missing_sets.len();
        self.set_info(format!(" {count} missing beatmapsets"));
    }

    fn rebuild_display_items(&mut self) {
        self.display_items.clear();
        let mut current_collection_id: Option<u32> = None;

        for (idx, beatmap) in self.missing_sets.iter().enumerate() {
            if current_collection_id != Some(beatmap.collection_id) {
                current_collection_id = Some(beatmap.collection_id);
                self.display_items
                    .push(BeatmapDisplayItem::CollectionHeader {
                        collection_id: beatmap.collection_id,
                    });
            }
            self.display_items
                .push(BeatmapDisplayItem::Beatmap { beatmap_idx: idx });
        }
    }

    pub fn selected_collection_ids(&self) -> Vec<u64> {
        self.local_collections
            .iter()
            .filter_map(|c| if c.selected { c.collection_id } else { None })
            .collect()
    }

    pub fn selected_beatmapset_ids(&self) -> Vec<u32> {
        self.selected_missing.iter().copied().collect()
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
