# Mirror Error Detection Fix

## Problem Summary

Two distinct issues were causing the updater to incorrectly handle beatmapset updates:

### Issue 1: Mirror Error Pages (Fixed Previously)
Mirrors return HTTP 200 OK with HTML/JSON error pages instead of actual `.osz` archives when beatmapsets are unavailable.

### Issue 2: Stale osucollector Checksums (Fixed Now)
osucollector API caches beatmapset data that becomes stale when beatmaps are updated or deleted on osu. This causes false positives where beatmapsets appear to need updating but are actually up-to-date locally.

## Root Cause Analysis

### Stale Checksum Flow

1. **osucollector cache**: Stores beatmapset with checksums [A, B, C]
2. **osu updates beatmap**: Difficulty B is deleted, now only [A, C] remain
3. **Comparison logic**:
   - osucollector says: checksums [A, B, C]
   - Local has: checksums [A, C]
   - Result: "checksum B missing" → marked as needing update
4. **Mirror check**: Nekoha returns 404 (doesn't have it cached)
5. **Fallback**: Other mirrors have it, but no checksums available
6. **Download**: Beatmapset downloaded unnecessarily

### Example: Beatmapset 1793445
```
osucollector (stale): 3 checksums
  - cda21d8d562d5fdbc0a9d7e1f761311c  ← DELETED on osu!
  - 448aac631663171e39721f7525067b40
  - f3c9e999447abb51ec291317aa5f47f8

catboy/osu API (current): 2 checksums
  - 448aac631663171e39721f7525067b40
  - f3c9e999447abb51ec291317aa5f47f8

Local DB: 2 checksums (matches current osu!)
  - 448aac631663171e39721f7525067b40
  - f3c9e999447abb51ec291317aa5f47f8
```

## Solution Implementation

### Fix: Catboy API Fallback for Checksums

**File:** `src/download/size_fetcher.rs`

Added Catboy API as a secondary checksum source when Nekoha is unavailable:

```rust
// Three-pass approach:
// 1. Nekoha API (primary - has full beatmapset info)
// 2. Catboy API (secondary - mirrors osu! API with current data)
// 3. ZIP magic verification (fallback - availability only)

async fn fetch_catboy_checksums(client: &Client, beatmapset_id: u32) -> Option<Vec<String>> {
    let url = format!("{}/s/{}", CATBOY_API_BASE, beatmapset_id);
    // ... fetch and parse checksums
}
```

### Checksum Verification Flow

```
┌─────────────────────┐
│ osucollector API    │
│ (may have stale     │
│  checksums)         │
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│ Compare with local  │
│ checksums           │
└─────────┬───────────┘
          │
          ▼ (if mismatch)
┌─────────────────────┐
│ 1. Try Nekoha API   │──→ Has current checksums? → Use them
└─────────┬───────────┘
          │ (404)
          ▼
┌─────────────────────┐
│ 2. Try Catboy API   │──→ Has current checksums? → Use them
└─────────┬───────────┘
          │ (error)
          ▼
┌─────────────────────┐
│ 3. ZIP availability │──→ Available? → Include with no checksums
└─────────────────────┘    (trusts original status)
```

## Technical Details

### API Endpoints Used

| API | Endpoint | Purpose |
|-----|----------|---------|
| Nekoha | `/api4/beatmapsetFull/{id}` | Primary checksum source |
| Catboy | `/api/v2/s/{id}` | Secondary checksum source (mirrors osu! API) |
| Mirrors | Various download URLs | Availability verification |

### Catboy Response Structure
```json
{
  "id": 1793445,
  "beatmaps": [
    {"checksum": "448aac631663171e39721f7525067b40", ...},
    {"checksum": "f3c9e999447abb51ec291317aa5f47f8", ...}
  ]
}
```

## Impact

After this fix:

1. **Stale checksum detection**: Beatmapsets with outdated osucollector data are validated against current osu! API data via Catboy
2. **Reduced false positives**: Beatmapsets that are already up-to-date locally won't be re-downloaded
3. **Better accuracy**: Mirror checksums take precedence over potentially stale osucollector data

## Related Files

- [src/download/size_fetcher.rs](../src/download/size_fetcher.rs) - Mirror checksum fetching with Catboy fallback
- [src/app/runtime.rs](../src/app/runtime.rs) - Update comparison logic
- [src/worker/io.rs](../src/worker/io.rs) - Archive validation
