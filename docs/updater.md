## Updates Tab Implementation Plan

### Overview

The Updates tab will allow users to check their local osu! installation for missing or outdated beatmaps from osu!collector collections and selectively download only what's needed. It will support both **osu! stable** (r/w via `osu_db` crate) and **osu! lazer** (read-only via RealmDB).

---

### 1. Database Access Layer (`src/osu_db/`)

Create a new module for reading local osu! installations:

#### 1.1 mod.rs
```
pub mod stable;
pub mod lazer;
pub mod common;

pub use common::{LocalBeatmap, LocalBeatmapset, OsuClient};
```

#### 1.2 `src/osu_db/common.rs`
- Define `LocalBeatmap` struct: `{ id: u32, checksum: String, beatmapset_id: u32 }`
- Define `LocalBeatmapset` struct: `{ id: u32, beatmaps: Vec<LocalBeatmap>, folder_name: String }`
- Define `OsuClient` enum: `Stable | Lazer`
- Trait `BeatmapReader`: `fn list_beatmaps(&self) -> Result<Vec<LocalBeatmapset>>`

#### 1.3 `src/osu_db/stable.rs`
- **Read** from `osu!.db` using existing `osu_db::Listing`
- Parse beatmap entries to extract: `beatmap_id`, `beatmapset_id`, `md5_hash`, `folder_name`
- Auto-detect osu! installation path:
  - Windows: `%localappdata%\osu!`
  - Linux (Wine): `~/.wine/drive_c/users/.../Local Settings/Application Data/osu!`
  - macOS: `/Applications/osu!.app/Contents/Resources/drive_c/Program Files/osu!/`

#### 1.4 `src/osu_db/lazer.rs`
- **Read-only** access to `client.realm` via RealmDB
- Add dependency: Consider SQLite-based approach since Realm stores data in SQLite-like format, or use `rusqlite` with realm file workaround
- Alternative: Parse the realm file's raw storage format (complex) or use realm-dart FFI bindings
- Extract: `beatmap_id`, `beatmapset_id`, `md5_hash` from `BeatmapInfo` table

---

### 2. App State for Updates Tab (`src/app/updates.rs`)

#### 2.1 State Model
```rust
pub enum UpdatesField {
    OsuPath,
    ClientType,      // Stable / Lazer toggle
    CollectionUrl,
    ScanButton,
    SelectAll,
    DeselectAll,
    BeatmapList,     // Scrollable list with checkboxes
}

pub struct UpdatesTab {
    pub osu_path: InputField,
    pub client_type: OsuClient,
    pub collection_url: InputField,
    pub local_beatmaps: Option<Vec<LocalBeatmapset>>,
    pub collection: Option<Collection>,
    pub missing_sets: Vec<MissingBeatmapset>,
    pub selected: HashSet<u32>,  // Selected beatmapset IDs for download
    pub scan_status: ScanStatus,
    pub focus: UpdatesField,
    pub scroll_offset: usize,
    pub message: Option<AppMessage>,
}

pub enum ScanStatus {
    Idle,
    ReadingDatabase,
    FetchingCollection,
    Comparing,
    Ready,
    Error(String),
}

pub struct MissingBeatmapset {
    pub id: u32,
    pub status: MissingStatus,
}

pub enum MissingStatus {
    NotInstalled,
    ChecksumMismatch,  // Different version installed
}
```

#### 2.2 Key Methods
- `scan_local_beatmaps()` - Reads local db
- `fetch_and_compare()` - Compares with collection
- `toggle_selection(beatmapset_id)` - Toggle individual selection
- `select_all()` / `deselect_all()`
- `build_update_request()` - Creates download request for selected

---

### 3. UI View (`src/tui/view/updates.rs`)

#### 3.1 Layout
```
╭─ Updates ───────────────────────────────────────────────────╮
│ osu! path: [/path/to/osu                                  ] │
│ Client: [● Stable ○ Lazer]                                  │
│ Collection: [https://osucollector.com/collections/...     ] │
│                                                             │
│ [Scan for Updates]                       Status: Ready (42) │
│─────────────────────────────────────────────────────────────│
│ Missing Beatmaps (15 selected / 42 total):                  │
│ ├─ [x] #123456 - Artist - Title (Not installed)            │
│ ├─ [ ] #123457 - Artist - Title (Version mismatch)         │
│ ├─ [x] #123458 - Artist - Title (Not installed)            │
│ └─ ... (scroll for more)                                    │
│─────────────────────────────────────────────────────────────│
│ [Select All] [Deselect All] [Download Selected]             │
╰─────────────────────────────────────────────────────────────╯
```

#### 3.2 Components
- `render_path_input()` - osu! path field with auto-detect button
- `render_client_toggle()` - Stable/Lazer radio buttons
- `render_collection_input()` - Collection URL/ID field
- `render_scan_button()` - Triggers scan
- `render_missing_list()` - Scrollable checkbox list with beatmap info
- `render_action_buttons()` - Select all, deselect, download

---

### 4. Integration with Existing Code

#### 4.1 Modify state.rs
```rust
const HOME_TAB_INDEX: usize = 0;
const UPDATES_TAB_INDEX: usize = 1;  // NEW
const CONFIG_TAB_INDEX: usize = 2;   // Was 1
const STATIC_TABS: usize = 3;        // Was 2

pub struct App {
    pub home: HomeTab,
    pub updates: UpdatesTab,  // NEW
    pub config: ConfigTab,
    // ...
}
```

#### 4.2 Modify mod.rs
```rust
pub mod updates;
pub use updates::{UpdatesField, UpdatesTab};
```

#### 4.3 Modify mod.rs
- Add `mod updates;`
- Update `AppView` to include `updates: UpdatesView`
- Update tab rendering match arm:
```rust
match view.active_tab {
    0 => home::render(...),
    1 => updates::render(...),  // NEW
    2 => config::render(...),
    _ => download::render(...),
}
```

#### 4.4 Update `tab_titles()` in state.rs
```rust
pub fn tab_titles(&self) -> Vec<String> {
    let mut titles = vec!["Home", "Updates", "Config"];
    // ...
}
```

---

### 5. New Commands & Events

#### 5.1 `AppCommand` additions
```rust
pub enum AppCommand {
    // existing...
    ScanLocalBeatmaps,
    FetchCollectionForUpdate { url: String },
    StartSelectiveDownload {
        id: DownloadId,
        request: SelectiveDownloadRequest
    },
}
```

#### 5.2 New async operations in runtime.rs
- Handle `ScanLocalBeatmaps` - spawn task to read local db
- Handle `FetchCollectionForUpdate` - fetch collection, compare, update state
- Handle `StartSelectiveDownload` - reuse existing download pipeline with filtered beatmapsets

---

### 6. Dependencies to Add (Cargo.toml)

```toml
# For lazer RealmDB (read-only)
rusqlite = { version = "0.32", features = ["bundled"] }
# Or for realm format parsing:
# realm-core = "..."  # If available
```

**Note:** Lazer's `client.realm` is a Realm database. Options:
1. Use Realm's SQLite-based backup format (if available)
2. Parse the raw realm file (complex, undocumented)
3. Implement a simplified reader for the specific tables needed
4. Initially, only support stable and add lazer later

---

### 7. Implementation Phases

#### Phase 1: Core Infrastructure
1. Create `src/osu_db/` module with stable db reader
2. Add `UpdatesTab` state structure
3. Add tab to UI navigation

#### Phase 2: Basic Scanning
1. Implement local beatmap reading (stable only)
2. Implement collection comparison logic
3. Display missing beatmaps list

#### Phase 3: Selective Download
1. Add selection checkboxes UI
2. Implement filtered download request
3. Integrate with existing download pipeline

#### Phase 4: Lazer Support
1. Research realm file format/access
2. Implement read-only lazer db reader
3. Add client type toggle to UI

---

### 8. Key Files to Create

| File | Purpose |
|------|---------|
| mod.rs | Module root |
| `src/osu_db/common.rs` | Shared types |
| `src/osu_db/stable.rs` | Stable db reader |
| `src/osu_db/lazer.rs` | Lazer db reader (read-only) |
| `src/app/updates.rs` | Updates tab state |
| `src/tui/view/updates.rs` | Updates tab UI |

---

### 9. Key Files to Modify

| File | Changes |
|------|---------|
| mod.rs | Export updates module |
| state.rs | Add updates tab, update tab indices |
| mod.rs | Add updates view, update routing |
| footer.rs | Update help text |
| Cargo.toml | Add realm/sqlite dependency |

---

### 10. Potential Challenges

1. **Lazer RealmDB Access**: Realm uses a proprietary format. May need to:
   - Use realm's C++ SDK via FFI
   - Parse raw file format (reverse engineering)
   - Wait for user to export data manually

2. **Large Databases**: `osu!.db` can be huge (100MB+). Need:
   - Streaming/lazy loading
   - Progress indicators
   - Background thread processing

3. **Path Detection**: Different OS layouts and custom install paths

4. **Collection Size**: Large collections (10K+ maps) need:
   - Pagination in UI
   - Efficient comparison (hash maps)
   - Batch selection operations
