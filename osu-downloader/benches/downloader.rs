use std::hint::black_box;
use std::sync::{LazyLock, mpsc};

use bytes::Bytes;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use md5::{Digest, Md5};
use osu_downloader::{Mirror, sanitize_filename};

fn bench_md5_hex_format(c: &mut Criterion) {
    // Represents the hot path in HashWorker::new (worker.rs:53-58):
    //   hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
    // This runs once per completed download to produce the MD5 hex digest.
    let data = b"some representative beatmap archive bytes for hashing purposes 1234567890";

    c.bench_function("md5_hex_format", |b| {
        b.iter(|| {
            let mut hasher = Md5::new();
            hasher.update(black_box(data));
            let digest = hasher.finalize();
            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            black_box(hex)
        })
    });
}

fn bench_mirror_url_for(c: &mut Criterion) {
    // Represents mirror.url_for(beatmapset_id) (mirrors/mod.rs:216-218):
    //   self.template.replace("{id}", &beatmapset_id.to_string())
    // Called in the inner retry loop for every mirror attempt.
    let mirrors = [
        Mirror::nerinyan(),
        Mirror::osu_direct(),
        Mirror::sayobot(),
        Mirror::nekoha(),
    ];
    let ids: &[u32] = &[1, 123, 99999, 1_234_567, u32::MAX];

    let mut group = c.benchmark_group("mirror_url_for");
    for &id in ids {
        group.bench_with_input(BenchmarkId::new("nerinyan", id), &id, |b, &id| {
            b.iter(|| black_box(mirrors[0].template().replace("{id}", &id.to_string())))
        });
    }
    group.finish();
}

fn bench_sanitize_filename(c: &mut Criterion) {
    // Represents sanitize_filename (download.rs:86-92):
    //   name.chars().map(|c| match c { forbidden => '_', c => c }).collect::<String>()
    // Called once per download to clean the Content-Disposition filename.
    let cases: &[(&str, u32)] = &[
        // typical clean filename — no replacements needed
        ("1234567 Artist - Song Title [Difficulty].osz", 1_234_567),
        // filename with many forbidden chars (worst case replacement path)
        ("1234567 Art:ist - Song/Title [Diff*cult\\y].osz", 1_234_567),
        // short fallback-triggering None input
        ("", 999),
        // long filename (~200 chars)
        (
            "9999999 A Very Long Artist Name With Spaces - A Very Long Song Title \
             That Goes On And On Including Extra Details [Expert Difficulty].osz",
            9_999_999,
        ),
    ];

    let mut group = c.benchmark_group("sanitize_filename");
    for (name, id) in cases {
        let label = if name.is_empty() {
            "empty"
        } else if name.contains(':') || name.contains('/') || name.contains('*') {
            "with_forbidden"
        } else if name.len() > 100 {
            "long_clean"
        } else {
            "typical_clean"
        };
        group.bench_with_input(
            BenchmarkId::new(label, id),
            &(*name, *id),
            |b, &(name, id)| {
                b.iter(|| black_box(sanitize_filename(Some(black_box(name)), black_box(id))))
            },
        );
    }
    group.finish();
}

fn bench_hash_worker_update(c: &mut Criterion) {
    // Represents HashWorker::update (worker.rs):
    //   fn update(&self, data: Bytes) { sender.send(data); }
    // Called per ~128 KB network chunk during streaming download. Previously used
    // to_vec() (128 KB heap alloc per chunk); now sends a Bytes clone (Arc refcount
    // bump, ~2 ns) — the reqwest bytes_stream() already returns Bytes.
    const CHUNK_128K: usize = 128 * 1024;
    const CHUNK_4K: usize = 4 * 1024;

    let chunk_128k = Bytes::from(vec![0xABu8; CHUNK_128K]);
    let chunk_4k = Bytes::from(vec![0xABu8; CHUNK_4K]);

    let cases: &[(&str, &Bytes)] = &[("128k_chunk", &chunk_128k), ("4k_chunk", &chunk_4k)];

    let mut group = c.benchmark_group("hash_worker_update");
    for (label, chunk) in cases {
        group.throughput(Throughput::Bytes(chunk.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), chunk, |b, chunk| {
            // Replicate the exact production shape: unbounded channel, send a Bytes clone.
            let (sender, receiver) = mpsc::channel::<Bytes>();
            // Drain the receiver in a background thread so the channel never blocks.
            std::thread::spawn(move || while receiver.recv().is_ok() {});
            b.iter(|| {
                let _ = sender.send(black_box((*chunk).clone()));
            });
        });
    }
    group.finish();
}

fn bench_find_eocd_position(c: &mut Criterion) {
    // Represents find_eocd_position (validation.rs:176-180):
    //   buffer.windows(EOCD_SIGNATURE.len()).rposition(|w| w == EOCD_SIGNATURE)
    // Called once per archive during precheck (up to 65 536 bytes) to locate the
    // ZIP end-of-central-directory record. Hot when hundreds of archives are
    // validated in parallel.
    const EOCD_SIG: &[u8] = &[0x50, 0x4B, 0x05, 0x06];
    const BUF_SIZE: usize = 65_536;

    // Case 1: EOCD present at the very end (common happy path — minimal real archive).
    let mut buf_eocd_at_end = vec![0u8; BUF_SIZE];
    buf_eocd_at_end[BUF_SIZE - 22..BUF_SIZE - 22 + 4].copy_from_slice(EOCD_SIG);

    // Case 2: EOCD absent — full scan with no match (worst case for corrupted archives).
    let buf_no_eocd = vec![0u8; BUF_SIZE];

    // Case 3: EOCD near the middle (must find the last occurrence).
    let mut buf_eocd_mid = vec![0u8; BUF_SIZE];
    buf_eocd_mid[BUF_SIZE / 2..BUF_SIZE / 2 + 4].copy_from_slice(EOCD_SIG);

    // Inline the exact production pattern so the bench measures the real code shape.
    let find_eocd = |buffer: &[u8]| -> Option<usize> {
        if buffer.len() < EOCD_SIG.len() {
            return None;
        }
        let end = buffer.len() - EOCD_SIG.len();
        memchr::memrchr_iter(0x50, &buffer[..=end])
            .find(|&pos| buffer[pos..pos + EOCD_SIG.len()] == *EOCD_SIG)
    };

    let mut group = c.benchmark_group("find_eocd_position");
    group.throughput(Throughput::Bytes(BUF_SIZE as u64));

    group.bench_function("eocd_at_end", |b| {
        b.iter(|| black_box(find_eocd(black_box(&buf_eocd_at_end))))
    });
    group.bench_function("no_eocd", |b| {
        b.iter(|| black_box(find_eocd(black_box(&buf_no_eocd))))
    });
    group.bench_function("eocd_at_mid", |b| {
        b.iter(|| black_box(find_eocd(black_box(&buf_eocd_mid))))
    });

    group.finish();
}

// Inline of the exact production iterator (download.rs) — private, not pub.
struct ContentDispositionParts<'a> {
    rest: &'a str,
    done: bool,
}

fn content_disposition_parts(header_value: &str) -> ContentDispositionParts<'_> {
    ContentDispositionParts {
        rest: header_value,
        done: false,
    }
}

impl<'a> Iterator for ContentDispositionParts<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if self.done {
            return None;
        }
        let mut quoted = false;
        let mut escaped = false;
        for (index, ch) in self.rest.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' if quoted => escaped = true,
                '"' => quoted = !quoted,
                ';' if !quoted => {
                    let part = self.rest[..index].trim();
                    self.rest = &self.rest[index + 1..];
                    return Some(part);
                }
                _ => {}
            }
        }
        self.done = true;
        Some(self.rest.trim())
    }
}

fn bench_split_content_disposition(c: &mut Criterion) {
    // Represents content_disposition_parts (download.rs):
    //   allocation-free iterator over ';'-separated header segments.
    // Called inside parse_content_disposition on every successful mirror
    // response. Two real-world header shapes benched.

    // Common case: simple quoted filename.
    let simple = r#"attachment; filename="1234567 Artist - Song [Hard].osz""#;
    // Extended RFC 5987 UTF-8 encoded filename (less common but realistic).
    let extended = r#"attachment; filename="fallback.osz"; filename*=UTF-8''1234567%20Artist%20-%20Song%20%5BHard%5D.osz"#;

    let mut group = c.benchmark_group("split_content_disposition");
    group.bench_function("simple_filename", |b| {
        b.iter(|| black_box(content_disposition_parts(black_box(simple)).count()))
    });
    group.bench_function("extended_filename_star", |b| {
        b.iter(|| black_box(content_disposition_parts(black_box(extended)).count()))
    });
    group.finish();
}

// ── process_mirror_response content-type check ────────────────────────────────
//
// osu-downloader/src/download.rs:process_mirror_response — called on every
// successful mirror HTTP response to validate the Content-Type header.
// Current shape:
//   value.to_ascii_lowercase()  — allocates a new String for every response
//   is_archive_content_type(&lowercased)
// Hot because it runs once per download attempt, per mirror retry.
//
// Candidate: pass the raw header value to a version of is_archive_content_type
// that uses eq_ignore_ascii_case — zero allocation.
//
// Bench inputs: real-world Content-Type values from known mirrors (with and
// without the "; charset=…" parameter suffix that forces a split).

fn is_archive_content_type_baseline(raw: &str) -> bool {
    let mime = raw.split(';').next().map(str::trim).unwrap_or("");
    matches!(
        mime,
        "application/x-osu-beatmap-archive"
            | "application/octet-stream"
            | "binary/octet-stream"
            | "application/zip"
            | "application/x-zip-compressed"
    )
}

fn is_archive_content_type_no_alloc(raw: &str) -> bool {
    let mime = raw.split(';').next().map(str::trim).unwrap_or("");
    [
        "application/x-osu-beatmap-archive",
        "application/octet-stream",
        "binary/octet-stream",
        "application/zip",
        "application/x-zip-compressed",
    ]
    .iter()
    .any(|&known| mime.eq_ignore_ascii_case(known))
}

fn bench_content_type_check(c: &mut Criterion) {
    // Raw Content-Type header values as returned by mirrors — mixed case is
    // realistic; Nerinyan sends lowercase, osu!direct sends mixed.
    let cases: &[(&str, &str)] = &[
        ("application/zip", "lowercase_zip"),
        ("Application/Zip", "mixed_case_zip"),
        ("application/octet-stream", "octet_stream"),
        ("application/x-osu-beatmap-archive", "osu_archive"),
        ("application/zip; charset=utf-8", "zip_with_param"),
        ("text/html; charset=utf-8", "wrong_type"),
    ];

    let mut group = c.benchmark_group("content_type_check");

    for &(header, label) in cases {
        // Baseline: to_ascii_lowercase() allocates on every call.
        group.bench_with_input(
            BenchmarkId::new("to_ascii_lowercase", label),
            header,
            |b, header| {
                b.iter(|| {
                    let lowered = black_box(header).to_ascii_lowercase();
                    black_box(is_archive_content_type_baseline(&lowered))
                })
            },
        );

        // Candidate: eq_ignore_ascii_case — zero allocation.
        group.bench_with_input(
            BenchmarkId::new("eq_ignore_ascii_case", label),
            header,
            |b, header| b.iter(|| black_box(is_archive_content_type_no_alloc(black_box(header)))),
        );
    }

    group.finish();
}

// ── collection_hashes clone without capacity ──────────────────────────────────
//
// osu-downloader/src/collection.rs:collection_hashes — called once per
// collection write to build a Vec<String> of all beatmap MD5 checksums.
// Current shape:
//   flat_map(…).map(|beatmap| beatmap.checksum.clone()).collect()
// No with_capacity hint: Vec grows via doubling from 0.  For a 500-beatmapset
// collection averaging 5 maps each, that is ~2500 32-byte String clones with
// log2(2500) ≈ 12 reallocs.
//
// Candidate: pre-size via beatmap_count() (already a O(n) sum, paid once) and
// extend instead of collect.
//
// Bench inputs: (beatmapsets, beatmaps_per_set) = (50,5), (200,5), (500,5).

fn make_checksums(beatmapsets: usize, per_set: usize) -> Vec<Vec<String>> {
    (0..beatmapsets)
        .map(|s| {
            (0..per_set)
                .map(|b| format!("{:032x}", s * per_set + b))
                .collect()
        })
        .collect()
}

fn bench_collection_hashes(c: &mut Criterion) {
    let configs: &[(&str, usize, usize)] = &[("50x5", 50, 5), ("200x5", 200, 5), ("500x5", 500, 5)];

    let mut group = c.benchmark_group("collection_hashes");

    for &(label, sets, per_set) in configs {
        let beatmapsets: Vec<Vec<String>> = make_checksums(sets, per_set);

        // Baseline: flat_map + clone + collect — no capacity hint.
        group.bench_with_input(
            BenchmarkId::new("flat_map_collect", label),
            &beatmapsets,
            |b, beatmapsets| {
                b.iter(|| {
                    let hashes: Vec<String> = black_box(beatmapsets)
                        .iter()
                        .flat_map(|set| set.iter().cloned())
                        .collect();
                    black_box(hashes)
                })
            },
        );

        // Candidate: with_capacity pre-sized, extend.
        group.bench_with_input(
            BenchmarkId::new("with_capacity_extend", label),
            &beatmapsets,
            |b, beatmapsets| {
                b.iter(|| {
                    let total: usize = black_box(beatmapsets).iter().map(|s| s.len()).sum();
                    let mut hashes = Vec::with_capacity(total);
                    for set in black_box(beatmapsets) {
                        hashes.extend(set.iter().cloned());
                    }
                    black_box(hashes)
                })
            },
        );
    }

    group.finish();
}

// ── pending_mirrors_clone ─────────────────────────────────────────────────────
//
// osu-downloader/src/download.rs:download_beatmapset — at the start of each
// per-beatmapset download, the current code builds a working list of mirrors:
//   let mut pending = params.mirror_pool.mirrors().to_vec();
// For the 4-mirror default pool each `Mirror` clone allocates 3 `Box<str>`
// (template + SplitTemplate prefix/suffix) plus an `Option<HeaderMap>`.
// With 4 mirrors: 4 heap allocs for the Vec + 12 Box<str> allocs = 16 allocs
// per beatmapset at the outer loop entry.
//
// Candidate: carry a Vec<usize> of pending mirror indices instead of cloning
// the Mirror values.  Indices are Copy — zero heap alloc for the pending list.
//
// Bench inputs: pool sizes of 2 (minimal), 4 (default), 6 (hypothetical
// max with custom mirror) to show per-mirror scaling.

fn bench_pending_mirrors_clone(c: &mut Criterion) {
    use osu_downloader::Mirror;

    let all_mirrors: Vec<Mirror> = vec![
        Mirror::nerinyan(),
        Mirror::osu_direct(),
        Mirror::sayobot(),
        Mirror::nekoha(),
        Mirror::nerinyan(), // extra to simulate 5-mirror case
        Mirror::osu_direct(),
    ];

    let sizes: &[usize] = &[2, 4, 6];
    let mut group = c.benchmark_group("pending_mirrors_clone");

    for &n in sizes {
        let mirrors = &all_mirrors[..n];

        // Baseline: current production shape — .to_vec() clones every Mirror.
        group.bench_with_input(BenchmarkId::new("to_vec_mirror", n), &n, |b, _| {
            b.iter(|| {
                let pending: Vec<Mirror> = black_box(mirrors).to_vec();
                black_box(pending)
            })
        });

        // Candidate: Vec<usize> of pending indices — Copy, no per-Mirror alloc.
        group.bench_with_input(BenchmarkId::new("index_vec", n), &n, |b, &n| {
            b.iter(|| {
                let pending: Vec<usize> = (0..black_box(n)).collect();
                black_box(pending)
            })
        });
    }

    group.finish();
}

// ── write_collections_db dedup ────────────────────────────────────────────────
//
// osu-downloader/src/collection.rs:write_collections_db — called once per
// collection write (end of a successful download run) to build the osu!
// collection.db file.  Current dedup pattern:
//   .filter(|hash| seen.insert((*hash).clone()))   ← clone into HashSet<String>
//   .cloned()                                       ← clone into output Vec
// Every hash that passes the filter is cloned TWICE: once into the seen-set
// and once into the Vec<Option<String>> output.  For a 500-beatmapset
// collection averaging 5 maps each that is 5000 String clones instead of 2500.
//
// Candidate: HashSet<&str> for dedup — the hash string is borrowed by the seen
// set, so only the final .cloned() into the output Vec allocates.  Saves one
// clone (one String heap alloc) per unique hash.
//
// Bench inputs: (beatmapsets × beatmaps_per_set) matching realistic collection
// sizes: 50×5 (250 hashes), 200×5 (1000 hashes), 500×5 (2500 hashes).

fn bench_write_collections_db_dedup(c: &mut Criterion) {
    let configs: &[(&str, usize, usize)] = &[("50x5", 50, 5), ("200x5", 200, 5), ("500x5", 500, 5)];

    let mut group = c.benchmark_group("write_collections_db_dedup");

    for &(label, sets, per_set) in configs {
        // Flat list of MD5-like 32-char hex strings (no duplicates — worst case
        // for the HashSet: every entry must be inserted and cloned).
        let hashes: Vec<String> = make_checksums(sets, per_set)
            .into_iter()
            .flatten()
            .collect();

        // Baseline: double-clone — HashSet<String> insert + .cloned() output.
        group.bench_with_input(
            BenchmarkId::new("double_clone_hashset_string", label),
            &hashes,
            |b, hashes| {
                b.iter(|| {
                    let mut seen = std::collections::HashSet::<String>::new();
                    let out: Vec<Option<String>> = black_box(hashes)
                        .iter()
                        .filter(|hash| seen.insert((*hash).clone()))
                        .cloned()
                        .map(Some)
                        .collect();
                    black_box(out)
                })
            },
        );

        // Candidate: single-clone — HashSet<&str> borrows for dedup, .cloned()
        // once into the output Vec.
        group.bench_with_input(
            BenchmarkId::new("single_clone_hashset_str", label),
            &hashes,
            |b, hashes| {
                b.iter(|| {
                    let mut seen = std::collections::HashSet::<&str>::new();
                    let out: Vec<Option<String>> = black_box(hashes)
                        .iter()
                        .filter(|hash| seen.insert(hash.as_str()))
                        .cloned()
                        .map(Some)
                        .collect();
                    black_box(out)
                })
            },
        );
    }

    group.finish();
}

// ── temp_path_for pid format ──────────────────────────────────────────────────
//
// osu-downloader/src/worker.rs:248-258 — called once per download attempt to
// build the temporary file path used during streaming:
//   format!("{name}.download-{}-{counter}.tmp", std::process::id())
// This calls std::process::id() (a getpid syscall, platform-dependent caching)
// and allocates a new String on every call.
//
// Candidate: compute the pid string exactly once with LazyLock<String> and use
// with_capacity + push_str to assemble the path without a format! allocation.
//
// Bench inputs: typical short filename (common) and long filename (stress).

static PID_STR: LazyLock<String> = LazyLock::new(|| std::process::id().to_string());

fn temp_path_for_baseline(name: &str, counter: u64) -> String {
    // Exact production shape: format! with std::process::id() each call.
    format!("{name}.download-{}-{counter}.tmp", std::process::id())
}

fn temp_path_for_cached_pid(name: &str, counter: u64) -> String {
    // Candidate: pid fetched once from LazyLock; manual with_capacity + push_str.
    let pid = &*PID_STR;
    // Exact capacity: name + ".download-" + pid + "-" + up to 20 digits + ".tmp"
    let counter_digits = if counter == 0 {
        1
    } else {
        counter.checked_ilog10().unwrap_or(0) as usize + 1
    };
    let cap = name.len() + ".download-".len() + pid.len() + 1 + counter_digits + ".tmp".len();
    let mut s = String::with_capacity(cap);
    s.push_str(name);
    s.push_str(".download-");
    s.push_str(pid);
    s.push('-');
    // Write counter digits without allocating.
    let mut buf = [0u8; 20];
    let mut n = counter;
    let mut pos = 20usize;
    if n == 0 {
        pos -= 1;
        buf[pos] = b'0';
    } else {
        while n > 0 {
            pos -= 1;
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }
    // SAFETY: buf[pos..] contains only ASCII digit bytes, valid UTF-8.
    s.push_str(unsafe { std::str::from_utf8_unchecked(&buf[pos..]) });
    s.push_str(".tmp");
    s
}

fn bench_temp_path_for_pid(c: &mut Criterion) {
    // name is the file_name() component from the output_path (typical .osz filename).
    let cases: &[(&str, u64, &str)] = &[
        ("1234567 Artist - Song Title [Hard].osz", 0, "typical_short"),
        (
            "9999999 A Very Long Artist Name With Spaces - A Very Long Song Title \
             That Goes On And On Including Extra Details [Expert Difficulty].osz",
            999,
            "long_name",
        ),
        // Fallback name when output path has no file_name component.
        ("download", 42, "fallback_name"),
    ];

    let mut group = c.benchmark_group("temp_path_for_pid");

    for &(name, counter, label) in cases {
        group.bench_with_input(
            BenchmarkId::new("format_process_id", label),
            &(name, counter),
            |b, &(name, counter)| {
                b.iter(|| black_box(temp_path_for_baseline(black_box(name), black_box(counter))))
            },
        );

        group.bench_with_input(
            BenchmarkId::new("lazy_lock_pid", label),
            &(name, counter),
            |b, &(name, counter)| {
                b.iter(|| {
                    black_box(temp_path_for_cached_pid(
                        black_box(name),
                        black_box(counter),
                    ))
                })
            },
        );
    }

    group.finish();
}

// ── beatmapset_ids HashSet capacity ──────────────────────────────────────────
//
// osu-downloader/src/collection.rs:Collection::beatmapset_ids (line 62-69) —
// called once per session to derive the download list from the collection.
// Current shape:
//   let mut seen = std::collections::HashSet::new();
// For a 500-beatmapset collection this starts at capacity 0 and reallocs
// ~9 times as the set grows (doubling from 0 → 1 → 2 → 4 → 8 → …).
//
// Candidate: HashSet::with_capacity(self.beatmapsets.len()) — pre-sizes to
// fit all IDs in a single allocation, zero reallocs.
//
// Bench inputs: 50, 200, 500 beatmapsets (all unique IDs — worst case for the
// HashSet: every insert grows the set).

fn beatmapset_ids_no_capacity(ids: &[u32]) -> Vec<u32> {
    let mut seen = std::collections::HashSet::new();
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

fn beatmapset_ids_with_capacity(ids: &[u32]) -> Vec<u32> {
    let mut seen = std::collections::HashSet::with_capacity(ids.len());
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

fn bench_beatmapset_ids_capacity(c: &mut Criterion) {
    let configs: &[(&str, usize)] = &[("50", 50), ("200", 200), ("500", 500)];

    let mut group = c.benchmark_group("beatmapset_ids_capacity");

    for &(label, n) in configs {
        // All-unique IDs so every insert hits the growth path.
        let ids: Vec<u32> = (0..n as u32).collect();

        group.bench_with_input(BenchmarkId::new("hashset_new", label), &ids, |b, ids| {
            b.iter(|| black_box(beatmapset_ids_no_capacity(black_box(ids))))
        });

        group.bench_with_input(
            BenchmarkId::new("hashset_with_capacity", label),
            &ids,
            |b, ids| b.iter(|| black_box(beatmapset_ids_with_capacity(black_box(ids)))),
        );
    }

    group.finish();
}

// ── extract_filename osz append ───────────────────────────────────────────────
//
// osu-downloader/src/download.rs:extract_filename (lines 747-762) — called once
// per successful mirror response to derive the final archive filename.
// Current shape when the parsed filename lacks an `.osz` extension:
//   format!("{filename}.osz")
// This allocates a second String even though `filename` is already owned and has
// sufficient capacity for an in-place append.
//
// Candidate: filename.push_str(".osz") — reuses the existing allocation when the
// String's capacity allows it; only reallocates if the original string was
// shrink-fitted (which String::from/to_string never does).
//
// The common path (filename already ends with `.osz`) is unchanged.
// This bench isolates the append branch: both baseline and candidate receive
// filenames without an extension so the append is always taken.
//
// Bench inputs: short, typical, and long filenames (all without .osz).

fn extract_filename_format(filename: &str) -> String {
    // Exact production shape: format! to append extension.
    let owned = filename.to_string();
    format!("{owned}.osz")
}

fn extract_filename_push(filename: &str) -> String {
    // Candidate: push_str reuses the owned allocation.
    let mut owned = filename.to_string();
    owned.push_str(".osz");
    owned
}

fn bench_extract_filename_append(c: &mut Criterion) {
    // Filenames without .osz extension — exercises the append branch only.
    let cases: &[(&str, &str)] = &[
        ("1234567", "id_only"),
        ("1234567 Artist - Song Title [Hard]", "typical"),
        (
            "9999999 A Very Long Artist Name With Spaces - A Very Long Song Title \
             That Goes On And On Including Extra Details [Expert Difficulty]",
            "long",
        ),
    ];

    let mut group = c.benchmark_group("extract_filename_append");

    for &(name, label) in cases {
        group.bench_with_input(BenchmarkId::new("format_append", label), name, |b, name| {
            b.iter(|| black_box(extract_filename_format(black_box(name))))
        });

        group.bench_with_input(
            BenchmarkId::new("push_str_append", label),
            name,
            |b, name| b.iter(|| black_box(extract_filename_push(black_box(name)))),
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_md5_hex_format,
    bench_mirror_url_for,
    bench_sanitize_filename,
    bench_hash_worker_update,
    bench_find_eocd_position,
    bench_split_content_disposition,
    bench_content_type_check,
    bench_collection_hashes,
    bench_pending_mirrors_clone,
    bench_write_collections_db_dedup,
    bench_temp_path_for_pid,
    bench_beatmapset_ids_capacity,
    bench_extract_filename_append,
    bench_event_enum_channel_send,
);
criterion_main!(benches);

// ── Event enum size — channel send cost ───────────────────────────────────────
//
// osu-downloader/src/event.rs — Event is sent via tokio::sync::mpsc on every
// beatmapset status update, progress tick, and completion.  The enum size is
// determined by its largest variant, SessionCompleted { summary: Summary }.
//
// Estimated layout (64-bit):
//   Summary = usize(8) + Vec<u32>(24) + Vec<(u32,Skip)>(24) + Vec<(u32,Error)>(24)
//             + u64(8) + Duration(16) = 104 bytes
//   Event discriminant + max-payload alignment → ~112 bytes
//
// The Progress variant (hottest: ~1 send per 128 KB chunk) only carries:
//   beatmapset_id:u32 + downloaded_bytes:u64 + total_bytes:Option<u64>(16) + speed_bps:u64
//   = ~40 bytes payload — but the channel moves the full 112-byte union.
//
// Candidate: Box<Summary> in SessionCompleted shrinks the variant to a pointer
//   (8 bytes), reducing enum size to ~88 bytes (~21% smaller).
//
// PUBLIC API CHANGE: SessionCompleted { summary: Box<Summary> } breaks callers
//   that pattern-match or construct this variant.  Requires a library semver bump.
//
// This bench measures the memcpy cost difference as a proxy, using stack-allocated
// structs of matching sizes sent through std::sync::mpsc channels (same move
// semantics as tokio::sync::mpsc for the send path).  Each "send" moves the value;
// the receiver just drops it — isolates the copy cost.

#[repr(C)]
#[derive(Clone)]
struct FatEvent([u8; 112]);

#[repr(C)]
#[derive(Clone)]
struct SlimEvent([u8; 88]);

fn bench_event_enum_channel_send(c: &mut Criterion) {
    const BATCH: usize = 500;

    let mut group = c.benchmark_group("event_enum_channel_send");
    group.throughput(Throughput::Elements(BATCH as u64));

    // Baseline: current enum size ~112 bytes per send.
    group.bench_function("fat_112_bytes", |b| {
        b.iter(|| {
            let (tx, rx) = mpsc::channel::<FatEvent>();
            for i in 0..BATCH {
                let ev = FatEvent([i as u8; 112]);
                tx.send(black_box(ev)).unwrap();
            }
            drop(tx);
            while rx.recv().is_ok() {}
        })
    });

    // Candidate: Box<Summary> variant reduces enum to ~88 bytes per send.
    group.bench_function("slim_88_bytes", |b| {
        b.iter(|| {
            let (tx, rx) = mpsc::channel::<SlimEvent>();
            for i in 0..BATCH {
                let ev = SlimEvent([i as u8; 88]);
                tx.send(black_box(ev)).unwrap();
            }
            drop(tx);
            while rx.recv().is_ok() {}
        })
    });

    group.finish();
}
