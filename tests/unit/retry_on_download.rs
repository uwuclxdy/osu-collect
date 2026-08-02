//! Pre-download retry-failed prompt — config tri-state + modal flow.

use crate::{
    app::{
        App, AppCommand, ConfigField, ConfigTab, Tab,
        failed_maps::{FailedMapsFile, save},
    },
    config::{Config, RetryFailedOnDownload},
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use std::collections::HashSet;
use tempfile::TempDir;

const COLLECTION_ID: u32 = 1234;
const FAILED_SET_IDS: [u32; 3] = [10, 20, 30];
const COLLECTION_BEATMAPSET_IDS: [u32; 4] = [10, 20, 99, 100];

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

/// Build an `App` whose collection field is primed with a numeric ID and
/// whose home cache already holds resolved beatmapset ids. Writes a
/// `failed-beatmapsets.json` with two of those ids into a tempdir and points
/// the app at it via `failed_maps_path_override`.
///
/// The tempdir also hosts the config file, and a `TempEnvVar` guard redirects
/// `OSU_COLLECT_CONFIG` at it for the helper's lifetime. `request_download`
/// persists recent inputs by calling `save_config`, which resolves that env var
/// — without the guard, a parallel `config_theme` test can see its own temp
/// config overwritten. Returned alongside `(App, TempDir)` so the caller binds
/// the guard for as long as the app is used.
fn app_with_failed_maps(
    mode: RetryFailedOnDownload,
) -> (App, TempDir, crate::test_env::TempEnvVar) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("config.toml");
    let env = crate::test_env::TempEnvVar::set(
        "OSU_COLLECT_CONFIG",
        config_path.to_str().expect("config path is utf-8"),
    );

    let mut app = App::new(Config::default());
    app.config.retry_failed_on_download = mode;
    app.home.collection.value = COLLECTION_ID.to_string();
    // Ensure the form passes build_request: any directory is fine, mirrors
    // already on by default.
    app.home.directory.value = "/tmp/osu-collect-test".to_string();
    app.home
        .set_resolved_collection(COLLECTION_ID, COLLECTION_BEATMAPSET_IDS.to_vec());

    let path = dir.path().join("failed-beatmapsets.json");
    save(
        &FailedMapsFile {
            schema_version: 1,
            beatmapset_ids: FAILED_SET_IDS.to_vec(),
        },
        &path,
    );
    app.failed_maps_path_override = Some(path);
    app.active_tab = Tab::Home;
    (app, dir, env)
}

// ── config: cycle + serde ────────────────────────────────────────────────────

#[test]
fn cycle_retry_failed_on_download_cycles_ask_yes_no() {
    let mut tab = ConfigTab::new(&Config::default());
    tab.focus = ConfigField::RetryFailedOnDownload;
    assert_eq!(tab.retry_failed_on_download, RetryFailedOnDownload::Ask);

    tab.toggle_current();
    assert_eq!(tab.retry_failed_on_download, RetryFailedOnDownload::Yes);

    tab.toggle_current();
    assert_eq!(tab.retry_failed_on_download, RetryFailedOnDownload::No);

    tab.toggle_current();
    assert_eq!(tab.retry_failed_on_download, RetryFailedOnDownload::Ask);
}

#[test]
fn retry_failed_on_download_serde_roundtrip() {
    let toml_text = toml::to_string(&Config::default()).expect("serialize");
    let parsed: Config = toml::from_str(&toml_text).expect("parse");
    assert_eq!(
        parsed.download.retry_failed_on_download,
        RetryFailedOnDownload::Ask
    );

    let yes_text = "[download]\nretry_failed_on_download = \"yes\"\n";
    let parsed_yes: Config = toml::from_str(yes_text).expect("parse yes");
    assert_eq!(
        parsed_yes.download.retry_failed_on_download,
        RetryFailedOnDownload::Yes
    );
}

#[test]
fn old_config_without_retry_field_loads_with_ask_default() {
    // Old configs predating the field must still parse.
    let old_text = "[download]\nconcurrent = 4\nno_video = false\n";
    let parsed: Config = toml::from_str(old_text).expect("parse old");
    assert_eq!(
        parsed.download.retry_failed_on_download,
        RetryFailedOnDownload::Ask
    );
}

// ── intersect & request_download flow ────────────────────────────────────────

#[test]
fn yes_mode_skips_modal_and_includes_failed_ids() {
    let (mut app, _dir, _env) = app_with_failed_maps(RetryFailedOnDownload::Yes);

    let result = app.request_download();
    assert!(result.is_some(), "Yes mode must dispatch without a modal");
    let (_id, request) = result.unwrap();
    assert!(
        request.previously_failed_skipped.is_empty(),
        "Yes mode must exclude nothing, so the run retries every previously failed id"
    );
    assert!(
        app.confirm_retry_on_start.is_none(),
        "Yes mode must not open the modal"
    );
}

#[test]
fn no_mode_skips_modal_and_excludes_failed_ids() {
    let (mut app, _dir, _env) = app_with_failed_maps(RetryFailedOnDownload::No);

    let result = app.request_download();
    assert!(result.is_some(), "No mode must dispatch without a modal");
    let (_id, request) = result.unwrap();
    assert_eq!(
        request.previously_failed_skipped,
        HashSet::from([10, 20]),
        "No mode must hand the run exactly this collection's previously failed ids"
    );
    assert!(
        app.confirm_retry_on_start.is_none(),
        "No mode must not open the modal"
    );
}

#[test]
fn ask_mode_opens_modal_when_failures_intersect() {
    let (mut app, _dir, _env) = app_with_failed_maps(RetryFailedOnDownload::Ask);

    let result = app.request_download();
    assert!(
        result.is_none(),
        "Ask mode with intersecting failures must defer the dispatch"
    );
    let modal = app
        .confirm_retry_on_start
        .as_ref()
        .expect("modal must be open");
    assert_eq!(
        modal.previously_failed,
        HashSet::from([10, 20]),
        "intersection of [10,20,30] and [10,20,99,100] is {{10,20}}"
    );
}

#[test]
fn ask_mode_enter_dispatches_with_retry() {
    let (mut app, _dir, _env) = app_with_failed_maps(RetryFailedOnDownload::Ask);
    let _ = app.request_download();
    assert!(app.confirm_retry_on_start.is_some());

    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(
        app.confirm_retry_on_start.is_none(),
        "enter must close the modal"
    );
    let Some(AppCommand::StartDownload { request, .. }) = cmd else {
        panic!("enter must emit StartDownload, got {cmd:?}");
    };
    assert!(
        request.previously_failed_skipped.is_empty(),
        "enter (retry) must dispatch with nothing excluded from the run"
    );
}

#[test]
fn ask_mode_skip_button_dispatches_without_retry() {
    let (mut app, _dir, _env) = app_with_failed_maps(RetryFailedOnDownload::Ask);
    let _ = app.request_download();
    assert!(app.confirm_retry_on_start.is_some());

    // Buttons are [cancel, skip, retry] with retry default-focused; ← moves to
    // the `skip` button, then enter dispatches without retry.
    app.handle_key(press(KeyCode::Left));
    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(
        app.confirm_retry_on_start.is_none(),
        "activating skip must close the modal"
    );
    let Some(AppCommand::StartDownload { request, .. }) = cmd else {
        panic!("skip must emit StartDownload, got {cmd:?}");
    };
    assert_eq!(
        request.previously_failed_skipped,
        HashSet::from([10, 20]),
        "skip must dispatch handing the run exactly the ids the prompt counted"
    );
}

#[test]
fn ask_mode_cancel_button_discards_download() {
    let (mut app, _dir, _env) = app_with_failed_maps(RetryFailedOnDownload::Ask);
    let downloads_before = app.downloads.len();
    let _ = app.request_download();
    assert!(app.confirm_retry_on_start.is_some());

    // ← twice from the default `retry` lands on `cancel`; enter discards the
    // queued download (same effect as esc).
    app.handle_key(press(KeyCode::Left));
    app.handle_key(press(KeyCode::Left));
    let cmd = app.handle_key(press(KeyCode::Enter));
    assert!(cmd.is_none(), "cancel must not dispatch a command");
    assert!(
        app.confirm_retry_on_start.is_none(),
        "cancel must close the modal"
    );
    assert_eq!(
        app.downloads.len(),
        downloads_before,
        "cancel must not leave a queued page behind"
    );
}

#[test]
fn ask_mode_esc_cancels_download() {
    let (mut app, _dir, _env) = app_with_failed_maps(RetryFailedOnDownload::Ask);
    let downloads_before = app.downloads.len();
    let _ = app.request_download();
    assert!(app.confirm_retry_on_start.is_some());

    let cmd = app.handle_key(press(KeyCode::Esc));
    assert!(cmd.is_none(), "esc must not dispatch a command");
    assert!(
        app.confirm_retry_on_start.is_none(),
        "esc must close the modal"
    );
    assert_eq!(
        app.downloads.len(),
        downloads_before,
        "esc must not leave a queued page behind"
    );
}

#[test]
fn no_intersection_skips_modal_under_ask() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("config.toml");
    let _env = crate::test_env::TempEnvVar::set(
        "OSU_COLLECT_CONFIG",
        config_path.to_str().expect("config path is utf-8"),
    );

    let mut app = App::new(Config::default());
    app.config.retry_failed_on_download = RetryFailedOnDownload::Ask;
    app.home.collection.value = COLLECTION_ID.to_string();
    app.home.directory.value = "/tmp/osu-collect-test".to_string();
    // resolved ids do not overlap with persisted failures
    app.home
        .set_resolved_collection(COLLECTION_ID, vec![1, 2, 3]);

    let path = dir.path().join("failed-beatmapsets.json");
    save(
        &FailedMapsFile {
            schema_version: 1,
            beatmapset_ids: vec![100, 200],
        },
        &path,
    );
    app.failed_maps_path_override = Some(path);

    let result = app.request_download();
    assert!(
        result.is_some(),
        "no intersection must dispatch without a modal"
    );
    assert!(
        app.confirm_retry_on_start.is_none(),
        "no intersection must not open the modal"
    );
}
