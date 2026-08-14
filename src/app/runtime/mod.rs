mod auth;
mod cover;
mod details;
mod enrich;
mod filter;
mod mirror_probe;
mod resolve;
pub(crate) mod scan;
mod search;
mod size;
mod update;

pub use mirror_probe::{MirrorProbeEvent, ProbeResult, probe_url};
pub use scan::{
    FetchCompareSettings, FetchMissingResult, UpdatesEvent, collection_ids_for_scan,
    exclude_reincluded_sets, fetch_missing_beatmapsets, owned_beatmapset_ids, read_local_database,
    retain_held_back_in_snapshots, should_hide_failed_beatmapset, snapshot_diffs_for_scan,
};

pub use cover::{HomeCoverEvent, handle_home_cover_event};
pub use enrich::{EnrichEvent, handle_enrich_event};
pub use filter::{HomeFilterEvent, handle_home_filter_event};
pub use resolve::{HomeResolveEvent, HomeResolveKind, handle_home_resolve_event};
pub use search::{HomeSearchEvent, handle_home_search_event};
pub use size::{HomeSizeEvent, handle_home_size_event};

use super::{App, AppCommand, AuthLoginState, EnrichTarget, FindBackend, Tab};
use crate::{
    config::Config,
    download::{self, DownloadEvent, DownloadHandle, DownloadId},
    tui::terminal::{TerminalGuard, TuiTerminal, setup_terminal, spawn_input_thread},
    tui::{apply_theme, draw},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_image::picker::{Picker, ProtocolType};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};

use auth::{
    AuthEvent, handle_auth_event, spawn_lazer_login_task, spawn_logout_task, spawn_reissue_task,
    spawn_supporter_refresh_task, spawn_verification_task,
};
use cover::schedule_cover_fetch;
use details::{HomeDetailsEvent, handle_home_details_event, schedule_details};
use enrich::{enrich_sink_mut, schedule_enrichment};
use filter::schedule_filter;
use mirror_probe::{handle_mirror_probe_event, schedule_probe};
use resolve::{cancel_resolve, schedule_resolve};
use scan::{handle_updates_event, spawn_failed_map_recheck_task, spawn_scan_task};
use search::schedule_search;
use size::schedule_size_probe;
use update::{UpdateEvent, handle_update_event, spawn_apply_update, spawn_update_check};

/// Render one frame. A focused text field positions the terminal caret via
/// [`ratatui::Frame::set_cursor_position`] inside the draw closure; ratatui 0.30
/// applies it *after* the buffer flush, so there is no flash at the old spot. A
/// frame that never calls it leaves the cursor hidden.
fn render_frame(terminal: &mut TuiTerminal, app: &App) -> std::io::Result<()> {
    terminal.draw(|f| draw(f, app))?;
    Ok(())
}

/// What sits between this process and the terminal that paints the image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Host {
    /// Nothing: the process environment describes the real terminal.
    Direct,
    /// A multiplexer `ratatui-image` will wrap its escapes in passthrough for,
    /// so a pixel protocol still reaches the outer terminal.
    Passthrough,
    /// A multiplexer that swallows graphics escapes: GNU screen, or a tmux
    /// `ratatui-image` won't wrap for. Only text can be trusted to arrive.
    Opaque,
}

/// The outer terminal's graphics capability, as far as trustworthy evidence goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outer {
    /// Speaks kitty graphics with unicode placeholders. Placement anchors to
    /// text cells, making it the one pixel protocol a multiplexer's redraw
    /// cannot wipe.
    KittyPlaceholders,
    /// Speaks iTerm2 inline images and nothing better (konsole).
    Iterm2Only,
    /// No evidence worth acting on.
    Unknown,
}

/// The queried graphics picker, with a protocol forced when detection was
/// blocked by a stale environment. See [`protocol_override`].
fn query_cover_picker() -> Picker {
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let host = detect_host();
    let outer = resolve_outer(outer_termname(host).as_deref(), is_konsole_env(), host);
    let detected = picker.protocol_type();
    if let Some(forced) = protocol_override(detected, outer) {
        picker.set_protocol_type(forced);
    }
    // Every "covers don't render" report turns on these five values.
    debug!(
        ?host,
        ?outer,
        ?detected,
        chosen = ?picker.protocol_type(),
        font_size = ?picker.font_size(),
        "Resolved cover graphics protocol"
    );
    picker
}

/// What to force the picker to, or `None` to keep whatever detection settled on.
///
/// `ratatui-image` blacklists Kitty and Sixel outright whenever `KONSOLE_VERSION`
/// or `WEZTERM_EXECUTABLE` is set (its sixel path is buggy there and neither
/// terminal implements kitty placeholders), then reaches iTerm2 only through a
/// `TERM_PROGRAM` allowlist konsole isn't on. A blacklisted session therefore
/// lands on halfblocks with the deciding queries never sent, and renders the
/// cover as a ~30x16 mosaic. This puts back the protocol that evidence supports.
///
/// Gating on halfblocks keeps this a floor-raise rather than an override: a
/// protocol the terminal actually answered for outranks any guess, so lifting
/// the upstream blacklist retires this on its own.
fn protocol_override(detected: ProtocolType, outer: Outer) -> Option<ProtocolType> {
    if detected != ProtocolType::Halfblocks {
        return None;
    }
    match outer {
        Outer::KittyPlaceholders => Some(ProtocolType::Kitty),
        Outer::Iterm2Only => Some(ProtocolType::Iterm2),
        Outer::Unknown => None,
    }
}

/// Who the outer terminal is, given a `termname` sourced as [`outer_termname`]
/// does for this `host`.
fn resolve_outer(termname: Option<&str>, konsole_env: bool, host: Host) -> Outer {
    if host == Host::Opaque {
        return Outer::Unknown;
    }
    if let Some(outer) = termname.and_then(outer_from_termname) {
        return outer;
    }
    // KONSOLE_VERSION is inherited by every descendant, tmux panes included, so
    // it describes the terminal in front of us only when nothing sits between.
    // It stays the signal despite that because konsole is unidentifiable by
    // TERM, which it reports as a plain xterm-256color.
    match host {
        Host::Direct if konsole_env => Outer::Iterm2Only,
        _ => Outer::Unknown,
    }
}

/// kitty and ghostty are the terminals whose kitty-graphics support includes the
/// unicode placeholders `ratatui-image` emits. wezterm and konsole implement
/// kitty without them and are left out on purpose.
fn outer_from_termname(termname: &str) -> Option<Outer> {
    matches!(termname, "xterm-kitty" | "xterm-ghostty" | "ghostty")
        .then_some(Outer::KittyPlaceholders)
}

/// Where to read the outer terminal's `TERM` from, given what sits in between.
fn outer_termname(host: Host) -> Option<String> {
    match host {
        Host::Direct => std::env::var("TERM").ok(),
        Host::Passthrough => tmux_client_termname(),
        Host::Opaque => None,
    }
}

/// Inside tmux the process environment names whichever terminal started the
/// *server*, which need not be the attached client. Only tmux knows the client.
fn tmux_client_termname() -> Option<String> {
    let output = std::process::Command::new("tmux")
        .args(["display-message", "-p", "#{client_termname}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!name.is_empty()).then_some(name)
}

fn detect_host() -> Host {
    classify_host(
        std::env::var_os("TMUX").is_some(),
        std::env::var_os("STY").is_some(),
        std::env::var("TERM").ok().as_deref(),
        std::env::var("TERM_PROGRAM").ok().as_deref(),
    )
}

/// `$TMUX`/`$STY` are set by the multiplexer itself in every pane, so they beat
/// `TERM`, which a `default-terminal` override can rename to anything.
fn classify_host(
    in_tmux: bool,
    in_screen: bool,
    term: Option<&str>,
    term_program: Option<&str>,
) -> Host {
    // Screen wins when both are set: whichever nests inside the other, the
    // escapes still have to cross screen, which has no passthrough of its own
    // and eats them. A stale `$STY` costs a mosaic, a missed one costs the
    // whole image.
    if in_screen {
        return Host::Opaque;
    }
    if in_tmux {
        // `ratatui-image` decides on passthrough wrapping from TERM/TERM_PROGRAM
        // alone, never `$TMUX`, so a `default-terminal` override on a tmux too
        // old to set TERM_PROGRAM leaves it emitting raw escapes for tmux to
        // swallow. Forcing a pixel protocol into that renders nothing at all.
        let wrapped =
            term.is_some_and(|term| term.starts_with("tmux")) || term_program == Some("tmux");
        return if wrapped {
            Host::Passthrough
        } else {
            Host::Opaque
        };
    }
    // A multiplexer whose $TMUX/$STY got scrubbed still announces itself here.
    match term {
        Some(term) if term.starts_with("tmux") || term.starts_with("screen") => Host::Opaque,
        _ => Host::Direct,
    }
}

/// Keyed off the same env var, and the same non-empty test, that
/// `ratatui-image`'s own blacklist reads.
fn is_konsole_env() -> bool {
    std::env::var("KONSOLE_VERSION").is_ok_and(|version| !version.is_empty())
}

/// One event off any of the runtime's channels, so a whole queued batch can go
/// through the same handler between two renders.
enum LoopEvent {
    Download(DownloadEvent),
    Updates(UpdatesEvent),
    Auth(AuthEvent),
    Input(InputEvent),
    HomeResolve(HomeResolveEvent),
    HomeSearch(HomeSearchEvent),
    HomeFilter(HomeFilterEvent),
    HomeEnrich(EnrichEvent),
    HomeDetails(HomeDetailsEvent),
    HomeSize(HomeSizeEvent),
    HomeCover(HomeCoverEvent),
    MirrorProbe(MirrorProbeEvent),
    Update(UpdateEvent),
}

/// The first event already sitting in any of the listed receivers, polled in the
/// order given (the `select!`'s own arm order) and never awaiting. `None` when
/// the whole set is momentarily empty, which is the loop's cue to render.
macro_rules! next_queued {
    ($($rx:ident => $variant:ident),+ $(,)?) => {{
        let mut queued: Option<LoopEvent> = None;
        $(
            if queued.is_none()
                && let Ok(event) = $rx.try_recv()
            {
                queued = Some(LoopEvent::$variant(event));
            }
        )+
        queued
    }};
}

pub async fn run(
    config: Config,
    startup_notice: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting application runtime loop");
    apply_theme(config.display.theme);
    let auto_update = config.update.auto_update;
    let update_prereleases = config.update.prereleases;
    let validation_issue = config.validate().err().map(|e| e.to_string());
    let mut terminal = setup_terminal()?;
    // Guarantees the extra terminal escapes are reversed (+ ratatui restore) on
    // every exit path below, including the `render_frame(..)?` early-returns
    // that would otherwise skip the teardown tail. `DefaultTerminal` has no
    // restoring Drop of its own, so this guard is the single teardown site.
    let _terminal_guard = TerminalGuard;
    // Query the terminal's graphics protocol + font size AFTER raw mode + the alt
    // screen are live, and BEFORE the input thread starts. `from_query_stdio`
    // toggles its own termios and restores it to whatever it found: run before
    // `setup_terminal` it saves+restores COOKED mode, which then defeats
    // crossterm's raw-mode enable and leaves all keyboard input echoing/dead;
    // run here it saves+restores RAW, leaving input intact. It must also precede
    // `spawn_input_thread` so the query's escape replies aren't eaten off stdin
    // by the event loop. Falls back to halfblocks (needs no graphics protocol).
    let cover_picker = query_cover_picker();
    let mut app = App::new(config);
    // The disk-backed history store attaches here (not in `App::new`) so tests
    // constructing an `App` never read or write the user's real history file.
    app.history = super::download_history::DownloadHistory::load();
    // Swap the test-safe halfblocks default for the queried picker (same reason
    // the history store attaches here: `App::new` must never touch the terminal).
    app.covers.picker = cover_picker;
    if let Some(msg) = validation_issue {
        warn!(error = %msg, "Configuration validation failed; surfacing to UI");
        app.toast_err(msg);
    }
    if let Some(message) = startup_notice {
        app.toast_info(message);
    }

    let (download_tx, mut download_rx) = mpsc::unbounded_channel::<DownloadEvent>();
    let (updates_tx, mut updates_rx) = mpsc::unbounded_channel::<UpdatesEvent>();
    let (auth_tx, mut auth_rx) = mpsc::unbounded_channel::<AuthEvent>();
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<InputEvent>();
    let (home_resolve_tx, mut home_resolve_rx) = mpsc::unbounded_channel::<HomeResolveEvent>();
    let (home_search_tx, mut home_search_rx) = mpsc::unbounded_channel::<HomeSearchEvent>();
    let (home_filter_tx, mut home_filter_rx) = mpsc::unbounded_channel::<HomeFilterEvent>();
    let (home_enrich_tx, mut home_enrich_rx) = mpsc::unbounded_channel::<EnrichEvent>();
    let (home_details_tx, mut home_details_rx) = mpsc::unbounded_channel::<HomeDetailsEvent>();
    let (home_size_tx, mut home_size_rx) = mpsc::unbounded_channel::<HomeSizeEvent>();
    let (home_cover_tx, mut home_cover_rx) = mpsc::unbounded_channel::<HomeCoverEvent>();
    let (mirror_probe_tx, mut mirror_probe_rx) = mpsc::unbounded_channel::<MirrorProbeEvent>();
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<UpdateEvent>();
    let input_handle = spawn_input_thread(input_tx.clone());

    let mut should_quit = false;
    let mut active_downloads: HashMap<DownloadId, DownloadHandle> = HashMap::new();
    let mut tasks = BackgroundTasks {
        login: None,
        resolve: None,
        resolve_cancel: None,
        home_resolve_tx,
        search: None,
        search_cancel: None,
        home_search_tx,
        filter: None,
        filter_cancel: None,
        home_filter_tx,
        enrich_find: None,
        enrich_collection: None,
        enrich_update: None,
        home_enrich_tx,
        home_details_tx,
        home_size_tx,
        home_cover_tx,
        mirror_probe: None,
        mirror_probe_cancel: None,
        mirror_probe_tx: mirror_probe_tx.clone(),
        update_apply: None,
    };

    // Home-tab startup work: probe mirror latency, and resolve the pre-filled
    // collection value (restored from the last run) so its status shows without
    // the user touching the field. `schedule_resolve` parses + debounces, so a
    // non-parseable prefill just clears.
    if app.active_tab == Tab::Home {
        schedule_probe(
            &mut tasks.mirror_probe,
            &mut tasks.mirror_probe_cancel,
            &tasks.mirror_probe_tx,
        );
        if !app.home.collection.value.trim().is_empty() {
            let generation = app.home.supersede_resolve();
            schedule_resolve(
                &app.home.collection.value,
                generation,
                &mut tasks.resolve,
                &mut tasks.resolve_cancel,
                &tasks.home_resolve_tx,
            );
        }
    }

    // A session that was already logged in at startup carries whatever supporter
    // answer its last login wrote — including none at all, for a token stored
    // before the flag existed. Re-probe in the background so the supporter-gated
    // rows appear without a logout/login round trip.
    if app.config.login_state == AuthLoginState::LoggedIn {
        spawn_supporter_refresh_task(auth_tx.clone());
    }

    // Background self-update check. Auto mode downloads+applies; notify mode only
    // surfaces availability (header indicator + `u` modal). `update_tx` is kept
    // for the user-confirmed apply path.
    spawn_update_check(update_tx.clone(), auto_update, update_prereleases);

    while !should_quit {
        render_frame(&mut terminal, &app)?;

        let event = tokio::select! {
            Some(event) = download_rx.recv() => LoopEvent::Download(event),
            Some(event) = updates_rx.recv() => LoopEvent::Updates(event),
            Some(event) = auth_rx.recv() => LoopEvent::Auth(event),
            Some(event) = input_rx.recv() => LoopEvent::Input(event),
            Some(event) = home_resolve_rx.recv() => LoopEvent::HomeResolve(event),
            Some(event) = home_search_rx.recv() => LoopEvent::HomeSearch(event),
            Some(event) = home_filter_rx.recv() => LoopEvent::HomeFilter(event),
            Some(event) = home_enrich_rx.recv() => LoopEvent::HomeEnrich(event),
            Some(event) = home_details_rx.recv() => LoopEvent::HomeDetails(event),
            Some(event) = home_size_rx.recv() => LoopEvent::HomeSize(event),
            Some(event) = home_cover_rx.recv() => LoopEvent::HomeCover(event),
            Some(event) = mirror_probe_rx.recv() => LoopEvent::MirrorProbe(event),
            Some(event) = update_rx.recv() => LoopEvent::Update(event),
            else => break,
        };

        should_quit = apply_event(
            event,
            &mut app,
            &download_tx,
            &updates_tx,
            &auth_tx,
            &update_tx,
            &mut active_downloads,
            &mut tasks,
        );

        // Everything else already queued goes through the same handlers before
        // the next frame. A key repeat outpacing the frame cost would otherwise
        // buy a frame per event off unbounded channels, and the backlog keeps
        // the list moving after the key is released.
        while !should_quit {
            let Some(event) = next_queued!(
                download_rx => Download,
                updates_rx => Updates,
                auth_rx => Auth,
                input_rx => Input,
                home_resolve_rx => HomeResolve,
                home_search_rx => HomeSearch,
                home_filter_rx => HomeFilter,
                home_enrich_rx => HomeEnrich,
                home_details_rx => HomeDetails,
                home_size_rx => HomeSize,
                home_cover_rx => HomeCover,
                mirror_probe_rx => MirrorProbe,
                update_rx => Update,
            ) else {
                break;
            };
            should_quit = apply_event(
                event,
                &mut app,
                &download_tx,
                &updates_tx,
                &auth_tx,
                &update_tx,
                &mut active_downloads,
                &mut tasks,
            );
        }
    }

    if let Some(handle) = tasks.login.take() {
        handle.abort();
    }
    if let Some(handle) = tasks.resolve.take() {
        handle.abort();
    }
    if let Some(handle) = tasks.search.take() {
        handle.abort();
    }
    if let Some(handle) = tasks.filter.take() {
        handle.abort();
    }
    if let Some(handle) = tasks.enrich_find.take() {
        handle.abort();
    }
    if let Some(handle) = tasks.enrich_collection.take() {
        handle.abort();
    }
    if let Some(handle) = tasks.enrich_update.take() {
        handle.abort();
    }
    if let Some(handle) = tasks.mirror_probe.take() {
        handle.abort();
    }

    // Apply download events still queued when the loop broke — a run that
    // finished in the quit window would otherwise flush with its last-seen
    // in-flight stage and be misrecorded as cancelled.
    while let Ok(event) = download_rx.try_recv() {
        app.handle_download_event(event);
    }
    // Record every still-retained run (settled pages promote their pending
    // records; aborted in-flight runs record as cancelled) before the pages
    // drop with the process.
    app.flush_history_on_exit();

    app.home.quit_prompt = false;
    app.toast_info("quitting…");
    render_frame(&mut terminal, &app)?;

    drop(download_rx);
    drop(updates_rx);
    drop(input_rx);
    signal_abort_downloads(&mut active_downloads);
    abort_and_wait_downloads(&mut active_downloads).await;

    drop(input_tx);
    if let Some(handle) = input_handle {
        let _ = handle.join();
    }
    // Terminal teardown is owned by `_terminal_guard` (dropped on return); no
    // explicit cleanup here keeps it in exactly one place across all exit paths.

    Ok(())
}

/// Apply one [`LoopEvent`], returning `true` only when the app should quit.
/// Every arm is the loop's own handling for that channel; the loop calls it both
/// for the event `select!` woke on and for each one already queued behind it.
#[allow(clippy::too_many_arguments)]
fn apply_event(
    event: LoopEvent,
    app: &mut App,
    download_tx: &mpsc::UnboundedSender<DownloadEvent>,
    updates_tx: &mpsc::UnboundedSender<UpdatesEvent>,
    auth_tx: &mpsc::UnboundedSender<AuthEvent>,
    update_tx: &mpsc::UnboundedSender<UpdateEvent>,
    downloads: &mut HashMap<DownloadId, DownloadHandle>,
    tasks: &mut BackgroundTasks,
) -> bool {
    match event {
        LoopEvent::Download(event) => {
            trace!(?event, "Received download event");
            if let Some(completed_id) = download_finished_id(&event) {
                debug!(
                    download_id = completed_id,
                    "Download handle finished; awaiting join"
                );
                if let Some(handle) = downloads.remove(&completed_id) {
                    tokio::spawn(async move {
                        handle.wait().await;
                    });
                }
            }
            app.handle_download_event(event);
        }
        LoopEvent::Updates(event) => {
            trace!(?event, "Received updates event");
            // The handler may hand back a follow-up (the auto-fetch of the
            // missing-set enrichment's first page after a scan lands); run it
            // through the same dispatch, mirroring the search/filter arms.
            let follow_up = handle_updates_event(event, app, updates_tx);
            return dispatch_command(
                follow_up,
                app,
                download_tx,
                updates_tx,
                auth_tx,
                update_tx,
                downloads,
                tasks,
            );
        }
        LoopEvent::Auth(event) => {
            trace!(?event, "Received auth event");
            // Clear the stored handle once its task reports completion.
            // Reissue + logout are fire-and-forget (never stored), so a
            // queued ReissueComplete must not wipe a live login/verify handle.
            if matches!(
                event,
                AuthEvent::LazerLoginComplete(_) | AuthEvent::VerificationComplete(_)
            ) {
                tasks.login = None;
            }
            handle_auth_event(event, app);
        }
        LoopEvent::Input(input) => {
            trace!(?input, "Processing input event");
            return handle_input(
                input,
                app,
                download_tx,
                updates_tx,
                auth_tx,
                update_tx,
                downloads,
                tasks,
            );
        }
        LoopEvent::HomeResolve(event) => {
            trace!(?event, "Received home resolve event");
            handle_home_resolve_event(event, &mut app.home);
        }
        LoopEvent::HomeSearch(event) => {
            trace!(?event, "Received home search event");
            // The handler may hand back a follow-up (a size probe of the
            // checked osu results); run it through the same dispatch.
            let follow_up = handle_home_search_event(event, app);
            return dispatch_command(
                follow_up,
                app,
                download_tx,
                updates_tx,
                auth_tx,
                update_tx,
                downloads,
                tasks,
            );
        }
        LoopEvent::HomeFilter(event) => {
            trace!(?event, "Received home filter event");
            // The handler may hand back a follow-up command (the auto-fetch
            // of the first details page); run it through the same dispatch.
            let follow_up = handle_home_filter_event(event, app);
            return dispatch_command(
                follow_up,
                app,
                download_tx,
                updates_tx,
                auth_tx,
                update_tx,
                downloads,
                tasks,
            );
        }
        LoopEvent::HomeEnrich(event) => {
            trace!(?event, "Received home enrich event");
            handle_enrich_event(event, app);
        }
        LoopEvent::HomeDetails(event) => {
            trace!(?event, "Received home details event");
            // The handler may hand back a follow-up (the osu-batch page for
            // the seeds a landed details page just derived); run it through
            // the same dispatch.
            let follow_up = handle_home_details_event(event, app);
            return dispatch_command(
                follow_up,
                app,
                download_tx,
                updates_tx,
                auth_tx,
                update_tx,
                downloads,
                tasks,
            );
        }
        LoopEvent::HomeSize(event) => {
            trace!(?event, "Received home size event");
            handle_home_size_event(event, &mut app.home.find);
        }
        LoopEvent::HomeCover(event) => {
            trace!(?event, "Received home cover event");
            handle_home_cover_event(event, app);
        }
        LoopEvent::MirrorProbe(event) => {
            trace!(?event, "Received mirror probe event");
            handle_mirror_probe_event(event, &mut app.home);
        }
        LoopEvent::Update(event) => {
            trace!(?event, "Received update event");
            handle_update_event(event, app);
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn handle_input(
    input: InputEvent,
    app: &mut App,
    download_tx: &mpsc::UnboundedSender<DownloadEvent>,
    updates_tx: &mpsc::UnboundedSender<UpdatesEvent>,
    auth_tx: &mpsc::UnboundedSender<AuthEvent>,
    update_tx: &mpsc::UnboundedSender<UpdateEvent>,
    downloads: &mut HashMap<DownloadId, DownloadHandle>,
    tasks: &mut BackgroundTasks,
) -> bool {
    match input {
        InputEvent::Key(key) => handle_key_event(
            key,
            app,
            download_tx,
            updates_tx,
            auth_tx,
            update_tx,
            downloads,
            tasks,
        ),
        InputEvent::Paste(text) => {
            let cmd = app.handle_paste(text);
            dispatch_command(
                cmd,
                app,
                download_tx,
                updates_tx,
                auth_tx,
                update_tx,
                downloads,
                tasks,
            )
        }
        InputEvent::Resize => false,
        InputEvent::Tick => {
            app.on_tick();
            // Debounced cover prefetch for the highlighted set-browse row; the
            // returned command (if any) rides the shared dispatch, which owns
            // the fetch channel.
            let cmd = app.poll_cover_prefetch();
            dispatch_command(
                cmd,
                app,
                download_tx,
                updates_tx,
                auth_tx,
                update_tx,
                downloads,
                tasks,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_key_event(
    key: KeyEvent,
    app: &mut App,
    download_tx: &mpsc::UnboundedSender<DownloadEvent>,
    updates_tx: &mpsc::UnboundedSender<UpdatesEvent>,
    auth_tx: &mpsc::UnboundedSender<AuthEvent>,
    update_tx: &mpsc::UnboundedSender<UpdateEvent>,
    downloads: &mut HashMap<DownloadId, DownloadHandle>,
    tasks: &mut BackgroundTasks,
) -> bool {
    trace!(?key, "Handling key event");
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        warn!("CTRL+C received; signalling abort for all downloads");
        signal_abort_downloads(downloads);
        return true;
    }

    let cmd = app.handle_key(key);
    dispatch_command(
        cmd,
        app,
        download_tx,
        updates_tx,
        auth_tx,
        update_tx,
        downloads,
        tasks,
    )
}

/// Run the side effects for an [`AppCommand`] produced by a key or paste event,
/// returning `true` only when the app should quit. Shared by the key and paste
/// input paths so both dispatch identically.
#[allow(clippy::too_many_arguments)]
fn dispatch_command(
    cmd: Option<AppCommand>,
    app: &mut App,
    download_tx: &mpsc::UnboundedSender<DownloadEvent>,
    updates_tx: &mpsc::UnboundedSender<UpdatesEvent>,
    auth_tx: &mpsc::UnboundedSender<AuthEvent>,
    update_tx: &mpsc::UnboundedSender<UpdateEvent>,
    downloads: &mut HashMap<DownloadId, DownloadHandle>,
    tasks: &mut BackgroundTasks,
) -> bool {
    match cmd {
        Some(AppCommand::StartDownload { id, request }) => {
            let handle = download::spawn_download(id, request, download_tx.clone());
            info!(download_id = id, "Spawned download from UI request");
            downloads.insert(id, handle);
        }
        Some(AppCommand::StartSelectiveDownload { id, request }) => {
            let handle = download::spawn_selective_download(id, request, download_tx.clone());
            info!(
                download_id = id,
                "Spawned selective download from update source"
            );
            downloads.insert(id, handle);
        }
        Some(AppCommand::StartIdsDownload { id, request }) => {
            let source = request.source;
            let handle = download::spawn_ids_download(id, request, download_tx.clone());
            info!(
                download_id = id,
                ?source,
                "Spawned raw-ids download from get-maps source"
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
        Some(AppCommand::DeferRateLimited { id }) => {
            if let Some(handle) = downloads.get(&id) {
                handle.defer_rate_limited();
                info!(download_id = id, "Deferring rate-limited maps");
            }
        }
        Some(AppCommand::SkipRateLimited { id }) => {
            if let Some(handle) = downloads.get(&id) {
                handle.skip_rate_limited();
                info!(download_id = id, "Dropping rate-limited maps");
            }
        }
        Some(AppCommand::LazerLogin { username, password }) => {
            if let Some(prev) = tasks.login.take() {
                prev.abort();
            }
            tasks.login = Some(spawn_lazer_login_task(username, password, auth_tx.clone()));
        }
        Some(AppCommand::SubmitVerification { code }) => {
            if let Some(prev) = tasks.login.take() {
                prev.abort();
            }
            tasks.login = Some(spawn_verification_task(code, auth_tx.clone()));
        }
        Some(AppCommand::ReissueVerification) => {
            // Fire-and-forget: do not occupy `tasks.login`, so it can't clobber
            // or be clobbered by an in-flight login / verify handle.
            spawn_reissue_task(auth_tx.clone());
        }
        Some(AppCommand::CancelLogin) => {
            if let Some(handle) = tasks.login.take() {
                handle.abort();
                info!("Login cancelled by user");
            }
        }
        Some(AppCommand::Logout) => {
            spawn_logout_task(auth_tx.clone());
        }
        Some(AppCommand::CancelResolve) => {
            cancel_resolve(&mut tasks.resolve, &mut tasks.resolve_cancel);
        }
        Some(AppCommand::ResolveCollectionUrl { generation, value }) => {
            // Cancel then schedule, sequenced HERE rather than folded into
            // `schedule_resolve`: they are two effects, and the command that
            // wants only the first one is right above this.
            cancel_resolve(&mut tasks.resolve, &mut tasks.resolve_cancel);
            schedule_resolve(
                &value,
                generation,
                &mut tasks.resolve,
                &mut tasks.resolve_cancel,
                &tasks.home_resolve_tx,
            );
        }
        Some(AppCommand::RunSearch { query, append }) => {
            schedule_search(
                query,
                append,
                &mut tasks.search,
                &mut tasks.search_cancel,
                &tasks.home_search_tx,
            );
        }
        Some(AppCommand::RunFilter { query }) => {
            schedule_filter(
                query,
                &mut tasks.filter,
                &mut tasks.filter_cancel,
                &tasks.home_filter_tx,
            );
        }
        Some(AppCommand::LoadEnrichment { target }) => {
            // The nzbasic find target seeds its osu-batch pager from LANDED
            // details pages (the details response is the only place nzbasic
            // pairs a diff with its set), so a dispatch serves the pager first
            // when it holds unfetched seeds — the auto-follow a details
            // landing returns — and only otherwise advances the details walk
            // (the auto-fetch after results land, and `m`). Collection and
            // update never seed a walk: their seeds arrive pre-paired and page
            // directly, exactly as before.
            let is_nzbasic_find = target == EnrichTarget::Find
                && app.home.find.results_backend() == Some(FindBackend::Nzbasic);
            let enrich_handle = match target {
                EnrichTarget::Find => &mut tasks.enrich_find,
                EnrichTarget::Collection => &mut tasks.enrich_collection,
                EnrichTarget::Update => &mut tasks.enrich_update,
            };
            if is_nzbasic_find && !app.home.find.browse.has_unpaged_enrichment() {
                // Fire-and-forget, no handle: concurrent pages are disjoint id
                // slices under one generation, and the walk's in-flight counter
                // drives the shared loading cue.
                if let Some(page) = app.home.find.browse.next_details_page() {
                    app.home.find.browse.mark_details_dispatched();
                    let generation = app.home.find.browse.details_walk_generation();
                    schedule_details(generation, page, &tasks.home_details_tx);
                }
            } else {
                let sink = enrich_sink_mut(app, target);
                // The first page (cursor 0) follows a reseed: the reseed already
                // invalidated any in-flight page via the generation guard, so start
                // fresh (`schedule_enrichment` aborts the target's prior task). A
                // `m`-triggered page (cursor > 0) respects busy — aborting an
                // in-flight page AFTER the pager advanced past it would lose those
                // rows (rewind fires on a failure event, not on abort-by-supersede).
                let first_page = sink.enrich_cursor() == 0;
                let busy = enrich_handle
                    .as_ref()
                    .is_some_and(|handle| !handle.is_finished());
                if first_page || !busy {
                    let generation = sink.enrich_generation();
                    let rewind_to = sink.enrich_cursor();
                    // A dry pager (every page requested) makes this a no-op.
                    if let Some(page) = sink.next_enrich_page() {
                        // Count this page as outstanding; the landing / failing
                        // event decrements it (`handle_enrich_event`). A counter
                        // (not a bool) so an older page's late event can't clear
                        // the cue while a newer fetch is still pending.
                        sink.mark_enrichment_dispatched();
                        schedule_enrichment(
                            target,
                            generation,
                            page,
                            rewind_to,
                            enrich_handle,
                            &tasks.home_enrich_tx,
                        );
                    }
                }
            }
        }
        Some(AppCommand::ProbeMirrors) => {
            schedule_probe(
                &mut tasks.mirror_probe,
                &mut tasks.mirror_probe_cancel,
                &tasks.mirror_probe_tx,
            );
        }
        Some(AppCommand::ProbeFindSizes) => {
            // Claim marks the newly-checked, un-probed osu ids `Pending`, so a
            // burst of toggles never double-fetches; the spawn no-ops on empty.
            let ids = app.home.find.claim_size_probes();
            schedule_size_probe(ids, &tasks.home_size_tx);
        }
        Some(AppCommand::FetchCover { set_id }) => {
            // The prefetch already marked the id `Pending`, so the same
            // highlighted row never spawns twice.
            schedule_cover_fetch(set_id, &tasks.home_cover_tx);
        }
        Some(AppCommand::ScanLocalDatabase) => {
            spawn_scan_task(app, updates_tx.clone());
        }
        Some(AppCommand::RecheckFailedMaps) => {
            spawn_failed_map_recheck_task(app, updates_tx.clone());
        }
        Some(AppCommand::RetryAllFailed { download_id }) => {
            let retryable_ids = app
                .downloads
                .iter()
                .find(|p| p.id == download_id)
                .map(|p| p.retryable_ids(None))
                .unwrap_or_default();
            if !retryable_ids.is_empty()
                && let Some((new_id, request)) =
                    app.start_retry_download(download_id, retryable_ids)
            {
                let handle = download::spawn_ids_download(new_id, request, download_tx.clone());
                info!(
                    source_download_id = download_id,
                    retry_download_id = new_id,
                    "Spawned retry-all download"
                );
                downloads.insert(new_id, handle);
            }
        }
        Some(AppCommand::FocusOutputDir) => {
            app.focus_output_dir();
        }
        Some(AppCommand::StartUpdate) => {
            info!("User confirmed update; downloading and applying");
            tasks.update_apply = Some(spawn_apply_update(
                update_tx.clone(),
                app.config.prereleases,
            ));
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

#[cfg(test)]
#[path = "../../../tests/unit/runtime_picker.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/runtime_loop.rs"]
mod loop_tests;

#[cfg(test)]
#[path = "../../../tests/unit/runtime_dispatch.rs"]
mod dispatch_tests;

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
    /// A bracketed-paste payload to route into the focused text field.
    Paste(String),
    Resize,
    Tick,
}

/// Background task handles and their associated channels, kept by the runtime loop.
struct BackgroundTasks {
    login: Option<tokio::task::JoinHandle<()>>,
    resolve: Option<tokio::task::JoinHandle<()>>,
    resolve_cancel: Option<tokio::sync::watch::Sender<bool>>,
    home_resolve_tx: mpsc::UnboundedSender<HomeResolveEvent>,
    search: Option<tokio::task::JoinHandle<()>>,
    search_cancel: Option<tokio::sync::watch::Sender<bool>>,
    home_search_tx: mpsc::UnboundedSender<HomeSearchEvent>,
    filter: Option<tokio::task::JoinHandle<()>>,
    filter_cancel: Option<tokio::sync::watch::Sender<bool>>,
    home_filter_tx: mpsc::UnboundedSender<HomeFilterEvent>,
    /// In-flight osu-batch enrichment page tasks, one slot per target browse
    /// (one page per target; a find first page force-schedules and must never
    /// abort an in-flight collection page, or vice versa — an aborted task
    /// fires no event, so its rows would stay id-only with the cursor advanced).
    enrich_find: Option<tokio::task::JoinHandle<()>>,
    enrich_collection: Option<tokio::task::JoinHandle<()>>,
    enrich_update: Option<tokio::task::JoinHandle<()>>,
    home_enrich_tx: mpsc::UnboundedSender<EnrichEvent>,
    /// nzbasic details channel. Fire-and-forget (no stored handle) — a page left
    /// running at quit just finds the receiver dropped, and a reseed's stale page
    /// is dropped by the generation guard, so nothing needs an abort.
    home_details_tx: mpsc::UnboundedSender<HomeDetailsEvent>,
    /// Nekoha size-probe channel. Fire-and-forget (no stored handle) — a probe
    /// left running at quit just finds the receiver dropped and ends.
    home_size_tx: mpsc::UnboundedSender<HomeSizeEvent>,
    /// Cover-image fetch channel. Fire-and-forget (no stored handle) — a fetch
    /// left running at quit just finds the receiver dropped and ends.
    home_cover_tx: mpsc::UnboundedSender<HomeCoverEvent>,
    mirror_probe: Option<tokio::task::JoinHandle<()>>,
    mirror_probe_cancel: Option<tokio::sync::watch::Sender<bool>>,
    mirror_probe_tx: mpsc::UnboundedSender<MirrorProbeEvent>,
    /// Handle for the apply-update task spawned by [`AppCommand::StartUpdate`].
    /// Fire-and-forget in practice, but stored so the dispatch-arm test can
    /// assert the spawn happened.
    update_apply: Option<tokio::task::JoinHandle<()>>,
}
