use super::super::{SPINNER_FRAMES_PADDED, spinner_str};
use super::{hint_for, hint_line};
use crate::app::{
    App, ConfigField, HomeField, Tab, collection::CollectionPage, collection::FailureReason,
};
use crate::config::Config;
use crate::download::{DownloadId, DownloadStage, FailedMap};

#[test]
fn spinner_wraps_correctly() {
    for tick in 0u64..30 {
        let frame = spinner_str(tick);
        assert!(SPINNER_FRAMES_PADDED.contains(&frame));
    }
}

#[test]
fn hint_line_has_key_and_label_spans() {
    let line = hint_line("↑↓ move  ·  q quit");
    let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(full.contains("↑↓"));
    assert!(full.contains("move"));
    assert!(full.contains("q"));
    assert!(full.contains("quit"));
}

fn push_focused_page(app: &mut App, id: DownloadId, stage: DownloadStage) {
    let mut page = CollectionPage::new(id, format!("col {id}"), 1);
    page.stage = stage;
    app.downloads.push(page);
    app.active_tab = Tab::Download(app.downloads.len() - 1);
}

#[test]
fn footer_hint_includes_close_on_completed_tab() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Completed);

    let hint = hint_for(&app);
    assert!(
        hint.contains("close"),
        "completed-tab hint should advertise `close`, got: {hint}"
    );
}

#[test]
fn footer_hint_includes_close_on_failed_tab() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Failed);

    let hint = hint_for(&app);
    assert!(
        hint.contains("close"),
        "failed-tab hint must include `close`, got: {hint}"
    );
}

#[test]
fn footer_hint_omits_close_on_downloading_tab() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Downloading);

    let hint = hint_for(&app);
    assert!(
        !hint.contains("close"),
        "in-progress hint must not advertise close: {hint}"
    );
    assert!(
        hint.contains("q abort"),
        "in-progress hint must keep abort: {hint}"
    );
}

#[test]
fn footer_hint_settled_tab_advertises_close_without_a_dismiss_token() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Completed);

    let hint = hint_for(&app);
    // Both esc and q close a settled page. `x` is toast-only (a notification
    // key, not a download-page action) so it must not appear in the hint.
    assert!(
        hint.contains("esc/q close"),
        "settled tab must advertise `esc/q close`, got: {hint}"
    );
    assert!(
        !hint.contains("dismiss"),
        "settled tab must not advertise a toast-only `x dismiss` token, got: {hint}"
    );
    assert!(
        !hint.contains("x/q"),
        "the `x/q` compound must be dropped, got: {hint}"
    );
}

#[test]
fn footer_hint_advertises_dismiss_only_while_a_toast_is_visible() {
    let mut app = App::new(Config::default());
    // Non-text row so keys act globally (not editing) and `x` is a live hotkey.
    app.home.focus = HomeField::AutoOverwrite;

    assert!(
        !hint_for(&app).contains("x dismiss"),
        "no toast → no dismiss token"
    );

    app.toast_info("hi");
    assert!(
        hint_for(&app).contains("x dismiss"),
        "a visible toast must advertise `x dismiss`, got: {}",
        hint_for(&app)
    );

    // Editing types a literal `x`, so the hint must not promise dismissal.
    app.home.focus = HomeField::Collection;
    app.editing = true;
    assert!(
        !hint_for(&app).contains("x dismiss"),
        "editing must not advertise `x dismiss`, got: {}",
        hint_for(&app)
    );
}

#[test]
fn footer_hint_caps_at_four_segments_on_settled_tab() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Completed);

    let hint = hint_for(&app);
    let segments = hint.split('·').count();
    assert!(
        segments <= 4,
        "footer must keep <=4 hints, got {segments}: {hint}"
    );
}

#[test]
fn footer_hint_includes_retry_when_page_has_retryable_failed_maps() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Failed);
    app.downloads[0].failed_maps.push(FailedMap {
        beatmapset_id: 1,
        title: None,
        reason: FailureReason::NetworkError,
    });

    let hint = hint_for(&app);
    assert!(
        hint.contains("r retry"),
        "retryable failures must advertise `r retry`, got: {hint}"
    );
}

#[test]
fn footer_hint_omits_retry_when_failures_are_all_404() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Failed);
    // NotFound (404) is never retryable, so the hint must not promise a key that
    // would do nothing.
    app.downloads[0].failed_maps.push(FailedMap {
        beatmapset_id: 1,
        title: None,
        reason: FailureReason::NotFound,
    });

    let hint = hint_for(&app);
    assert!(
        !hint.contains("retry"),
        "404-only failures must not advertise retry: {hint}"
    );
}

#[test]
fn footer_hint_omits_retry_without_failed_maps() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Failed);

    let hint = hint_for(&app);
    assert!(
        !hint.contains("retry"),
        "hint without failed maps must not advertise retry: {hint}"
    );
}

#[test]
fn home_hint_shows_quit_on_non_text_input_row() {
    let mut app = App::new(Config::default());
    app.home.focus = HomeField::AutoOverwrite;

    let hint = hint_for(&app);
    assert!(
        hint.contains("q quit"),
        "non-text-input home row must advertise `q quit`, got: {hint}"
    );
}

#[test]
fn home_hint_shows_edit_then_done_on_text_input_row() {
    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Collection;

    // Selected-not-editing: enter descends into edit; q still quits (global).
    let hint = hint_for(&app);
    assert!(
        hint.contains("↵ edit"),
        "selected text-input row must advertise `↵ edit`, got: {hint}"
    );
    assert!(hint.contains("q quit"), "not editing → q quits: {hint}");

    // Editing: the hint collapses to the exit affordance.
    app.editing = true;
    let hint = hint_for(&app);
    assert!(
        hint.contains("esc done"),
        "editing must advertise `esc done`, got: {hint}"
    );
}

#[test]
fn footer_hint_trails_help_then_quit_after_the_global_hints() {
    // cloudy-tui order: middle hints → globals → `? help` → back/quit last.
    let mut app = App::new(Config::default());
    app.home.focus = HomeField::AutoOverwrite;

    let hint = hint_for(&app);
    let client = hint.find("c switch client").expect("c switch client shows");
    let help = hint.find("? help").expect("? help shows");
    let quit = hint.find("q quit").expect("q quit shows");
    assert!(
        client < help && help < quit,
        "order must be globals · ? help · q quit, got: {hint}"
    );
}

#[test]
fn config_footer_advertises_reorder_only_on_a_builtin_mirror_row() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;

    app.config.focus = ConfigField::MirrorNerinyan;
    assert!(
        hint_for(&app).contains("⇧↑↓ reorder"),
        "a built-in mirror row must advertise ⇧↑↓ reorder, got: {}",
        hint_for(&app)
    );

    // A non-mirror config row cannot reorder, so the hint must be absent.
    app.config.focus = ConfigField::DownloadVideo;
    assert!(
        !hint_for(&app).contains("reorder"),
        "a non-mirror row must not advertise reorder, got: {}",
        hint_for(&app)
    );
}
