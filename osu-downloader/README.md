# osu-downloader

A vibecoded Rust library for downloading osu! beatmapsets in bulk from multiple mirrors, with failover, rate-limit backoff, MD5 + ZIP validation, and a streaming event API. Build a `Downloader`, hand it beatmapset IDs and an output directory, then consume `Event`s off the session until it finishes.

```toml
[dependencies]
osu-downloader = "0.9"
```

## Features

- Concurrent downloads across as many mirrors as you configure, with automatic failover when one returns 404, 429, or transient errors
- Per-mirror rate-limit backoff with a shared penalty pool: a throttled mirror sits out and the rest keep working
- Round-robin initial mirror per map so concurrent downloads spread across mirrors instead of all starting on the first one, plus per-mirror request spacing (100 ms between requests to the same mirror; 1 s for the official osu! API)
- Real-time progress, status, and completion events over a `Stream`, plus a one-shot summary on `.wait()`
- Streaming downloads with MD5 hashing and ZIP/EOCD validation, written through a temp file and hard-linked into place
- Optional osucollector.com collection fetcher (`collection` feature); writing `collection.db` stays in the caller, so the library never touches osu! database files
- Optional Nekoha-backed size and availability probes (`size-fetch` feature)

## Quick start

```rust
use osu_downloader::{Downloader, Event, Mirror};
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let downloader = Downloader::builder()
        .mirror(Mirror::nerinyan())
        .mirror(Mirror::osu_direct())
        .concurrent_downloads(8)
        .network_retry_attempts(2)
        .build()?;

    let mut session = downloader.download_many([123456u32, 789012, 345678], "./downloads");
    let mut events = session.events().expect("first events() call");

    while let Some(event) = events.next().await {
        match event {
            Event::BeatmapsetCompleted { beatmapset_id, filename, .. } => {
                println!("ok  #{beatmapset_id}: {filename}");
            }
            Event::BeatmapsetFailed { beatmapset_id, error, .. } => {
                eprintln!("err #{beatmapset_id}: {error}");
            }
            _ => {}
        }
    }

    let summary = session.wait().await?;
    println!("{} of {} downloaded", summary.downloaded.len(), summary.total);
    Ok(())
}
```

`download_many` accepts anything iterable over `u32`. `session.cancel()` aborts running downloads at the next checkpoint. `.builtins()` on the builder adds every built-in mirror in one call. `.on_exists(OnExists::Overwrite)` controls what happens when a beatmapset's archive is already on disk (default: skip).

## Mirrors

- **Nerinyan**: https://api.nerinyan.moe
- **osu.direct**: https://osu.direct
- **Sayobot**: https://dl.sayobot.cn
- **Nekoha**: https://mirror.nekoha.moe
- **Beatconnect**: https://beatconnect.io (anonymous `/b/{id}/`)
- **osu!dl**: https://osudl.org (anonymous `/s/{id}`, `302` → presigned R2; ranked/approved/loved only)
- **Hinamizawa**: https://mirror.hinamizawa.ai (server-side cascade over the others)
- **osu! official**: `https://osu.ppy.sh/api/v2/beatmapsets/{id}/download`. **Needs auth**: attach a `lazer`-scope bearer token via `Mirror::osu_api().with_headers(..)`. `MirrorKind::requires_auth()` reports this.
- **Custom**: `Mirror::custom("https://your.mirror/d/{id}")?`

`Mirror::builtins()` returns every built-in as a `Vec` (including `OsuApi`, which needs a caller-supplied auth header; filter with `MirrorKind::requires_auth()` in anonymous contexts). `Mirror::builtin(MirrorKind::Sayobot)` constructs a single built-in by tag (returns `None` for `Custom`). `Mirror::nerinyan().no_video()` switches to the no-video template for mirrors that have one (no-op for custom mirrors and osu! official).

## Collections

```rust
use osu_downloader::{collection::CollectionClient, parse_collection_id, Downloader};

let id = parse_collection_id("https://osucollector.com/collections/12345")?;
let collection = CollectionClient::new().fetch_retrying(id, 3).await?;
let downloader = Downloader::builder().builtins().build()?;
let mut session = downloader.download_many(collection.beatmapset_ids(), "./downloads");
// consume events, then session.wait()
```

- `CollectionClient::fetch(id)` does a single request and surfaces errors verbatim.
- `CollectionClient::fetch_retrying(id, attempts)` adds the library's built-in retry policy (rate-limit-aware sleeps + exponential backoff for transient network errors).
- `parse_collection_id(input)` is at the crate root and handles ID-or-URL parsing with strict `osucollector.com` HTTPS validation.
- `Collection` exposes `beatmapset_ids()`, `beatmap_count()`, and `folder_name()`; the raw `Beatmapset` / `Beatmap` / `Uploader` data is public too.
- Writing `collection.db` is deliberately **not** part of this library; pair it with the [osu-db](https://crates.io/crates/osu-db) crate in your app.

## Events and summary

`Event::BeatmapsetFailed` covers every failure path, including transient/network failures that exhausted every mirror; those arrive with `Error::Network(_)` (or another `Error::is_transient()` variant) and `mirror: None`. `Summary::failed` is `Vec<(u32, Error)>`; there is no separate "network errors" bucket.

## Errors

Everything funnels into one [`Error`](https://docs.rs/osu-downloader/latest/osu_downloader/enum.Error.html) enum so callers can match exhaustively without juggling layered error types:

```rust
match err {
    osu_downloader::Error::NotFound => …,
    osu_downloader::Error::RateLimited { retry_after } => …,
    osu_downloader::Error::Network(msg) => …,
    osu_downloader::Error::Timeout => …,
    osu_downloader::Error::Validation(msg) => …,
    _ => …,
}
```

`Error::is_transient()` is the shortcut for "should I retry?".

## Validation

```rust
use osu_downloader::{validate_archive, validate_and_remove, ArchiveValidation};

match validate_archive(path, ArchiveValidation::Eocd).await? {
    ArchiveValidationResult::Valid => …,
    ArchiveValidationResult::NotFound => …,
    ArchiveValidationResult::Invalid(reason) => …,
}

// or, to delete the file on validation failure:
validate_and_remove(path, ArchiveValidation::Eocd).await?;
```

## Output directory scanning

When walking the directory the downloader writes into, `classify_output_entry(name)` tells you which entries belong to the library:

```rust
use osu_downloader::{classify_output_entry, OutputEntry};

match classify_output_entry(&entry.file_name()) {
    OutputEntry::Archive { beatmapset_id } => …,
    OutputEntry::OrphanTemp => /* leftover from a cancelled download — safe to delete */,
    OutputEntry::Other => /* foreign file */,
}
```

## Size lookups

The `size-fetch` feature asks the Nekoha mirror how large a beatmapset's archive is. Pick the call by whether you need to tell a real answer from a failure:

```rust
use osu_downloader::size::SizeFetcher;

let fetcher = SizeFetcher::new();

// Per-id: separates the two, so a caller can cache an answer and retry a failure.
match fetcher.fetch_size(1091132).await {
    Ok(Some(bytes)) => /* the mirror's size record */,
    Ok(None)        => /* the mirror has no size for this set — a stable answer */,
    Err(err)        => /* the mirror was unreachable; says nothing about the set */,
}

// Aggregate: folds "no record" and "probe failed" together into `missing_count`,
// then bills each unknown at the average of what landed.
let estimate = fetcher.fetch_sizes(&[1091132, 1234567]).await;
```

`fetch_size` performs a single request and never retries; retry on the caller side if you want one.

## Availability checks

The `size-fetch` feature exposes `SizeFetcher::check_availability` for cheap "is this id reachable on any mirror" probes. It accepts `Mirror` objects directly, so the typical call is:

```rust
use osu_downloader::{Mirror, size::SizeFetcher};

let fetcher = SizeFetcher::new();
// Drop auth-gated mirrors — availability is an anonymous probe.
let mirrors: Vec<_> = Mirror::builtins()
    .into_iter()
    .filter(|m| !m.kind().requires_auth())
    .collect();
let result = fetcher
    .check_availability(&[123, 456, 789], &mirrors, |checked, total| {
        println!("checked {checked}/{total}");
    })
    .await;

println!("available: {:?}, unavailable: {:?}", result.available, result.unavailable);
```

## Feature flags

- `collection` (default): `CollectionClient`, `Collection`, `Beatmapset`, `Beatmap`, `Uploader`
- `size-fetch` (default): `SizeFetcher` for beatmapset size estimates and mirror availability probes

## License

MIT. See [LICENSE](LICENSE).

## Acknowledgments

- Inspired by [BBD (beatmap batch downloader)](https://github.com/nzbasic/batch-beatmap-downloader)
- Written by Claude
