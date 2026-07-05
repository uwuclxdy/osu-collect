//! RAM measurement harness for the autonomous memory-optimisation loop.
//!
//! Placement: in the `osu-collect` package (not `osu-downloader`) because it exercises
//! `src/app/runtime::read_local_database` and the Updates-tab state structs that live in the
//! binary crate's lib surface. The `osu-downloader` package has no access to those.
//!
//! Build: `cargo build --release --example ram_harness`
//! Run:   `./target/release/examples/ram_harness <cold|updates|download|idle>`

use osu_collect::{app::update_source::UpdateSource, osu_db::OsuClient};
use osu_db::{
    Mode,
    collection::{Collection as DbCollection, CollectionList},
    listing::{Beatmap, Grade, Listing, RankedStatus, TimingPoint},
};
use std::time::Duration;
use tempfile::TempDir;

// ── synthetic DB shape ────────────────────────────────────────────────────────

const N_SETS: usize = 5_000;
const M_DIFFS: usize = 4;
/// Number of synthetic local collections each referencing a disjoint slice of the beatmaps.
const N_COLLECTIONS: usize = 10;

/// 32-char lowercase hex string from a deterministic seed (no real MD5).
fn fake_hash(beatmapset_id: u32, diff_idx: usize) -> String {
    let a = beatmapset_id as u64;
    let b = diff_idx as u64;
    let hi = a.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(b);
    let lo = b
        .wrapping_mul(0x6c62_272e_07bb_0142)
        .wrapping_add(a ^ 0xdeadbeef);
    format!("{hi:016x}{lo:016x}")
}

/// Minimal `Beatmap` struct accepted by `osu-db` serialisation.
fn minimal_beatmap(beatmapset_id: u32, beatmap_id: u32, hash: &str) -> Beatmap {
    use chrono::DateTime;
    let epoch = DateTime::from_timestamp(0, 0).unwrap();
    Beatmap {
        artist_ascii: None,
        artist_unicode: None,
        title_ascii: None,
        title_unicode: None,
        creator: None,
        difficulty_name: None,
        audio: None,
        hash: Some(hash.to_owned()),
        file_name: None,
        status: RankedStatus::Ranked,
        hitcircle_count: 0,
        slider_count: 0,
        spinner_count: 0,
        last_modified: epoch,
        approach_rate: 5.0,
        circle_size: 4.0,
        hp_drain: 5.0,
        overall_difficulty: 5.0,
        slider_velocity: 1.4,
        std_ratings: vec![],
        taiko_ratings: vec![],
        ctb_ratings: vec![],
        mania_ratings: vec![],
        drain_time: 60,
        total_time: 65_000,
        preview_time: 10_000,
        timing_points: vec![TimingPoint {
            bpm: 180.0,
            offset: 0.0,
            inherits: true,
        }],
        beatmap_id: beatmap_id as i32,
        beatmapset_id: beatmapset_id as i32,
        thread_id: 0,
        std_grade: Grade::Unplayed,
        taiko_grade: Grade::Unplayed,
        ctb_grade: Grade::Unplayed,
        mania_grade: Grade::Unplayed,
        local_beatmap_offset: 0,
        stack_leniency: 0.7,
        mode: Mode::Standard,
        song_source: None,
        tags: None,
        online_offset: 0,
        title_font: None,
        last_played: None,
        is_osz2: false,
        folder_name: Some(format!("set{beatmapset_id}")),
        last_online_check: epoch,
        ignore_sounds: false,
        ignore_skin: false,
        disable_storyboard: false,
        disable_video: false,
        visual_override: false,
        mysterious_short: None,
        mysterious_last_modified: 0,
        mania_scroll_speed: 0,
    }
}

/// Write a synthetic `osu!.db` + `collection.db` to `dir`.
fn write_synth_db(dir: &std::path::Path) {
    let mut beatmaps = Vec::with_capacity(N_SETS * M_DIFFS);
    for set_idx in 0..N_SETS {
        let set_id = (set_idx + 1) as u32;
        for diff_idx in 0..M_DIFFS {
            let bm_id = (set_idx * M_DIFFS + diff_idx + 1) as u32;
            let hash = fake_hash(set_id, diff_idx);
            beatmaps.push(minimal_beatmap(set_id, bm_id, &hash));
        }
    }
    let listing = Listing {
        version: 20191106,
        folder_count: N_SETS as u32,
        unban_date: None,
        player_name: Some("harness".to_owned()),
        beatmaps,
        user_permissions: 1,
    };
    listing
        .save(dir.join("osu!.db"))
        .expect("failed to write osu!.db");

    // Build N_COLLECTIONS collections with disjoint beatmap slices.
    let total_maps = N_SETS * M_DIFFS;
    let per_collection = total_maps / N_COLLECTIONS;
    let collections: Vec<DbCollection> = (0..N_COLLECTIONS)
        .map(|c| {
            let start = c * per_collection;
            let end = ((c + 1) * per_collection).min(total_maps);
            let hashes: Vec<Option<String>> = (start..end)
                .map(|global_diff| {
                    let set_idx = global_diff / M_DIFFS;
                    let diff_idx = global_diff % M_DIFFS;
                    Some(fake_hash((set_idx + 1) as u32, diff_idx))
                })
                .collect();
            DbCollection {
                // Embed a numeric osu!collector ID so `extract_collection_id` picks it up.
                name: Some(format!("[osucollector.com/collections/{c}] Synth {c}")),
                beatmap_hashes: hashes,
            }
        })
        .collect();
    CollectionList {
        version: 20150203,
        collections,
    }
    .to_file(dir.join("collection.db"))
    .expect("failed to write collection.db");
}

// ── /proc/self/status reader ──────────────────────────────────────────────────

struct ProcStatus {
    vm_hwm_kb: u64,
    vm_rss_kb: u64,
    vm_data_kb: u64,
}

fn read_proc_status() -> ProcStatus {
    let text = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status unavailable");
    let mut hwm = 0u64;
    let mut rss = 0u64;
    let mut data = 0u64;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("VmHWM:") => hwm = parts.next().unwrap_or("0").parse().unwrap_or(0),
            Some("VmRSS:") => rss = parts.next().unwrap_or("0").parse().unwrap_or(0),
            Some("VmData:") => data = parts.next().unwrap_or("0").parse().unwrap_or(0),
            _ => {}
        }
    }
    ProcStatus {
        vm_hwm_kb: hwm,
        vm_rss_kb: rss,
        vm_data_kb: data,
    }
}

fn print_mem(label: &str) {
    let s = read_proc_status();
    println!(
        "{label}: pid={} VmHWM={} kB  VmRSS={} kB  VmData={} kB",
        std::process::id(),
        s.vm_hwm_kb,
        s.vm_rss_kb,
        s.vm_data_kb,
    );
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "cold".to_owned());

    print_mem("startup");

    if scenario == "cold" {
        print_mem("cold");
        println!("scenario=cold done");
        return;
    }

    // Synthesise DB → read it through the real TUI code path → populate UpdateSource.
    let tmp: TempDir = TempDir::new().expect("tempdir");
    write_synth_db(tmp.path());

    let db_size = std::fs::metadata(tmp.path().join("osu!.db"))
        .map(|m| m.len())
        .unwrap_or(0);
    let coll_size = std::fs::metadata(tmp.path().join("collection.db"))
        .map(|m| m.len())
        .unwrap_or(0);
    println!(
        "synth-db: osu!.db={db_size} bytes  collection.db={coll_size} bytes  total={} bytes",
        db_size + coll_size
    );

    // Read through the same blocking code path the TUI uses.
    let (collections, beatmapsets, all_checksums) =
        osu_collect::app::runtime::read_local_database(OsuClient::Stable, tmp.path().to_path_buf())
            .expect("read_local_database failed");

    println!(
        "loaded: {} collections, {} beatmapsets, {} checksums",
        collections.len(),
        beatmapsets.len(),
        all_checksums.len()
    );

    print_mem("after-load");

    // Populate the real UpdateSource via its public setters — same path the TUI takes.
    let mut tab = UpdateSource::new();
    tab.set_collections(collections);
    tab.set_local_beatmapsets(beatmapsets);
    tab.set_all_checksums(all_checksums);

    // Keep `tab` alive so the allocations are measured.
    let _ = std::hint::black_box(&tab);

    print_mem("after-scan-build");
    std::thread::sleep(Duration::from_secs(2));
    print_mem("updates-settled");

    if scenario == "updates" {
        println!("scenario=updates done");
        drop(tab);
        return;
    }

    // download / idle: seed 200 pending IDs into a Vec<u32> as the download page would.
    let pending_ids: Vec<u32> = (1u32..=200).collect();
    let _ = std::hint::black_box(&pending_ids);

    print_mem("download-seeded");

    if scenario == "download" {
        println!("scenario=download done");
        return;
    }

    if scenario == "idle" {
        println!("sleeping 30s for idle check...");
        std::thread::sleep(Duration::from_secs(30));
        print_mem("idle-30s");
        println!("scenario=idle done");
        return;
    }

    eprintln!("unknown scenario: {scenario}. use cold|updates|download|idle");
    std::process::exit(1);
}
