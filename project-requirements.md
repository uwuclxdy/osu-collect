# osu!collector Downloader - Project Requirements

## Overview
A CLI tool to download osu! beatmap collections from osu!collector.

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
│   └── error.rs          # Custom error types
└── config.toml           # Default configuration file (optional)
```

## Technologies & Dependencies

### Required Crates
#### CLI argument parsing
clap

#### HTTP client for API requests and downloads
reqwest

#### JSON serialization/deserialization
serde
serde_json

#### Error handling
anyhow
thiserror

#### Progress bar
indicatif

#### URL parsing
url

#### Configuration file support (optional)
toml
dirs

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
- `-d, --directory <PATH>`: Download directory (required)
- `-m, --mirror <URL>`: Mirror base URL (optional, default: nerinyan.moe)
- `-y, --yes`: Skip confirmation for existing files (auto-overwrite)
- `--skip-existing`: Skip existing files without asking
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
