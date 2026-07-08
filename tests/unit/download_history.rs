//! `download_history.json` store tests: settle-then-drop record flow, the
//! cancel path, the ring cap, and restart survival (reload from disk).

use super::{DownloadHistory, HISTORY_CAP, HistoryStage};
use crate::app::collection::CollectionPage;
use crate::download::DownloadStage;
use std::path::PathBuf;

fn tmp_store() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("download_history.json");
    (dir, path)
}

fn page(id: u64, stage: DownloadStage) -> CollectionPage {
    let mut page = CollectionPage::new(id, format!("run {id}"), 2);
    page.stage = stage;
    page.total_maps = 10;
    page.stats.downloaded = 7;
    page.stats.skipped = 2;
    page.stats.failed = 1;
    page.output_dir = Some("/tmp/out".to_string());
    page
}

#[test]
fn settled_record_is_persisted_but_hidden_while_the_page_lives() {
    let (_dir, path) = tmp_store();
    let mut history = DownloadHistory::load_from(Some(path.clone()));

    history.record_settled(&page(1, DownloadStage::Completed));

    // Hidden from the list (the live page is the visible row)…
    assert!(history.records.is_empty());
    // …but already on disk, so a crash can't lose it.
    let reloaded = DownloadHistory::load_from(Some(path));
    assert_eq!(reloaded.records.len(), 1);
    assert_eq!(reloaded.records[0].stage, HistoryStage::Finished);
    assert_eq!(reloaded.records[0].downloaded, 7);
}

#[test]
fn removal_promotes_the_pending_record_without_duplicating() {
    let (_dir, path) = tmp_store();
    let mut history = DownloadHistory::load_from(Some(path.clone()));
    let settled = page(1, DownloadStage::Failed);

    history.record_settled(&settled);
    history.record_removed(&settled);

    assert_eq!(history.records.len(), 1, "one record per run, no dup");
    assert_eq!(history.records[0].stage, HistoryStage::Failed);
    let reloaded = DownloadHistory::load_from(Some(path));
    assert_eq!(reloaded.records.len(), 1);
}

#[test]
fn a_never_settled_page_records_as_cancelled() {
    let (_dir, path) = tmp_store();
    let mut history = DownloadHistory::load_from(Some(path.clone()));

    history.record_removed(&page(3, DownloadStage::Downloading));

    assert_eq!(history.records[0].stage, HistoryStage::Cancelled);
    assert_eq!(history.records[0].title, "run 3");
    // Survives a restart.
    let reloaded = DownloadHistory::load_from(Some(path));
    assert_eq!(reloaded.records[0].stage, HistoryStage::Cancelled);
}

#[test]
fn a_late_settle_refreshes_the_pending_record_in_place() {
    let (_dir, path) = tmp_store();
    let mut history = DownloadHistory::load_from(Some(path));
    let mut run = page(1, DownloadStage::Failed);

    history.record_settled(&run);
    run.stats.downloaded = 9;
    run.stage = DownloadStage::Completed;
    history.record_settled(&run);
    history.record_removed(&run);

    assert_eq!(history.records.len(), 1, "re-settle must not duplicate");
    assert_eq!(history.records[0].stage, HistoryStage::Finished);
    assert_eq!(history.records[0].downloaded, 9);
}

#[test]
fn ring_caps_at_fifty_newest_first() {
    let (_dir, path) = tmp_store();
    let mut history = DownloadHistory::load_from(Some(path.clone()));

    for id in 0..(HISTORY_CAP as u64 + 5) {
        history.record_removed(&page(id, DownloadStage::Completed));
    }

    assert_eq!(history.records.len(), HISTORY_CAP);
    assert_eq!(
        history.records[0].title,
        format!("run {}", HISTORY_CAP as u64 + 4),
        "newest record leads the ring"
    );
    let reloaded = DownloadHistory::load_from(Some(path));
    assert_eq!(reloaded.records.len(), HISTORY_CAP);
}

#[test]
fn corrupt_file_loads_as_empty() {
    let (_dir, path) = tmp_store();
    std::fs::write(&path, "{ not json").unwrap();

    let history = DownloadHistory::load_from(Some(path));

    assert!(history.records.is_empty());
}
