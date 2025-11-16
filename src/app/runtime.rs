use super::{App, AppCommand};
use crate::{
    config::Config,
    download::{self, DownloadEvent, DownloadHandle, DownloadId},
    tui::draw,
    tui::terminal::{cleanup_terminal, setup_terminal, spawn_input_thread},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};

pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting application runtime loop");
    let validation_issue = config.validate().err().map(|e| e.to_string());
    let mut terminal = setup_terminal()?;
    let mut app = App::new(config);
    if let Some(msg) = validation_issue {
        warn!(error = %msg, "Configuration validation failed; surfacing to UI");
        app.home.set_error(&msg);
    }

    let (download_tx, mut download_rx) = mpsc::unbounded_channel::<DownloadEvent>();
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
            Some(input) = input_rx.recv() => {
                trace!(?input, "Processing input event");
                should_quit = handle_input(input, &mut app, &download_tx, &mut active_downloads);
            }
            else => break,
        }
    }

    app.home.quit_prompt = false;
    app.home.set_info("Quitting...");
    terminal.draw(|f| draw(f, &app))?;

    drop(download_rx);
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
    downloads: &mut HashMap<DownloadId, DownloadHandle>,
) -> bool {
    match input {
        InputEvent::Key(key) => handle_key_event(key, app, download_tx, downloads),
        InputEvent::Resize => false,
        InputEvent::Tick => false,
    }
}

fn handle_key_event(
    key: KeyEvent,
    app: &mut App,
    download_tx: &mpsc::UnboundedSender<DownloadEvent>,
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

#[derive(Clone, Debug)]
pub enum InputEvent {
    Key(KeyEvent),
    Resize,
    Tick,
}
