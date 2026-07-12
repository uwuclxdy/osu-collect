use super::super::{SPINNER_FRAMES_PADDED, spinner_str};
use super::{hint_for, hint_line};
use crate::app::{
    App, ConfigField, HomeField, LoginTab, Tab, collection::CollectionPage,
    collection::FailureReason,
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

/// Push a page and descend into its Downloads-tab preview — where the
/// download-control keys (and their hints) live.
fn push_focused_page(app: &mut App, id: DownloadId, stage: DownloadStage) {
    let mut page = CollectionPage::new(id, format!("col {id}"), 1);
    page.stage = stage;
    app.downloads.push(page);
    app.active_tab = Tab::Downloads;
    app.downloads_tab.selected = 0;
    app.downloads_tab.preview_focused = true;
}

#[test]
fn footer_hint_omits_cancel_on_completed_preview() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Completed);

    let hint = hint_for(&app);
    // A settled run has nothing to cancel; esc just ascends (unadvertised).
    assert!(
        !hint.contains("esc cancel"),
        "settled preview must not advertise cancel, got: {hint}"
    );
    assert!(
        hint.contains("← list"),
        "settled preview must advertise the way back, got: {hint}"
    );
}

#[test]
fn footer_hint_advertises_cancel_on_downloading_preview() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Downloading);

    let hint = hint_for(&app);
    // `q` is destructive on a running preview — it must be advertised.
    assert!(
        hint.contains("q cancel"),
        "in-flight preview must advertise `q cancel`, got: {hint}"
    );
}

#[test]
fn footer_hint_downloads_list_advertises_open() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Downloading);
    app.downloads_tab.preview_focused = false;

    let hint = hint_for(&app);
    assert!(
        hint.contains("↵ open"),
        "the run list must advertise `↵ open`, got: {hint}"
    );
    assert!(hint.contains("q quit"), "list level quits on q: {hint}");
}

#[test]
fn footer_hint_settled_preview_has_no_dismiss_token() {
    let mut app = App::new(Config::default());
    push_focused_page(&mut app, 1, DownloadStage::Completed);

    let hint = hint_for(&app);
    // `x` is toast-only (a notification key, not a page action) so it must
    // not appear in the hint while no toast is up.
    assert!(
        !hint.contains("dismiss"),
        "settled preview must not advertise a toast-only `x dismiss` token, got: {hint}"
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
fn footer_hint_caps_at_four_segments_on_settled_preview() {
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

#[test]
fn footer_hint_drops_switch_client_while_login_split_is_open() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.login = Some(LoginTab::new(false));

    let hint = hint_for(&app);
    // `c` is gated `login.is_none()` while the login split traps focus, so the
    // bar must not promise it. The close and help affordances still apply.
    assert!(
        !hint.contains("switch client"),
        "login split must not advertise `c switch client`, got: {hint}"
    );
    assert!(hint.contains("close"), "login must advertise close: {hint}");
    assert!(
        hint.contains("? help"),
        "login must still advertise help: {hint}"
    );
}

#[test]
fn trim_to_fit_drops_globals_in_rank_order_before_context() {
    use super::{HintSegment, join_segments, rendered_width, trim_to_fit};
    use std::collections::HashSet;
    // rank: update(5) > dismiss(4) > switch-client(3) > help(2) > back(1) > context(0)
    let segs = vec![
        HintSegment::context("↑↓ scroll"),
        HintSegment::context("r retry failed"),
        HintSegment::global("c switch client", 3),
        HintSegment::global("u update", 5),
        HintSegment::global("x dismiss", 4),
        HintSegment::global("? help", 2),
        HintSegment::global("q cancel", 1),
    ];
    let wide = rendered_width(&segs);

    // A budget that holds everything trims nothing.
    let mut full = segs.clone();
    trim_to_fit(&mut full, wide);
    assert!(
        join_segments(&full).contains("switch client"),
        "wide budget must keep every hint"
    );

    // Sweep every narrower budget: a hint may vanish only after every
    // higher-rank hint has already gone, and context (rank 0) never drops
    // while any global/back hint remains.
    let rank_of = |t: &str| segs.iter().find(|s| s.text == t).unwrap().drop_rank;
    for budget in 1..=wide {
        let mut s = segs.clone();
        trim_to_fit(&mut s, budget);
        let present: HashSet<&str> = s.iter().map(|s| s.text).collect();
        if present.iter().any(|t| rank_of(t) > 0) {
            for context in ["↑↓ scroll", "r retry failed"] {
                assert!(
                    present.contains(context),
                    "budget {budget}: context hint dropped before a global"
                );
            }
        }
        for a in segs.iter().filter(|s| s.drop_rank > 0) {
            if present.contains(a.text) {
                for b in segs
                    .iter()
                    .filter(|s| s.drop_rank > 0 && s.drop_rank < a.drop_rank)
                {
                    assert!(
                        present.contains(b.text),
                        "budget {budget}: `{}` (rank {}) kept while `{}` (rank {}) was dropped",
                        a.text,
                        a.drop_rank,
                        b.text,
                        b.drop_rank
                    );
                }
            }
        }
    }
}

#[test]
fn source_strip_hint_merges_when_focused_standalone_otherwise() {
    let mut app = App::new(Config::default());

    // Unfocused: the strip-digit jump stands alone as `1-3 switch source`.
    app.home.focus = HomeField::Download;
    let unfocused = hint_for(&app);
    assert!(
        unfocused.contains("1-3 switch source"),
        "unfocused rows advertise the jump, got: {unfocused}"
    );
    assert!(
        !unfocused.contains("↵ / 1-3"),
        "unfocused rows must not show the focused merge, got: {unfocused}"
    );

    // Focused: cycle (`↵`) + jump (`1`-`3`) collapse into one hint so the word
    // "source" isn't repeated down the bar.
    app.home.focus = HomeField::Source;
    let focused = hint_for(&app);
    assert!(
        focused.contains("↵ / 1-3 switch source"),
        "source focus merges cycle+jump, got: {focused}"
    );
    assert!(
        !focused.contains("↵ switch source"),
        "old standalone `↵ switch source` must be gone, got: {focused}"
    );
    assert_eq!(
        focused.matches("switch source").count(),
        1,
        "exactly one switch-source hint, got: {focused}"
    );
}
