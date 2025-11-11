# osu!collector Downloader - Project Requirements

## Overview
A CLI tool to download osu! beatmap collections from osucollector.com using Rust.

## Project Structure
```
osu-collect/
├── Cargo.toml
├── README.md
├── src/
│   ├── main.rs           # Entry point, CLI parsing
│   ├── collector.rs      # osucollector.com API client
│   ├── downloader.rs     # Beatmap download logic
│   ├── config.rs         # Configuration handling
│   ├── lazer.rs          # osu!lazer integration (Realm DB, import)
│   └── error.rs          # Custom error types
└── config.toml           # Default configuration file (optional)
```

## Technologies & Dependencies

### Required Crates
```toml
[dependencies]
# CLI argument parsing
clap = { version = "4.5", features = ["derive"] }

# HTTP client for API requests and downloads
reqwest = { version = "0.12", features = ["json", "blocking"] }

# JSON serialization/deserialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Error handling
anyhow = "1.0"
thiserror = "1.0"

# Progress bar
indicatif = "0.17"

# URL parsing
url = "2.5"

# Configuration file support (optional)
toml = "0.8"
dirs = "5.0"

# For --lazer flag: Realm database reading
realm_db_reader = "0.2.0"  # Read-only Realm database access

# For --lazer flag: SHA-256 hashing
sha2 = "0.10"

# For --lazer flag: ZIP file creation
zip = "0.6"
```

## API Endpoints

### 1. osucollector.com API
**Endpoint**: `GET https://osucollector.com/api/collections/{collection_id}`

**Example Request**:
```
GET https://osucollector.com/api/collections/17503
```

**Response Structure**:
```json
{
  "id": 17503,
  "name": "aim slop",
  "uploader": {...},
  "beatmapsets": [
    {
      "id": 123456,
      "artist": "Artist Name",
      "title": "Song Title",
      "creator": "Mapper",
      "favouriteCount": 100,
      ...
    }
  ]
}
```

**Important**: Extract `beatmapsets[].id` to get beatmap set IDs.

### 2. Nerinyan Mirror API
**Endpoint**: `GET https://api.nerinyan.moe/d/{beatmapset_id}`

**Example Request**:
```
GET https://api.nerinyan.moe/d/123456
```

**Response**: Binary `.osz` file (ZIP archive containing beatmap files)

**Headers**: The Content-Disposition header contains the filename.

**Alternative mirrors** (configurable):
- `https://catboy.best/d/{beatmapset_id}`
- `https://api.chimu.moe/v1/download/{beatmapset_id}?n=1`

## Functional Requirements

### 1. CLI Interface
```bash
osu-collect -c "https://osucollector.com/collections/17503/aim-slop" -d ~/Downloads
```

**Arguments**:
- `-c, --collection <URL>`: Collection URL (required)
- `-d, --directory <PATH>`: Download directory (required, ignored with --lazer)
- `-m, --mirror <URL>`: Mirror base URL (optional, default: nerinyan.moe)
- `-y, --yes`: Skip confirmation for existing files (auto-overwrite)
- `--skip-existing`: Skip existing files without asking
- `--lazer`: Import directly into osu!lazer (auto-detects lazer directory)
- `--lazer-dir <PATH>`: Specify custom osu!lazer data directory
- `--collection-name <NAME>`: Name for the collection in lazer (default: collection name from osucollector)
- `-h, --help`: Show help
- `-V, --version`: Show version

### 2. URL Parsing
Extract collection ID from various URL formats:
- `https://osucollector.com/collections/17503`
- `https://osucollector.com/collections/17503/aim-slop`
- `17503` (direct ID)

### 3. Collection Fetching
1. Parse collection URL to extract ID
2. Make GET request to osucollector API
3. Parse JSON response
4. Extract all beatmapset IDs from `beatmapsets` array
5. Display collection info (name, uploader, total maps)

### 4. Download Process
For each beatmap set ID:
1. Check if file already exists in download directory
   - If exists: Prompt user (Skip/Overwrite/Abort) unless `--yes` or `--skip-existing` flag
2. Make GET request to mirror API
3. Handle response:
   - **Success (200)**: Save file with original filename
   - **Not Found (404)**: Show error, skip, continue
   - **Rate Limited (429)**: Show error, skip, continue
   - **Other errors**: Show error, skip, continue
4. Update progress bar

### 5. Progress Display
Use `indicatif` to show:
```
Downloading: [████████░░░░░░░░░░] 5/50 (10%)
Current: 123456 - Artist - Song Title.osz
```

### 6. Error Handling
**Fatal Errors** (stop execution):
- Invalid collection URL
- Cannot create download directory
- Network completely unavailable
- Invalid API response format

**Non-Fatal Errors** (show warning, continue):
- Individual beatmap 404
- Individual beatmap download failure
- Rate limiting on individual beatmap

### 7. Configuration File (Optional Enhancement)
Location: `~/.config/osu-collect/config.toml`

```toml
[mirror]
# Default mirror URL template
# Use {id} as placeholder for beatmapset ID
url = "https://api.nerinyan.moe/d/{id}"

[download]
# Auto-skip existing files without prompting
skip_existing = false

# Concurrent downloads (future enhancement)
concurrent = 1
```

## Implementation Steps

### Phase 1: Basic Structure
1. Set up Cargo project
2. Add dependencies to Cargo.toml
3. Create module structure
4. Implement CLI argument parsing with clap

### Phase 2: API Client
1. Implement collection fetching from osucollector.com
2. Parse JSON response and extract beatmapset IDs
3. Add error handling for API failures

### Phase 3: Downloader
1. Implement single beatmap download from mirror
2. Extract filename from Content-Disposition header
3. Save file to specified directory
4. Handle existing file checks and user prompts

### Phase 4: Progress & UX
1. Add indicatif progress bar
2. Implement download loop with progress updates
3. Add colored console output for errors/success
4. Display summary at end (downloaded/failed/skipped)

### Phase 5: Testing & Polish
1. Test with various collection sizes
2. Test error scenarios (404s, rate limits)
3. Test file existence handling
4. Add documentation and examples

## Example Usage Scenarios

### Basic download
```bash
osu-collect -c "https://osucollector.com/collections/17503" -d ~/osu/Songs
```

### Using alternative mirror
```bash
osu-collect -c "https://osucollector.com/collections/17503" \
  -d ~/osu/Songs \
  -m "https://catboy.best/d/{id}"
```

### Auto-skip existing files
```bash
osu-collect -c "https://osucollector.com/collections/17503" \
  -d ~/osu/Songs \
  --skip-existing
```

### Auto-overwrite existing files
```bash
osu-collect -c "https://osucollector.com/collections/17503" \
  -d ~/osu/Songs \
  -y
```

### Import directly into osu!lazer (auto-detect directory)
```bash
osu-collect -c "https://osucollector.com/collections/17503" --lazer
```

### Import into osu!lazer with custom directory and collection name
```bash
osu-collect -c "https://osucollector.com/collections/17503" \
  --lazer \
  --lazer-dir ~/.local/share/osu \
  --collection-name "My Aim Maps"
```

## Expected Output

### Successful Run
```
osu!collector Downloader v1.0.0
================================

Fetching collection: 17503
Collection: "aim slop"
Uploader: username
Total beatmaps: 50

Downloading to: /home/uwuclxdy/Downloads

Downloading: [████████████████████] 50/50 (100%)
Current: 987654 - Camellia - GHOST.osz

Summary:
✓ Downloaded: 48
⚠ Skipped (existing): 2
✗ Failed: 0

Done! All beatmaps downloaded successfully.
```

### With Errors
```
osu!collector Downloader v1.0.0
================================

Fetching collection: 17503
Collection: "aim slop"
Uploader: username
Total beatmaps: 50

Downloading to: /home/uwuclxdy/Downloads

Downloading: [███████████████░░░░░] 45/50 (90%)
✗ Error downloading 123456: 404 Not Found
✗ Error downloading 789012: Connection timeout
Current: 987654 - Camellia - GHOST.osz

Summary:
✓ Downloaded: 45
⚠ Skipped (existing): 3
✗ Failed: 2
  - 123456 (Not Found)
  - 789012 (Timeout)

Completed with errors.
```

## Security Considerations
1. Validate URLs to prevent injection attacks
2. Sanitize filenames from Content-Disposition header
3. Limit file size to prevent DoS (e.g., max 100MB per beatmap)
4. Use HTTPS for all API calls
5. Don't follow redirects blindly (limit redirect count)

## Future Enhancements
1. Concurrent downloads (parallel downloads with rate limiting)
2. Resume interrupted downloads
3. Verify downloaded file integrity (check if .osz is valid ZIP)
4. Cache collection data locally
5. Support for downloading specific beatmaps from a collection
6. Support for multiple collections in one command
7. Export collection to .db or .osdb format
8. Integration with osu! game directory auto-detection

## Performance Targets
- Parse collection: < 1 second
- Download single beatmap: 2-10 seconds (depends on size and network)
- Memory usage: < 50MB during operation
- Handle collections with 1000+ beatmaps without issues

---

# osu!lazer Integration (--lazer flag)

## Overview
When the `--lazer` flag is used, the program will:
1. Download beatmaps as `.osz` files (temporarily)
2. Import them directly into osu!lazer's Realm database
3. Create a collection with all the imported beatmaps
4. Clean up temporary files

## osu!lazer Storage Structure

### Default Locations
- **Linux**: `~/.local/share/osu`
- **Windows**: `%appdata%/osu`
- **macOS**: `~/Library/Application Support/osu`

### Directory Structure
```
~/.local/share/osu/
├── client.realm          # Main Realm database
├── client.realm.lock     # Database lock file
├── files/                # All beatmap files (SHA-256 hashed filenames)
│   ├── 0/
│   │   ├── 0a/
│   │   │   └── 0a1b2c3d4e...  # Hashed file
│   │   └── 0b/
│   └── ...
└── client.realm.management/  # Database backups and metadata
```

## How osu!lazer Import Works

### File Storage System
osu!lazer stores files using SHA-256 hashes as filenames, with mappings stored in a Realm database. This prevents duplicate files and tampering.

### Import Process
The standard way to import beatmaps into osu!lazer is:
1. Users can import .osz files through the in-game "Import files" button or by opening/sharing .osz files with osu!lazer
2. The game extracts the .osz (which is a ZIP file)
3. Each file is hashed with SHA-256
4. Files are stored in `files/` directory with hash-based names
5. Metadata is added to the Realm database

### Programmatic Import Approach
Since there's no official API for programmatic import, we'll use a **hybrid approach**:

**Option 1: Direct File System Integration (Recommended)**
1. Download .osz files
2. Extract each .osz (ZIP file)
3. Calculate SHA-256 hash for each file
4. Copy files to `files/` directory with proper hash-based naming
5. ~~Add entries to Realm database~~

**IMPORTANT LIMITATION**: The realm-cpp library for C++ is not mature enough for reliable use, and there's no official Rust Realm SDK that supports the full osu!lazer schema. The `realm_db_reader` crate only supports **read-only** access.

**Option 2: OS-Level File Association (Fallback - Not Recommended)**
- Use the system to "open" each .osz file with osu!lazer
- Requires osu!lazer to be running
- Less reliable and platform-dependent

### Recommended Implementation Strategy

Given the Realm database limitations, the **best approach** is:

1. **Download beatmaps** to a temporary directory as `.osz` files
2. **Move .osz files** to a staging directory that the user can easily import
3. **Provide instructions** for the user to:
   - Open osu!lazer
   - Use Settings → Maintenance → "Import files"
   - Select the staging directory

OR use a **simpler fallback**:

1. Download .osz files to a user-specified directory (or temp)
2. Create a bash/shell script that opens each .osz file with osu!lazer
3. On Linux: Use `xdg-open` to open .osz files (if lazer is set as default handler)
4. Print instructions for manual import if auto-import fails

### Collection Creation

Collections in osu!lazer are stored in the Realm database. Since we cannot write to the Realm database directly:

**Workaround Options**:
1. **Manual Collection** (Recommended): After import, provide instructions for user to manually create collection in-game
2. **collection.db Export**: Create a `collection.db` file (osu!stable format) and instruct user to import it
   - The collection.db format is still supported in lazer for importing from stable
   - We'd need to implement the osu!stable collection.db binary format
3. **Wait for official API**: Monitor for official osu!lazer import APIs

## Implementation Plan for --lazer Flag

### Phase 1: Basic File Download and Staging
1. Detect osu!lazer installation directory
   - Check standard paths for each OS
   - Allow override with `--lazer-dir`
2. Create temporary/staging directory for downloads
3. Download all beatmaps as .osz files
4. Provide clear user instructions for import

### Phase 2: Smart Import Helper
1. Create a helper script/command that:
   - Attempts to open each .osz with the system default handler
   - Falls back to manual instructions if it fails
2. Display progress and success/failure for each import

### Phase 3: Collection Management (Future)
1. Research osu!stable collection.db format
2. Implement collection.db writer
3. Generate collection.db file with all imported beatmap hashes
4. Provide import instructions

### Phase 4: (Future) Full Realm Integration
- Wait for mature Rust Realm SDK
- Implement direct Realm database writing
- Fully automated import with collection creation

## Technical Details for Realm Database Reading

Even though we can't write to Realm, we can **read** it to:
- Check if beatmaps already exist in lazer
- Avoid re-downloading existing beatmaps
- Verify successful imports

```rust
use realm_db_reader::{Realm, Group};

fn check_beatmap_exists(lazer_dir: &Path, beatmapset_id: i64) -> Result<bool> {
    let realm_path = lazer_dir.join("client.realm");
    let realm = Realm::open(&realm_path)?;
    let group = realm.into_group()?;
    
    // Access beatmapset table
    let table = group.get_table_by_name("BeatmapSet")?;
    
    // Check if beatmapset with OnlineID exists
    // (Implementation depends on realm_db_reader API)
    
    Ok(false)
}
```

## collection.db Format (osu!stable)

The collection.db format is a binary format:
- Header: Version number (int32)
- Number of collections (int32)
- For each collection:
  - Collection name (ULEB128 length + UTF-8 string)
  - Number of beatmaps (int32)
  - For each beatmap:
    - MD5 hash of .osu file (ULEB128 length + string)

**Implementation Reference**: The .db format stores collection names and map hashes, requiring an osu!.db file to identify maps locally

To create a collection.db:
1. Download all .osz files
2. Extract and parse each .osu file
3. Calculate MD5 hash of each .osu file content
4. Write collection.db with all hashes
5. Instruct user to place it in lazer directory

## User Experience for --lazer Mode

```bash
$ osu-collect -c "URL" --lazer

osu!collector Downloader v1.0.0
================================

Fetching collection: 17503
Collection: "aim slop"
Total beatmaps: 50

Detected osu!lazer at: ~/.local/share/osu
Downloading beatmaps...

Downloading: [████████████████████] 50/50 (100%)

✓ Downloaded 50 beatmaps to: /tmp/osu-import-12345/

Next steps to import into osu!lazer:
1. Open osu!lazer
2. Go to Settings (Ctrl+O)
3. Scroll to Maintenance section
4. Click "Import files"
5. Navigate to: /tmp/osu-import-12345/
6. Select all .osz files and import

OR: Run this command to auto-open files:
  for f in /tmp/osu-import-12345/*.osz; do xdg-open "$f"; done

After importing, create a collection manually:
- Name: "aim slop"
- Add all newly imported beatmaps
```

## Alternative: Simplified Approach

For the initial implementation, use a **much simpler approach**:

```bash
$ osu-collect -c "URL" --lazer

# Simply downloads to temp directory
# Prints instructions for drag-and-drop into lazer
# Or provides a script that opens each file with xdg-open
```

This avoids complex Realm database integration while still providing lazer support.