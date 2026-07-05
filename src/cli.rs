use crate::{
    app::{
        collection_state, failed_maps, ignored_maps, runtime, snapshots,
        update_source::extract_collection_id,
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
    let hidden_failed_count = failed_beatmapset_ids.len();
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

    let fetch_ms = t_fetch.elapsed().as_millis();
    let previously_deleted_count = missing.iter().filter(|m| m.previously_deleted).count();
    info!(
        missing = missing.len(),
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
            match tokio::task::spawn_blocking(move || {
                for (collection_id, snapshot) in current_snapshots {
                    snapshots::save(
                        &snapshot,
                        &snapshots::snapshot_path(&snapshot_dir, collection_id),
                    );
                }
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
    println!(
        "update-collections complete: {} missing beatmapsets",
        missing.len()
    );
    println!("  phase db-read:       {db_ms}ms");
    println!("  phase fetch+compare: {fetch_ms}ms");
    println!("  total:               {total_ms}ms");

    if previously_deleted_count > 0 {
        eprintln!(
            "info: {previously_deleted_count} maps skipped (previously deleted — select them to re-include)"
        );
    }
    if added_count > 0 {
        eprintln!("info: {added_count} maps added manually since last scan; they will remain");
    }
    if hidden_failed_count > 0 {
        eprintln!(
            "info: {hidden_failed_count} failed maps hidden; recheck from the TUI to include them"
        );
    }

    if !missing.is_empty() {
        println!("missing beatmapsets:");
        for m in &missing {
            let tag = if m.previously_deleted {
                " [skipped]"
            } else {
                ""
            };
            println!(
                "  beatmapset {} in collection {} ({}){tag}",
                m.id, m.collection_id, m.collection_name
            );
        }
    }

    Ok(())
}
