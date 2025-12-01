# Reading osu! stable Collections with Rust

## Overview

osu! stable uses custom binary database formats. The `osu-db` crate provides pure Rust parsing for:
- `osu!.db` - Main beatmap information cache
- `collection.db` - User's beatmap collections
- `scores.db` - Score summaries
- `.osr` - Individual replay files

## Database Locations

### Windows
```
%LOCALAPPDATA%\osu!\osu!.db
%LOCALAPPDATA%\osu!\collection.db
```

### Linux/macOS
```
~/.osu/osu!.db
~/.osu/collection.db
```

## Core API Reference

### Reading Collections

```rust
use osu_db::collection::CollectionList;

// Load collection.db
let collections = CollectionList::from_file("collection.db")?;

// Access data
collections.version          // u32: database version
collections.collections      // Vec<Collection>
```

### Collection Structure

```rust
pub struct Collection {
    pub name: Option<String>,               // Collection name
    pub beatmap_hashes: Vec<Option<String>>, // MD5 hashes of beatmaps
}
```

### Reading Beatmap Database

```rust
use osu_db::listing::Listing;

// Load osu!.db
let listing = Listing::from_file("osu!.db")?;

// Access data
listing.version        // u32: database version
listing.player_name    // String
listing.beatmaps       // Vec<Beatmap>
```

### Beatmap Structure (Key Fields)

```rust
pub struct Beatmap {
    pub artist_name: Option<String>,
    pub song_title: Option<String>,
    pub creator_name: Option<String>,
    pub difficulty: Option<String>,
    pub hash: Option<String>,              // MD5 hash
    pub mode: Mode,                        // Osu, Taiko, CatchTheBeat, OsuMania
    pub ranked_status: RankedStatus,       // Ranked, Approved, Loved, etc.
    pub std_star_rating: Option<Vec<(i32, f32)>>, // [(mods, star_rating)]
    // ... many more fields available
}
```

## Common Patterns

### List All Collections

```rust
let collections = CollectionList::from_file("collection.db")?;
for collection in &collections.collections {
    let name = collection.name.as_deref().unwrap_or("(unnamed)");
    let count = collection.beatmap_hashes.len();
    println!("{}: {} beatmaps", name, count);
}
```

### Find Beatmaps in Collection

```rust
let db = Listing::from_file("osu!.db")?;
let collections = CollectionList::from_file("collection.db")?;

let my_collection = collections.collections
    .iter()
    .find(|c| c.name.as_deref() == Some("My Collection"));

if let Some(collection) = my_collection {
    for hash_opt in &collection.beatmap_hashes {
        if let Some(hash) = hash_opt {
            // Find beatmap with this hash
            let beatmap = db.beatmaps.iter()
                .find(|b| b.hash.as_ref() == Some(hash));

            if let Some(beatmap) = beatmap {
                println!("{} - {}",
                    beatmap.artist_name.as_deref().unwrap_or(""),
                    beatmap.song_title.as_deref().unwrap_or(""));
            }
        }
    }
}
```

### Create/Modify Collections

```rust
use osu_db::collection::{CollectionList, Collection};

let mut collections = CollectionList::from_file("collection.db")?;

// Add new collection
collections.collections.push(Collection {
    name: Some("My New Collection".to_string()),
    beatmap_hashes: vec![Some("beatmap_md5_hash".to_string())],
});

// Save back
collections.save("collection.db")?;
```

### Filter Beatmaps

```rust
let db = Listing::from_file("osu!.db")?;

// By artist
let results: Vec<&Beatmap> = db.beatmaps.iter()
    .filter(|b| b.artist_name.as_deref().unwrap_or("").contains("Camellia"))
    .collect();

// By star rating (standard mode)
let high_star: Vec<&Beatmap> = db.beatmaps.iter()
    .filter(|b| {
        b.std_star_rating.as_ref()
            .and_then(|r| r.first())
            .map(|(_, sr)| *sr >= 7.0)
            .unwrap_or(false)
    })
    .collect();

// By mode
use osu_db::listing::Mode;
let mania_maps: Vec<&Beatmap> = db.beatmaps.iter()
    .filter(|b| b.mode == Mode::OsuMania)
    .collect();
```

## Important Notes

1. **Strings are Optional**: Most string fields are `Option<String>` because the format allows null values
2. **Version Compatibility**: The crate supports database versions up to at least 20211103
3. **Read-Only by Default**: Collections are only modified when explicitly saved
4. **Hash Matching**: Use MD5 hashes to match beatmaps between `osu!.db` and `collection.db`
5. **Thread Safety**: Database structures can be safely passed between threads

## Enums Reference

### Mode
```rust
pub enum Mode {
    Osu,           // Standard
    Taiko,
    CatchTheBeat,
    OsuMania,
}
```

### RankedStatus
```rust
pub enum RankedStatus {
    Unknown,
    Unsubmitted,
    Pending,
    Ranked,
    Approved,
    Qualified,
    Loved,
}
```

## Error Handling

```rust
use std::io;

match CollectionList::from_file("collection.db") {
    Ok(collections) => {
        // Process collections
    }
    Err(e) => {
        eprintln!("Failed to read collection.db: {}", e);
        // Handle error
    }
}
```

## Performance Tips

- Load databases once and reuse
- Use iterators instead of collecting when filtering
- Access `.unwrap_or_default()` is zero-cost for `Option<String>`
- Star ratings are stored per-mod combination (check first element for no-mod)
