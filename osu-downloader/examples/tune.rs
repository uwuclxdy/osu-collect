//! Tuning harness for `docs/plan/mirror-scheduler.md` (repo root).
//!
//! Runs a full, resumable download of an osucollector.com collection while
//! logging one JSONL line per HTTP attempt (via the `instrument` feature) plus
//! one line per terminal per-beatmapset event, so the AIMD/escalation
//! constants in `mirrors::pool` can be fitted from the log afterward.
//!
//! ```text
//! cargo run --manifest-path osu-downloader/Cargo.toml \
//!     --features "instrument collection" --example tune -- \
//!     --out ./tune-downloads [--collection 22346] [--log tune-22346.jsonl] [--threads 8]
//! ```
//!
//! Resumable: rerun with the same `--out`/`--log` to continue. `OnExists::Skip`
//! picks up where a prior run left off; the log is opened in append mode.

use futures_util::StreamExt;
use osu_downloader::collection::CollectionClient;
use osu_downloader::instrument::{AttemptObserver, AttemptRecord};
use osu_downloader::{Downloader, Event, Mirror, OnExists};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_COLLECTION: u32 = 22346;
const FETCH_ATTEMPTS: u8 = 5;
const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

struct Args {
    collection: u32,
    out: PathBuf,
    log: PathBuf,
    threads: Option<usize>,
}

fn usage() -> &'static str {
    "usage: tune --out <dir> [--collection <id>] [--log <path>] [--threads <n>]\n\n\
     downloads an osucollector.com collection (default 22346) into <dir>,\n\
     logging one JSONL line per mirror attempt plus terminal per-map events\n\
     for fitting the mirror scheduler's timing constants.\n\n\
     --out <dir>        output directory for downloaded archives (required)\n\
     --collection <id>  osucollector.com collection id (default 22346)\n\
     --log <path>       JSONL log path (default tune-<collection>.jsonl in cwd)\n\
     --threads <n>      concurrent downloads (default: library default, 4)\n\n\
     resumable: rerun with the same --out/--log to continue (OnExists::Skip).\n\
     ctrl-c cancels gracefully; the partial log stays valid."
}

fn parse_args() -> Result<Args, String> {
    let mut collection = DEFAULT_COLLECTION;
    let mut out: Option<PathBuf> = None;
    let mut log: Option<PathBuf> = None;
    let mut threads: Option<usize> = None;

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--collection" => {
                let value = argv.next().ok_or("--collection needs a value")?;
                collection = value
                    .parse()
                    .map_err(|_| format!("invalid --collection: {value}"))?;
            }
            "--out" => out = Some(PathBuf::from(argv.next().ok_or("--out needs a value")?)),
            "--log" => log = Some(PathBuf::from(argv.next().ok_or("--log needs a value")?)),
            "--threads" => {
                let value = argv.next().ok_or("--threads needs a value")?;
                let n: usize = value
                    .parse()
                    .map_err(|_| format!("invalid --threads: {value}"))?;
                if n == 0 {
                    return Err("--threads must be greater than zero".into());
                }
                threads = Some(n);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let out = out.ok_or("--out is required")?;
    let log = log.unwrap_or_else(|| PathBuf::from(format!("tune-{collection}.jsonl")));
    Ok(Args {
        collection,
        out,
        log,
        threads,
    })
}

/// Escape a string for embedding in a hand-rolled JSON line. Only `host`
/// (mirror-controlled) and error messages need this; every other field is
/// numeric or drawn from a fixed enum set via `{:?}`.
fn escape_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn opt_num<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".to_string(), |v| v.to_string())
}

fn write_line(writer: &Mutex<BufWriter<File>>, line: String) {
    // Best-effort: a log write failure shouldn't abort a multi-hour download.
    if let Ok(mut w) = writer.lock() {
        let _ = writeln!(w, "{line}");
        let _ = w.flush();
    }
}

fn write_attempt(writer: &Mutex<BufWriter<File>>, record: &AttemptRecord) {
    write_line(
        writer,
        format!(
            "{{\"ts_ms\":{},\"host\":\"{}\",\"kind\":\"{:?}\",\"outcome\":\"{:?}\",\
             \"http_status\":{},\"retry_after_ms\":{},\"interval_ms\":{},\"latency_ms\":{}}}",
            record.elapsed.as_millis(),
            escape_json(&record.host),
            record.kind,
            record.outcome,
            opt_num(record.http_status),
            opt_num(record.retry_after.map(|d| d.as_millis())),
            record.interval.as_millis(),
            record.latency.as_millis(),
        ),
    );
}

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::args().any(|a| a == "-h" || a == "--help") {
        eprintln!("{}", usage());
        return ExitCode::SUCCESS;
    }

    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("error: {err}\n\n{}", usage());
            return ExitCode::from(2);
        }
    };

    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}

async fn run(args: Args) -> Result<(), String> {
    std::fs::create_dir_all(&args.out).map_err(|e| format!("create --out dir: {e}"))?;

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args.log)
        .map_err(|e| format!("open --log {}: {e}", args.log.display()))?;
    let writer = Arc::new(Mutex::new(BufWriter::new(log_file)));

    eprintln!("fetching collection {}...", args.collection);
    let collection = CollectionClient::new()
        .fetch_retrying(args.collection, FETCH_ATTEMPTS)
        .await
        .map_err(|e| format!("fetch collection {}: {e}", args.collection))?;
    let ids = collection.beatmapset_ids();
    let total = ids.len();
    eprintln!(
        "collection {} ({:?}): {total} beatmapsets -> {}",
        args.collection,
        collection.name,
        args.out.display()
    );

    // OsuApi needs a bearer token this harness has no login flow to obtain;
    // every other built-in downloads anonymously.
    let mirrors: Vec<Mirror> = Mirror::builtins()
        .into_iter()
        .filter(|m| !m.kind().requires_auth())
        .collect();

    let obs_writer = writer.clone();
    let observer = AttemptObserver::new(move |record: AttemptRecord| {
        write_attempt(&obs_writer, &record);
    });

    let mut builder = Downloader::builder()
        .mirrors(mirrors)
        .on_exists(OnExists::Skip)
        .attempt_observer(observer);
    if let Some(threads) = args.threads {
        builder = builder.concurrent_downloads(threads);
    }
    let downloader = builder
        .build()
        .map_err(|e| format!("build downloader: {e}"))?;

    let start = Instant::now();
    let mut session = downloader.download_many(ids, &args.out);
    let mut events = session.events().expect("first events() call");
    let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await; // first tick fires immediately; skip it

    let mut active: HashSet<u32> = HashSet::new();
    let mut deferred_pending: HashSet<u32> = HashSet::new();
    let mut done = 0usize;
    let mut failed = 0usize;
    let mut cancelled = false;

    loop {
        tokio::select! {
            ctrl_c = tokio::signal::ctrl_c(), if !cancelled => {
                if ctrl_c.is_ok() {
                    cancelled = true;
                    eprintln!("\nctrl-c: cancelling, partial log stays valid (rerun to resume)...");
                    session.cancel();
                }
            }
            _ = ticker.tick() => {
                eprintln!(
                    "[{:>5}s] {done}/{total} done, {} active, {} deferred-pending, {failed} failed",
                    start.elapsed().as_secs(),
                    active.len(),
                    deferred_pending.len(),
                );
            }
            event = events.next() => {
                let Some(event) = event else { break };
                let ts_ms = start.elapsed().as_millis();
                match event {
                    Event::BeatmapsetStatus { beatmapset_id, .. } => {
                        active.insert(beatmapset_id);
                    }
                    Event::BeatmapsetDeferred { beatmapset_id, pass, retry_in } => {
                        active.remove(&beatmapset_id);
                        deferred_pending.insert(beatmapset_id);
                        write_line(&writer, format!(
                            "{{\"event\":\"deferred\",\"ts_ms\":{ts_ms},\"beatmapset_id\":{beatmapset_id},\
                             \"pass\":{pass},\"retry_in_ms\":{}}}",
                            retry_in.as_millis(),
                        ));
                    }
                    Event::BeatmapsetCompleted { beatmapset_id, size_bytes, mirror_used, .. } => {
                        active.remove(&beatmapset_id);
                        deferred_pending.remove(&beatmapset_id);
                        done += 1;
                        write_line(&writer, format!(
                            "{{\"event\":\"completed\",\"ts_ms\":{ts_ms},\"beatmapset_id\":{beatmapset_id},\
                             \"host\":\"{}\",\"kind\":\"{:?}\",\"size_bytes\":{size_bytes}}}",
                            escape_json(&mirror_used.host), mirror_used.kind,
                        ));
                    }
                    Event::BeatmapsetSkipped { beatmapset_id, reason } => {
                        active.remove(&beatmapset_id);
                        deferred_pending.remove(&beatmapset_id);
                        done += 1;
                        write_line(&writer, format!(
                            "{{\"event\":\"skipped\",\"ts_ms\":{ts_ms},\"beatmapset_id\":{beatmapset_id},\
                             \"reason\":\"{reason:?}\"}}",
                        ));
                    }
                    Event::BeatmapsetFailed { beatmapset_id, error, mirror } => {
                        active.remove(&beatmapset_id);
                        deferred_pending.remove(&beatmapset_id);
                        failed += 1;
                        let mirror_field = mirror.map_or_else(|| "null".to_string(), |k| format!("\"{k:?}\""));
                        write_line(&writer, format!(
                            "{{\"event\":\"failed\",\"ts_ms\":{ts_ms},\"beatmapset_id\":{beatmapset_id},\
                             \"error\":\"{}\",\"mirror\":{mirror_field}}}",
                            escape_json(&error.to_string()),
                        ));
                    }
                    Event::SessionStarted { .. } | Event::Progress { .. } | Event::SessionCompleted { .. } => {}
                }
            }
        }
    }

    let summary = session
        .wait()
        .await
        .map_err(|e| format!("session task: {e}"))?;
    eprintln!(
        "{}: {} downloaded, {} skipped, {} failed, {} bytes, {:.1}s elapsed -> log {}",
        if cancelled { "cancelled" } else { "complete" },
        summary.downloaded.len(),
        summary.skipped.len(),
        summary.failed.len(),
        summary.total_bytes,
        start.elapsed().as_secs_f64(),
        args.log.display(),
    );
    Ok(())
}
