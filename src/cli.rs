use crate::{
    app::{
        collection_state, failed_maps, ignored_maps, runtime, snapshots,
        update_source::{MissingBeatmapset, extract_collection_id},
    },
    osu_db::{BeatmapReader, LazerReader, LocalBeatmapset, Md5, OsuClient, StableReader},
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Instant,
};
use tracing::{info, warn};

#[derive(Debug)]
pub struct UpdateCollectionsArgs {
    pub db: Option<PathBuf>,
    pub client: OsuClient,
    pub dry_run: bool,
    pub verbose: bool,
}

#[derive(Debug)]
pub enum CliCommand {
    UpdateCollections(UpdateCollectionsArgs),
}

/// Parse CLI arguments. Returns `None` when no subcommand is given (TUI mode).
pub fn parse_args() -> Result<Option<CliCommand>, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() {
        return Ok(None);
    }

    match args[0].as_str() {
        "update-collections" => {
            let cmd = parse_update_collections(&args[1..])?;
            Ok(Some(CliCommand::UpdateCollections(cmd)))
        }
        "--help" | "-h" => {
            print_help();
            std::process::exit(0);
        }
        "--version" | "-V" => {
            println!("osu-collect {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        other => Err(format!("unknown subcommand: {other}")),
    }
}

fn parse_update_collections(args: &[String]) -> Result<UpdateCollectionsArgs, String> {
    let mut db: Option<PathBuf> = None;
    let mut client = OsuClient::Stable;
    let mut dry_run = false;
    let mut verbose = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--db" => {
                i += 1;
                let path = args.get(i).ok_or("--db requires a path argument")?;
                db = Some(PathBuf::from(path));
            }
            "--lazer" => client = OsuClient::Lazer,
            "--stable" => client = OsuClient::Stable,
            "--dry-run" => dry_run = true,
            "--verbose" => verbose = true,
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }

    Ok(UpdateCollectionsArgs {
        db,
        client,
        dry_run,
        verbose,
    })
}

fn print_help() {
    println!(concat!(
        "osu-collect ",
        env!("CARGO_PKG_VERSION"),
        "\n\n",
        "usage:\n",
        "  osu-collect                          launch TUI\n",
        "  osu-collect update-collections [opts] check and report missing beatmaps\n\n",
        "update-collections options:\n",
        "  --db <path>    override osu! database path\n",
        "  --stable       use osu! stable database (default)\n",
        "  --lazer        use osu! lazer database\n",
        "  --dry-run      run without writing state\n",
        "  --verbose      raise log level to debug",
    ));
}

/// Run the update-collections flow without the TUI.
pub async fn run_update_collections(
    args: UpdateCollectionsArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let t_total = Instant::now();

    // Resolve osu! database path
    let db_path =
        match args.db {
            Some(p) => p,
            None => match args.client {
                OsuClient::Stable => StableReader::default_path()
                    .ok_or("could not detect osu! stable path; use --db")?,
                OsuClient::Lazer => LazerReader::default_path()
                    .ok_or("could not detect osu! lazer path; use --db")?,
            },
        };

    info!(path = %db_path.display(), "reading local database");
    let t_db = Instant::now();

    let (collections, beatmapsets, all_checksums) = tokio::task::spawn_blocking({
        let path = db_path;
        let client = args.client;
        move || runtime::read_local_database(client, path)
    })
    .await
    .map_err(|e| format!("task panicked: {e}"))
    .and_then(|r| r)?;

    let db_ms = t_db.elapsed().as_millis();
    info!(
        collections = collections.len(),
        beatmapsets = beatmapsets.len(),
        checksums = all_checksums.len(),
        elapsed_ms = db_ms,
        "phase: read database"
    );

    // Extract collection IDs from names (same logic as UpdateSource::set_collections)
    let collection_ids: Vec<u32> = collections
        .iter()
        .filter_map(|c| extract_collection_id(&c.name).and_then(|id| u32::try_from(id).ok()))
        .collect();

    if collection_ids.is_empty() {
        warn!("no collections with recognizable osu!collector IDs found");
        println!("no updatable collections found.");
        return Ok(());
    }

    info!(count = collection_ids.len(), "collections to check");

    let current_snapshots =
        snapshots::current_snapshots(args.client, &collections, &beatmapsets, |name| {
            extract_collection_id(name).and_then(|id| u32::try_from(id).ok())
        });

    // Build local beatmapset index
    let local_beatmapsets: HashMap<u32, LocalBeatmapset> =
        beatmapsets.into_iter().map(|bs| (bs.id, bs)).collect();
    let local_set_ids: HashSet<u32> = local_beatmapsets.keys().copied().collect();
    let local_checksums_set: HashSet<Md5> = all_checksums.into_iter().collect();

    // Load collection state for compatibility with existing state file
    let state_path = collection_state::state_path();
    let coll_state = state_path
        .as_deref()
        .map(collection_state::load)
        .unwrap_or_default();

    let snapshot_dir = snapshots::snapshots_dir();
    let snapshot_diffs = snapshot_dir
        .as_deref()
        .map(|dir| runtime::snapshot_diffs_for_scan(dir, &collection_ids, &current_snapshots))
        .unwrap_or_default();
    let added_count = snapshot_diffs
        .values()
        .map(|diff| diff.manually_added.len())
        .sum::<usize>();

    let failed_beatmapset_ids = failed_maps::failed_maps_path()
        .as_deref()
        .map(failed_maps::load)
        .map(|failed_maps| failed_maps.ids())
        .unwrap_or_default();
    let ignored_beatmapset_ids = ignored_maps::ignored_maps_path()
        .map(|path| ignored_maps::reconcile_installed(&path, &local_set_ids))
        .unwrap_or_default();

    info!("fetching collections from API");
    let t_fetch = Instant::now();

    let fetch_result = runtime::fetch_missing_beatmapsets(
        args.client,
        collection_ids.clone(),
        local_set_ids,
        local_checksums_set,
        &collections,
        snapshot_diffs,
        runtime::FetchCompareSettings {
            hidden_failed_beatmapset_ids: failed_beatmapset_ids,
            ignored_beatmapset_ids,
        },
    )
    .await?;
    let missing = fetch_result.missing;
    let collection_seen = fetch_result.collection_seen;
    let hidden_failed_count = fetch_result.hidden_failed_count;

    let fetch_ms = t_fetch.elapsed().as_millis();
    let (fetchable_count, held_back_count) = report_counts(&missing);
    info!(
        missing = fetchable_count,
        held_back = held_back_count,
        elapsed_ms = fetch_ms,
        "phase: fetch + compare"
    );

    // Save updated state (unless dry-run)
    if !args.dry_run {
        if let Some(path) = state_path {
            let local_snapshot: Vec<u32> = local_beatmapsets.keys().copied().collect();
            let mut updated_state = coll_state;
            for (collection_id, ids) in collection_seen {
                let installed_ids: Vec<u32> = ids
                    .iter()
                    .copied()
                    .filter(|id| local_beatmapsets.contains_key(id))
                    .collect();
                updated_state.update(collection_id, ids, installed_ids, local_snapshot.clone());
            }
            let state_to_save = updated_state;
            match tokio::task::spawn_blocking(move || collection_state::save(&state_to_save, &path))
                .await
            {
                Ok(_) => {}
                Err(err) => warn!("failed to save collection state: {err}"),
            }
        }
        if let Some(snapshot_dir) = snapshot_dir {
            let client = args.client;
            let missing_for_persist = missing.clone();
            match tokio::task::spawn_blocking(move || {
                persist_baselines(
                    &snapshot_dir,
                    current_snapshots,
                    client,
                    &missing_for_persist,
                );
            })
            .await
            {
                Ok(_) => {}
                Err(err) => warn!("failed to save collection snapshots: {err}"),
            }
        }
    } else {
        info!("dry-run: skipping state write");
    }

    let total_ms = t_total.elapsed().as_millis();
    println!("update-collections complete: {fetchable_count} missing mapsets");
    println!("  phase db-read:       {db_ms}ms");
    println!("  phase fetch+compare: {fetch_ms}ms");
    println!("  total:               {total_ms}ms");

    if held_back_count > 0 {
        eprintln!(
            "info: {held_back_count} previously-deleted mapsets held back; re-include them from the update browse"
        );
    }
    if added_count > 0 {
        eprintln!("info: {added_count} mapsets added manually since last scan; they will remain");
    }
    if hidden_failed_count > 0 {
        eprintln!(
            "info: {hidden_failed_count} failed mapsets hidden; recheck from the TUI to include them"
        );
    }

    if !missing.is_empty() {
        println!("missing mapsets:");
        for m in &missing {
            let tag = row_tag(m);
            println!(
                "  mapset {} in collection {} ({}){tag}",
                m.id, m.collection_id, m.collection_name
            );
        }
    }

    Ok(())
}

/// The `update-collections` report's headline figures: how many sets an update
/// run would enqueue, and how many the scan held back as previously deleted.
/// Held-back sets still print in the listing (tagged), so the two always sum to
/// the scanned total.
fn report_counts(missing: &[MissingBeatmapset]) -> (usize, usize) {
    let fetchable = missing.iter().filter(|m| m.included).count();
    (fetchable, missing.len() - fetchable)
}

/// Write `update-collections`' per-collection baselines to disk, rebuilt so every
/// held-back set stays recorded as deleted.
///
/// This command reports, it never downloads — so writing the scan's raw baselines
/// would reset every collection to its current local membership and erase the very
/// deletions the summary is about to call held back, making the report
/// self-defeating. Same rebuild the TUI's run-completion write does.
///
/// The rebuild and the write live in ONE function on purpose: a helper that only
/// returned the rebuilt map would be pinned by asserting its return value, and
/// dropping its call from the caller would then change nothing any test observes.
/// Here the test asserts the bytes on disk, so dropping the rebuild changes what
/// lands and reds.
fn persist_baselines(
    snapshot_dir: &std::path::Path,
    mut baselines: HashMap<u32, snapshots::CollectionSnapshotFile>,
    client: OsuClient,
    missing: &[MissingBeatmapset],
) {
    runtime::retain_held_back_in_snapshots(&mut baselines, client, missing);
    for (collection_id, snapshot) in baselines {
        snapshots::save(
            &snapshot,
            &snapshots::snapshot_path(snapshot_dir, collection_id),
        );
    }
}

/// The listing's per-row marker. Reads the same field [`report_counts`] counts,
/// so the number of tagged rows and the reported held-back figure cannot
/// disagree.
fn row_tag(set: &MissingBeatmapset) -> &'static str {
    if set.included { "" } else { " [held back]" }
}

#[cfg(test)]
#[path = "../tests/unit/cli.rs"]
mod tests;
