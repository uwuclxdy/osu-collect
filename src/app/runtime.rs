use super::{App, AppCommand};
use crate::{
    app::updates::{MissingBeatmapset, MissingStatus, ScanStatus},
    config::Config,
    core::collection::{Collection, api_client},
    download::{self, DownloadEvent, DownloadHandle, DownloadId},
    osu_db::{BeatmapReader, LazerReader, LocalBeatmapset, OsuClient, StableReader},
    tui::draw,
    tui::terminal::{cleanup_terminal, setup_terminal, spawn_input_thread},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{collections::HashMap, path::PathBuf};
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};

#[derive(Debug, Clone)]
pub enum UpdatesEvent {
    DatabaseRead {
        generation: u64,
        collections: Vec<crate::osu_db::LocalCollection>,
        beatmapsets: Vec<LocalBeatmapset>,
    },
    ScanComplete {
        generation: u64,
        missing: Vec<MissingBeatmapset>,
    },
    Error(String),
}

pub async fn run(
    config: Config,
    startup_notice: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting application runtime loop");
    let validation_issue = config.validate().err().map(|e| e.to_string());
    let mut terminal = setup_terminal()?;
    let mut app = App::new(config);
    let mut notice = startup_notice;
    if let Some(msg) = validation_issue {
        warn!(error = %msg, "Configuration validation failed; surfacing to UI");
        if let Some(ref notice_text) = notice {
            app.home.set_error(&format!("{msg}\n{notice_text}"));
            notice = None;
        } else {
            app.home.set_error(&msg);
        }
    }
    if let Some(message) = notice.take() {
        app.home.set_info(&message);
    }

    let (download_tx, mut download_rx) = mpsc::unbounded_channel::<DownloadEvent>();
    let (updates_tx, mut updates_rx) = mpsc::unbounded_channel::<UpdatesEvent>();
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<InputEvent>();
    let input_handle = spawn_input_thread(input_tx.clone());

    let mut should_quit = false;
    let mut active_downloads: HashMap<DownloadId, DownloadHandle> = HashMap::new();

    while !should_quit {
        terminal.draw(|f| draw(f, &app))?;

        tokio::select! {
            Some(event) = download_rx.recv() => {
                trace!(?event, "Received download event");
                if let Some(completed_id) = download_finished_id(&event) {
                    debug!(download_id = completed_id, "Download handle finished; awaiting join");
                    if let Some(handle) = active_downloads.remove(&completed_id) {
                        tokio::spawn(async move {
                            handle.wait().await;
                        });
                    }
                }
                app.handle_download_event(event);
            }
            Some(event) = updates_rx.recv() => {
                trace!(?event, "Received updates event");
                handle_updates_event(event, &mut app, &updates_tx);
            }
            Some(input) = input_rx.recv() => {
                trace!(?input, "Processing input event");
                should_quit = handle_input(input, &mut app, &download_tx, &updates_tx, &mut active_downloads);
            }
            else => break,
        }
    }

    app.home.quit_prompt = false;
    app.home.set_info("Quitting...");
    terminal.draw(|f| draw(f, &app))?;

    drop(download_rx);
    drop(updates_rx);
    drop(input_rx);
    signal_abort_downloads(&mut active_downloads);
    abort_and_wait_downloads(&mut active_downloads).await;

    drop(input_tx);
    if let Some(handle) = input_handle {
        let _ = handle.join();
    }
    cleanup_terminal(&mut terminal)?;

    Ok(())
}

fn handle_input(
    input: InputEvent,
    app: &mut App,
    download_tx: &mpsc::UnboundedSender<DownloadEvent>,
    updates_tx: &mpsc::UnboundedSender<UpdatesEvent>,
    downloads: &mut HashMap<DownloadId, DownloadHandle>,
) -> bool {
    match input {
        InputEvent::Key(key) => handle_key_event(key, app, download_tx, updates_tx, downloads),
        InputEvent::Resize => false,
        InputEvent::Tick => false,
    }
}

fn handle_key_event(
    key: KeyEvent,
    app: &mut App,
    download_tx: &mpsc::UnboundedSender<DownloadEvent>,
    updates_tx: &mpsc::UnboundedSender<UpdatesEvent>,
    downloads: &mut HashMap<DownloadId, DownloadHandle>,
) -> bool {
    trace!(?key, "Handling key event");
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        warn!("CTRL+C received; signalling abort for all downloads");
        signal_abort_downloads(downloads);
        return true;
    }

    match app.handle_key(key) {
        Some(AppCommand::StartDownload { id, request }) => {
            let handle = download::spawn_download(id, request, download_tx.clone());
            info!(download_id = id, "Spawned download from UI request");
            downloads.insert(id, handle);
        }
        Some(AppCommand::StartSelectiveDownload { id, request }) => {
            let handle = download::spawn_selective_download(id, request, download_tx.clone());
            info!(
                download_id = id,
                "Spawned selective download from Updates tab"
            );
            downloads.insert(id, handle);
        }
        Some(AppCommand::CancelDownload { id }) => {
            let was_running = if let Some(handle) = downloads.remove(&id) {
                handle.request_shutdown();
                tokio::spawn(async move {
                    handle.wait().await;
                });
                info!(download_id = id, "Requested shutdown for active download");
                true
            } else {
                false
            };
            app.handle_cancel_result(id, was_running);
        }
        Some(AppCommand::ScanLocalDatabase) => {
            spawn_scan_task(app, updates_tx.clone());
        }
        Some(AppCommand::Quit) => {
            if downloads.is_empty() {
                info!("No downloads active; exiting application");
            } else {
                info!("Quit confirmed; aborting downloads and exiting");
            }
            signal_abort_downloads(downloads);
            return true;
        }
        None => {}
    }

    false
}

fn download_finished_id(event: &DownloadEvent) -> Option<DownloadId> {
    match event {
        DownloadEvent::Finished { id, .. } => Some(*id),
        DownloadEvent::Failed { id, .. } => Some(*id),
        _ => None,
    }
}

fn signal_abort_downloads(downloads: &mut HashMap<DownloadId, DownloadHandle>) {
    if downloads.is_empty() {
        return;
    }
    warn!(
        active = downloads.len(),
        "Signalling shutdown for active downloads"
    );
    for handle in downloads.values() {
        handle.request_shutdown();
    }
}

async fn abort_and_wait_downloads(downloads: &mut HashMap<DownloadId, DownloadHandle>) {
    if downloads.is_empty() {
        return;
    }

    warn!(
        remaining = downloads.len(),
        "Awaiting graceful shutdown for downloads"
    );
    for handle in downloads.values() {
        handle.request_shutdown();
    }

    let mut pending: Vec<DownloadHandle> = downloads.drain().map(|(_, handle)| handle).collect();
    for handle in pending.drain(..) {
        debug!("Waiting for download task to complete");
        handle.wait().await;
    }
}

fn handle_updates_event(
    event: UpdatesEvent,
    app: &mut App,
    updates_tx: &mpsc::UnboundedSender<UpdatesEvent>,
) {
    match event {
        UpdatesEvent::DatabaseRead {
            generation,
            collections,
            beatmapsets,
        } => {
            // Ignore stale results from previous scan
            if generation != app.updates.scan_generation {
                debug!(
                    expected = app.updates.scan_generation,
                    got = generation,
                    "Ignoring stale DatabaseRead event"
                );
                return;
            }

            app.updates.set_collections(collections);
            app.updates.set_local_beatmapsets(beatmapsets);
            app.updates.scan_status = ScanStatus::FetchingCollection;
            app.updates.set_loading("Fetching collections...");

            let selected_ids = app.updates.selected_collection_ids();
            if selected_ids.is_empty() {
                app.updates.scan_status = ScanStatus::Ready;
                app.updates
                    .set_info("No collections with IDs found to compare");
                return;
            }

            spawn_fetch_and_compare_task(app, updates_tx.clone());
        }
        UpdatesEvent::ScanComplete {
            generation,
            missing,
        } => {
            // Ignore stale results from previous scan
            if generation != app.updates.scan_generation {
                debug!(
                    expected = app.updates.scan_generation,
                    got = generation,
                    "Ignoring stale ScanComplete event"
                );
                return;
            }

            let count = missing.len();
            app.updates.set_missing_beatmaps(missing);
            app.updates.scan_status = ScanStatus::Ready;
            app.updates.set_info(format!(" {count} updatable beatmaps"));
        }
        UpdatesEvent::Error(msg) => {
            app.updates.set_error(msg);
        }
    }
}

fn spawn_scan_task(app: &mut App, tx: mpsc::UnboundedSender<UpdatesEvent>) {
    let client_type = app.updates.client_type;
    let osu_path = PathBuf::from(app.updates.osu_path());
    let generation = app.updates.scan_generation;

    app.updates.scan_status = ScanStatus::ReadingDatabase;
    app.updates.clear_message();
    app.updates.set_loading("Reading database...");

    tokio::spawn(async move {
        let result =
            tokio::task::spawn_blocking(move || read_local_database(client_type, osu_path))
                .await
                .map_err(|e| format!("Task panicked: {e}"))
                .and_then(|r| r);

        match result {
            Ok((collections, beatmapsets)) => {
                let _ = tx.send(UpdatesEvent::DatabaseRead {
                    generation,
                    collections,
                    beatmapsets,
                });
            }
            Err(err) => {
                let _ = tx.send(UpdatesEvent::Error(err));
            }
        }
    });
}

fn read_local_database(
    client_type: OsuClient,
    path: PathBuf,
) -> Result<(Vec<crate::osu_db::LocalCollection>, Vec<LocalBeatmapset>), String> {
    match client_type {
        OsuClient::Stable => {
            let reader = StableReader::new(path);
            let collections = reader.list_collections()?;
            let beatmapsets = reader.list_beatmapsets()?;
            Ok((collections, beatmapsets))
        }
        OsuClient::Lazer => {
            let reader = LazerReader::new(path);
            let collections = reader.list_collections()?;
            let beatmapsets = reader.list_beatmapsets()?;
            Ok((collections, beatmapsets))
        }
    }
}

fn spawn_fetch_and_compare_task(app: &mut App, tx: mpsc::UnboundedSender<UpdatesEvent>) {
    // Fetch ALL collections with IDs, not just selected ones
    // Selection filtering happens locally from cache
    let all_collection_ids: Vec<u32> = app
        .updates
        .local_collections
        .iter()
        .filter_map(|c| c.collection_id.and_then(|id| u32::try_from(id).ok()))
        .collect();

    let local_beatmapsets: HashMap<u32, LocalBeatmapset> = app.updates.local_beatmapsets.clone();
    let generation = app.updates.scan_generation;

    app.updates.scan_status = ScanStatus::FetchingCollection;

    tokio::spawn(async move {
        let result = fetch_and_compare(all_collection_ids, local_beatmapsets).await;

        match result {
            Ok(missing) => {
                let _ = tx.send(UpdatesEvent::ScanComplete {
                    generation,
                    missing,
                });
            }
            Err(err) => {
                let _ = tx.send(UpdatesEvent::Error(err));
            }
        }
    });
}

async fn fetch_and_compare(
    collection_ids: Vec<u32>,
    local_beatmapsets: HashMap<u32, LocalBeatmapset>,
) -> Result<Vec<MissingBeatmapset>, String> {
    let client = api_client::default_http_client().map_err(|e| e.to_string())?;
    let mut all_missing: Vec<MissingBeatmapset> = Vec::new();
    let mut seen_beatmapsets: std::collections::HashSet<u32> = std::collections::HashSet::new();

    let local_checksums: std::collections::HashSet<String> = local_beatmapsets
        .values()
        .flat_map(|bs| bs.beatmaps.iter().map(|b| b.checksum.clone()))
        .collect();

    for collection_id in collection_ids {
        let collection: Collection = api_client::fetch_collection(&client, collection_id)
            .await
            .map_err(|e| e.to_string())?;

        for beatmapset in &collection.beatmapsets {
            // Skip if we've already added this beatmapset (from another collection)
            if seen_beatmapsets.contains(&beatmapset.id) {
                continue;
            }

            let local_set = local_beatmapsets.get(&beatmapset.id);

            let status = match local_set {
                None => Some(MissingStatus::NotInstalled),
                Some(local) => {
                    let all_match = beatmapset.beatmaps.iter().all(|remote_beatmap| {
                        local_checksums.contains(remote_beatmap.checksum.as_ref())
                    });

                    if all_match {
                        None
                    } else {
                        // Check if online has beatmap IDs that local doesn't have
                        // This indicates the online version has new difficulties
                        let local_beatmap_ids: std::collections::HashSet<u32> =
                            local.beatmaps.iter().map(|b| b.id).collect();

                        let online_has_new_diffs = beatmapset
                            .beatmaps
                            .iter()
                            .any(|rb| !local_beatmap_ids.contains(&rb.id));

                        if online_has_new_diffs {
                            Some(MissingStatus::NewDifficulties)
                        } else {
                            // Same beatmap IDs but different checksums - can't determine
                            // if online is newer or local has modifications, skip it
                            None
                        }
                    }
                }
            };

            if let Some(status) = status {
                seen_beatmapsets.insert(beatmapset.id);
                all_missing.push(MissingBeatmapset {
                    id: beatmapset.id,
                    status,
                    collection_id,
                    collection_name: collection.name.to_string(),
                });
            }
        }
    }

    // Sort by collection_id first, then by beatmapset id within each collection
    all_missing.sort_by(|a, b| {
        a.collection_id
            .cmp(&b.collection_id)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(all_missing)
}

#[derive(Clone, Debug)]
pub enum InputEvent {
    Key(KeyEvent),
    Resize,
    Tick,
}
