//! Downloads-tab key behavior: descend/ascend, the esc download-control
//! scoping (cancel only a running preview), run retention after settling, and
//! the history hand-off on cancel. Replaces the per-run-tab close tests —
//! settled runs are retained on the list, never "closed".

use crate::{
    app::{App, AppCommand, HistoryStage, Tab, collection::CollectionPage},
    config::Config,
    download::{DownloadEvent, DownloadId, DownloadStage, DownloadSummary},
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

fn make_app() -> App {
    App::new(Config::default())
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

/// Push a page in the given stage and focus the Downloads tab on its row.
fn push_page(app: &mut App, id: DownloadId, stage: DownloadStage) {
    let mut page = CollectionPage::new(id, format!("col {id}"), 2);
    page.stage = stage;
    app.downloads.push(page);
    app.active_tab = Tab::Downloads;
    app.downloads_tab.selected = 0;
    app.downloads_tab.preview_focused = false;
}

// ── descend / ascend ──────────────────────────────────────────────────────────

#[test]
fn enter_on_list_descends_into_preview() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Downloading);

    let cmd = app.handle_key(press(KeyCode::Enter));

    assert!(cmd.is_none());
    assert!(app.downloads_tab.preview_focused, "enter must descend");
}

#[test]
fn enter_on_empty_list_does_not_descend() {
    let mut app = make_app();
    app.active_tab = Tab::Downloads;

    app.handle_key(press(KeyCode::Enter));

    assert!(!app.downloads_tab.preview_focused);
}

#[test]
fn left_ascends_from_preview_without_cancelling() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Downloading);
    app.downloads_tab.preview_focused = true;

    let cmd = app.handle_key(press(KeyCode::Left));

    assert!(cmd.is_none(), "← must never cancel");
    assert!(!app.downloads_tab.preview_focused, "← returns to the list");
    assert_eq!(app.active_tab, Tab::Downloads, "← must not switch tabs");
    assert_eq!(app.downloads.len(), 1);
}

#[test]
fn left_on_list_switches_tabs() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Downloading);

    app.handle_key(press(KeyCode::Left));

    assert_eq!(
        app.active_tab,
        Tab::Home,
        "list level keeps arrow tab-switching"
    );
}

// ── esc: the download-control key, preview-scoped ────────────────────────────

#[test]
fn esc_on_running_preview_ascends_without_cancelling() {
    let mut app = make_app();
    push_page(&mut app, 4, DownloadStage::Downloading);
    app.downloads_tab.preview_focused = true;

    let cmd = app.handle_key(press(KeyCode::Esc));

    assert!(
        cmd.is_none(),
        "esc must never cancel — that's `q`, got {cmd:?}"
    );
    assert!(
        !app.downloads_tab.preview_focused,
        "esc ascends to the list"
    );
    assert_eq!(app.downloads.len(), 1, "the run keeps running");
}

#[test]
fn esc_on_settled_preview_ascends_and_retains_the_run() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    app.downloads_tab.preview_focused = true;

    let cmd = app.handle_key(press(KeyCode::Esc));

    assert!(cmd.is_none(), "nothing to cancel on a settled run");
    assert!(!app.downloads_tab.preview_focused, "esc ascends instead");
    assert_eq!(app.downloads.len(), 1, "settled runs are retained");
}

#[test]
fn esc_on_list_does_not_cancel() {
    let mut app = make_app();
    push_page(&mut app, 4, DownloadStage::Downloading);

    let cmd = app.handle_key(press(KeyCode::Esc));

    assert!(cmd.is_none(), "esc is preview-scoped, got {cmd:?}");
    assert_eq!(app.downloads.len(), 1);
}

#[test]
fn q_on_running_preview_cancels_the_run() {
    let mut app = make_app();
    push_page(&mut app, 4, DownloadStage::Downloading);
    app.downloads_tab.preview_focused = true;

    let cmd = app.handle_key(press(KeyCode::Char('q')));

    assert!(
        matches!(cmd, Some(AppCommand::CancelDownload { id: 4 })),
        "q on a running preview requests cancellation, got {cmd:?}"
    );
    assert!(
        !app.home.quit_prompt,
        "q on a run cancels, it never arms quit"
    );
    assert_eq!(app.downloads.len(), 1, "page stays until the runtime acks");
}

#[test]
fn q_on_settled_preview_ascends_without_arming_quit() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    app.downloads_tab.preview_focused = true;

    let cmd = app.handle_key(press(KeyCode::Char('q')));

    assert!(cmd.is_none(), "nothing to cancel on a settled run");
    assert!(
        !app.downloads_tab.preview_focused,
        "q ascends the settled preview"
    );
    assert!(!app.home.quit_prompt, "ascending a preview never arms quit");
    assert_eq!(app.downloads.len(), 1, "settled runs are retained");
}

#[test]
fn q_on_downloads_list_arms_the_quit_prompt() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Downloading);
    // At the list level (not descended) q is the top-level 2-step quit.
    app.downloads_tab.preview_focused = false;

    let cmd = app.handle_key(press(KeyCode::Char('q')));

    assert!(cmd.is_none());
    assert!(app.home.quit_prompt, "list-level q arms the quit prompt");
    assert_eq!(app.downloads.len(), 1, "arming quit never closes a run");
}

// ── defer / skip stay preview-scoped ─────────────────────────────────────────

#[test]
fn s_on_list_does_not_defer() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Downloading);

    let cmd = app.handle_key(press(KeyCode::Char('s')));

    assert!(cmd.is_none(), "s acts only on the focused preview");
}

// ── cancel result → history record ───────────────────────────────────────────

#[test]
fn cancel_result_replaces_the_page_with_a_cancelled_record() {
    let mut app = make_app();
    push_page(&mut app, 4, DownloadStage::Downloading);
    app.downloads_tab.preview_focused = true;

    app.handle_cancel_result(4, true);

    assert!(app.downloads.is_empty(), "cancelled page must be removed");
    assert_eq!(app.history.records.len(), 1, "cancel must write a record");
    assert_eq!(app.history.records[0].stage, HistoryStage::Cancelled);
    assert!(
        !app.downloads_tab.preview_focused,
        "focus falls back to the list"
    );
    assert_eq!(app.active_tab, Tab::Downloads, "stay on the downloads tab");
}

// ── settling retains the page; the record surfaces only after removal ───────

#[test]
fn finished_event_retains_the_page_without_a_visible_record() {
    let mut app = make_app();
    push_page(&mut app, 7, DownloadStage::Downloading);

    app.handle_download_event(DownloadEvent::Finished {
        id: 7,
        summary: DownloadSummary {
            downloaded: 5,
            skipped: 0,
            failed: 0,
            unverified: 0,
        },
    });

    assert_eq!(app.downloads.len(), 1, "settled page is retained");
    assert_eq!(app.downloads[0].stage, DownloadStage::Completed);
    assert!(
        app.history.records.is_empty(),
        "the live page is the visible row; no duplicate record"
    );
}

#[test]
fn flush_on_exit_records_every_retained_run() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    let mut running = CollectionPage::new(2, "col 2".to_string(), 2);
    running.stage = DownloadStage::Downloading;
    app.downloads.push(running);
    app.handle_download_event(DownloadEvent::Finished {
        id: 1,
        summary: DownloadSummary {
            downloaded: 1,
            skipped: 0,
            failed: 0,
            unverified: 0,
        },
    });

    app.flush_history_on_exit();

    assert!(app.downloads.is_empty());
    let stages: Vec<HistoryStage> = app.history.records.iter().map(|r| r.stage).collect();
    assert!(stages.contains(&HistoryStage::Finished), "settled recorded");
    assert!(
        stages.contains(&HistoryStage::Cancelled),
        "aborted-in-flight run records as cancelled"
    );
}

// ── list rows: actives first, then past ──────────────────────────────────────

#[test]
fn rows_order_actives_before_settled_and_records() {
    use crate::app::DownloadsRow;
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    let mut running = CollectionPage::new(2, "col 2".to_string(), 2);
    running.stage = DownloadStage::Downloading;
    app.downloads.push(running);

    let rows = app.downloads_rows();

    assert_eq!(rows.len(), 2);
    assert!(
        matches!(&rows[0], DownloadsRow::Page(p) if p.id == 2),
        "the active run leads the list"
    );
    assert!(matches!(&rows[1], DownloadsRow::Page(p) if p.id == 1));
}

// ── cursor re-anchors by run identity across reorders ───────────────────────

#[test]
fn settle_reorder_keeps_the_cursor_on_the_same_run() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Downloading); // row 0 (A)
    let mut b = CollectionPage::new(2, "col 2".to_string(), 2);
    b.stage = DownloadStage::Downloading;
    app.downloads.push(b); // row 1 (B)
    app.downloads_tab.selected = 0; // descended on A
    app.downloads_tab.preview_focused = true;

    // A settles in the background: rows regroup to [B (active), A (settled)].
    app.handle_download_event(DownloadEvent::Finished {
        id: 1,
        summary: DownloadSummary {
            downloaded: 5,
            skipped: 0,
            failed: 0,
            unverified: 0,
        },
    });

    assert_eq!(
        app.selected_download_page().map(|p| p.id),
        Some(1),
        "the cursor must follow run A through the regroup, not stay at row 0"
    );
    // esc on the (now settled) preview must ascend — NOT cancel run B.
    let cmd = app.handle_key(press(KeyCode::Esc));
    assert!(
        cmd.is_none(),
        "esc after a background settle must never cancel a different run, got {cmd:?}"
    );
}

#[test]
fn cancelled_selected_run_lands_on_its_promoted_record() {
    let mut app = make_app();
    let mut other = CollectionPage::new(1, "other".to_string(), 2);
    other.stage = DownloadStage::Downloading;
    app.downloads.push(other);
    push_page(&mut app, 2, DownloadStage::Downloading);
    app.downloads_tab.selected = 1; // the run about to be cancelled
    app.downloads_tab.preview_focused = true;

    app.handle_cancel_result(2, true);

    // Rows are now [page 1, record-of-2]; the cursor sits on the record.
    assert_eq!(app.downloads_tab.selected, 1);
    assert!(
        app.selected_download_page().is_none(),
        "the cursor row is the promoted record, not a live page"
    );
}

// ── record preview: navigation stays put ────────────────────────────────────

#[test]
fn arrows_on_a_record_preview_do_not_move_the_cursor() {
    let mut app = make_app();
    let mut running = CollectionPage::new(1, "running".to_string(), 2);
    running.stage = DownloadStage::Downloading;
    app.downloads.push(running);
    // A cancelled run leaves a record row below the running page.
    let mut doomed = CollectionPage::new(2, "doomed".to_string(), 2);
    doomed.stage = DownloadStage::Downloading;
    app.downloads.push(doomed);
    app.active_tab = Tab::Downloads;
    app.handle_cancel_result(2, true);
    app.downloads_tab.selected = 1; // the record row
    app.downloads_tab.preview_focused = true;

    app.handle_key(press(KeyCode::Up));
    app.handle_key(press(KeyCode::Down));

    assert_eq!(
        app.downloads_tab.selected, 1,
        "a descended record preview must not walk the cursor onto other runs"
    );
}

// ── queueing from another tab lands on the list ──────────────────────────────

#[test]
fn queueing_from_another_tab_resets_stale_preview_focus() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Downloading);
    app.downloads_tab.preview_focused = true; // left descended
    app.active_tab = Tab::Home;

    let mut new_run = CollectionPage::new(2, "new".to_string(), 2);
    new_run.stage = DownloadStage::Downloading;
    app.downloads.push(new_run);
    app.focus_new_download_run();

    assert!(
        !app.downloads_tab.preview_focused,
        "a run queued from another tab must land on the list, not inside a preview"
    );
    assert_eq!(app.downloads_tab.selected, 1, "cursor on the new run");
    assert_eq!(
        app.active_tab,
        Tab::Home,
        "default launch behavior stays on the current tab"
    );
}

#[test]
fn queueing_with_jump_setting_switches_to_downloads_list() {
    let mut app = make_app();
    app.config.jump_to_downloads = true;
    push_page(&mut app, 1, DownloadStage::Downloading);
    app.downloads_tab.preview_focused = true; // left descended
    app.active_tab = Tab::Home;

    let mut new_run = CollectionPage::new(2, "new".to_string(), 2);
    new_run.stage = DownloadStage::Downloading;
    app.downloads.push(new_run);
    app.focus_new_download_run();

    assert_eq!(app.active_tab, Tab::Downloads, "jump setting switches tabs");
    assert!(
        !app.downloads_tab.preview_focused,
        "the jump lands on the list, never inside a preview"
    );
    assert_eq!(app.downloads_tab.selected, 1, "cursor on the new run");
}

#[test]
fn retry_from_descended_preview_never_jumps_or_ascends() {
    let mut app = make_app();
    app.config.jump_to_downloads = true;
    push_page(&mut app, 1, DownloadStage::Failed);
    app.downloads_tab.preview_focused = true; // retrying from the preview

    let mut retry_run = CollectionPage::new(2, "retry".to_string(), 2);
    retry_run.stage = DownloadStage::Downloading;
    app.downloads.push(retry_run);
    app.focus_new_download_run();

    assert_eq!(app.active_tab, Tab::Downloads);
    assert!(
        app.downloads_tab.preview_focused,
        "a retry queued from a descended preview keeps the preview"
    );
}

// ── x stays toast-only ───────────────────────────────────────────────────────

#[test]
fn x_dismisses_toast_and_never_touches_runs() {
    let mut app = make_app();
    push_page(&mut app, 1, DownloadStage::Completed);
    app.toast_err("network unreachable");

    app.handle_key(press(KeyCode::Char('x')));
    assert!(app.toasts.is_empty(), "x dismisses the topmost toast");
    assert_eq!(app.downloads.len(), 1);

    // No toast left: `x` stays inert on this tab.
    app.handle_key(press(KeyCode::Char('x')));
    assert_eq!(app.downloads.len(), 1, "x must never remove a run");
}
