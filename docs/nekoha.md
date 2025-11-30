qq# Mirror Server API Documentation

A REST API for accessing mirror.nekoha.moe, including beatmapsets, beatmaps, statistics, and downloads.

## Base URL

```
https://mirror.nekoha.moe
```

## Endpoints

### 1. Get Statistics

Returns overall statistics about the mirror database.

**Endpoint:** `GET /api4/stats`

**Response:**
```json
{
  "last_beatmapset_id": 4928644,
  "beatmapset_count": 155432,
  "beatmap_count": 1234567,
  "ranked_count": 120000,
  "approved_count": 5000,
  "loved_count": 25000,
  "graveyard_count": 3000,
  "pending_count": 2432,
  "total_size": 524288000000
}
```

**Fields:**
- `last_beatmapset_id` - Highest beatmapset ID in database
- `beatmapset_count` - Total number of beatmapsets
- `beatmap_count` - Total number of individual beatmaps (difficulties)
- `ranked_count` - Number of ranked beatmapsets
- `approved_count` - Number of approved beatmapsets
- `loved_count` - Number of loved beatmapsets
- `graveyard_count` - Number of graveyard beatmapsets
- `pending_count` - Number of pending beatmapsets
- `total_size` - Total size of downloaded beatmapsets in bytes

---

### 2. Get Beatmapset Metadata

Returns metadata for a specific beatmapset (without beatmap difficulties).

**Endpoint:** `GET /api4/beatmapset/:id`

**Parameters:**
- `id` (number, required) - Beatmapset ID

**Example:** `GET /api4/beatmapset/1234`

**Response:**
```json
{
  "id": 1234,
  "status": 1,
  "title": "Song Title",
  "title_unicode": "曲名",
  "artist": "Artist Name",
  "artist_unicode": "アーティスト",
  "creator": "mapper_username",
  "user_id": 5678,
  "source": "Anime/Game",
  "tags": "tag1 tag2 tag3",
  "beatmap_count": 5,
  "osu": 3,
  "taiko": 1,
  "fruits": 0,
  "mania": 1,
  "bpm": 180.0,
  "submitted": "2023-01-15T12:00:00Z",
  "updated": "2023-02-20T15:30:00Z",
  "ranked": "2023-03-01T10:00:00Z",
  "loved": null,
  "approved": null,
  "genre_id": 2,
  "language_id": 3,
  "missing_audio": false,
  "deleted": false,
  "downloaded": true,
  "file_size": 12345678
}
```

**Status Codes:**
- `1` = Ranked
- `2` = Approved
- `4` = Loved
- `0` = Pending
- `-2` = Graveyard

**Error Responses:**
- `400 Bad Request` - `{"error": "Invalid beatmapset ID. Must be a positive number."}`
- `404 Not Found` - `{"error": "Beatmapset not found"}`
- `500 Internal Server Error` - `{"error": "Failed to fetch beatmapset"}`

---

### 3. Get Beatmapset with Difficulties

Returns complete beatmapset data including all beatmap difficulties.

**Endpoint:** `GET /api4/beatmapsetFull/:id`

**Parameters:**
- `id` (number, required) - Beatmapset ID

**Example:** `GET /api4/beatmapsetFull/1234`

**Response:**
```json
{
  "id": 1234,
  "status": 1,
  "title": "Song Title",
  "artist": "Artist Name",
  "creator": "mapper_username",
  "bpm": 180.0,
  "downloaded": true,
  "file_size": 12345678,
  "...": "...other beatmapset fields...",
  "beatmaps": [
    {
      "id": 5001,
      "beatmapset_id": 1234,
      "mode": 0,
      "status": 1,
      "version": "Easy",
      "creator": "mapper_username",
      "user_id": 5678,
      "difficulty_rating": 2.5,
      "cs": 4.0,
      "ar": 8.0,
      "od": 7.0,
      "hp": 6.0,
      "count_circles": 150,
      "count_sliders": 80,
      "count_spinners": 2,
      "max_combo": 250,
      "bpm": 180.0,
      "total_length": 120,
      "hit_length": 110,
      "checksum": "abc123def456",
      "last_updated": "2023-02-20T15:30:00Z",
      "url": "https://osu.ppy.sh/beatmaps/5001",
      "playcount": 50000,
      "passcount": 25000,
      "is_scoreable": true
    },
    {
      "...": "...more difficulties..."
    }
  ]
}
```

**Game Modes:**
- `0` = osu!
- `1` = Taiko
- `2` = Catch the Beat (Fruits)
- `3` = osu!mania

**Beatmaps are sorted by difficulty_rating (easiest first)**

**Error Responses:**
- `400 Bad Request` - `{"error": "Invalid beatmapset ID. Must be a positive number."}`
- `404 Not Found` - `{"error": "Beatmapset not found"}`
- `500 Internal Server Error` - `{"error": "Failed to fetch full beatmapset"}`

---

### 4. Get Single Beatmap

Returns data for a specific beatmap difficulty.

**Endpoint:** `GET /api4/beatmap/:id`

**Parameters:**
- `id` (number, required) - Beatmap ID

**Example:** `GET /api4/beatmap/5001`

**Response:**
```json
{
  "id": 5001,
  "beatmapset_id": 1234,
  "mode": 0,
  "status": 1,
  "version": "Insane",
  "creator": "mapper_username",
  "user_id": 5678,
  "difficulty_rating": 5.27,
  "cs": 4.0,
  "ar": 9.3,
  "od": 8.5,
  "hp": 6.0,
  "count_circles": 450,
  "count_sliders": 200,
  "count_spinners": 3,
  "max_combo": 850,
  "bpm": 180.0,
  "total_length": 180,
  "hit_length": 165,
  "checksum": "abc123def456789",
  "last_updated": "2023-02-20T15:30:00Z",
  "url": "https://osu.ppy.sh/beatmaps/5001",
  "playcount": 250000,
  "passcount": 50000,
  "is_scoreable": true
}
```

**Difficulty Fields:**
- `difficulty_rating` - Star rating
- `cs` - Circle Size
- `ar` - Approach Rate
- `od` - Overall Difficulty
- `hp` - HP Drain Rate

**Error Responses:**
- `400 Bad Request` - `{"error": "Invalid beatmap ID. Must be a positive number."}`
- `404 Not Found` - `{"error": "Beatmap not found"}`
- `500 Internal Server Error` - `{"error": "Failed to fetch beatmap"}`

---

### 5. Download Beatmapset

Downloads the .osz file for a beatmapset.

**Endpoint:** `GET /api4/download/:id`

**Parameters:**
- `id` (number, required) - Beatmapset ID

**Example:** `GET /api4/download/1234`

**Response:**
- **Content-Type:** `application/x-osu-beatmap-archive`
- **Content-Disposition:** `attachment; filename="1234 Artist - Title.osz"`
- **Body:** Binary .osz file stream

**Error Responses:**
- `400 Bad Request` - `{"error": "Invalid beatmapset ID. Must be a positive number."}`
- `404 Not Found` - Multiple possible messages:
  - `{"error": "Beatmapset not found"}`
  - `{"error": "Beatmapset not downloaded yet"}`
  - `{"error": "Beatmapset file not found on disk"}`
  - `{"error": "Beatmapset .osz file not found"}`
- `410 Gone` - `{"error": "Beatmapset missing (not available for download)\nContact me to upload this mapset."}`
- `500 Internal Server Error` - `{"error": "Failed to download beatmapset"}` or `{"error": "Failed to stream beatmapset file"}`

**Notes:**
- Only beatmapsets marked as `downloaded: true` can be downloaded
- Beatmapsets with `missing_audio: true` return 410 Gone:
  - They need to be uploaded, contact me if u have the mapset.


## Usage Examples

```bash
# Get stats
curl http://localhost:3000/api4/stats

# Get beatmapset
curl http://localhost:3000/api4/beatmapset/1234

# Download beatmapset
curl -O -J http://localhost:3000/api4/download/1234
``
