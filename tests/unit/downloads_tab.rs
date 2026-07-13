//! `DownloadsTab` cursor/pane-state tests.

use super::DownloadsTab;

#[test]
fn selection_steps_and_wraps_at_the_ends() {
    let mut view = DownloadsTab::default();

    // Stepping up past the top wraps to the last row (selector-list convention).
    view.select_prev(3);
    assert_eq!(view.selected, 2, "up past the top wraps to the last row");

    // Stepping down past the last row wraps back to the top.
    view.select_next(3);
    assert_eq!(view.selected, 0, "down past the bottom wraps to the top");

    view.select_next(3);
    assert_eq!(view.selected, 1);

    // An empty list is inert either way.
    let mut empty = DownloadsTab::default();
    empty.select_prev(0);
    empty.select_next(0);
    assert_eq!(empty.selected, 0);
}

#[test]
fn paging_steps_by_ten_clamped() {
    let mut view = DownloadsTab::default();

    view.page_down(25);
    assert_eq!(view.selected, 10);
    view.page_down(25);
    assert_eq!(view.selected, 20);
    view.page_down(25);
    assert_eq!(view.selected, 24, "page clamps to the last row");
    view.page_up();
    assert_eq!(view.selected, 14);
}

#[test]
fn clamp_after_removals_keeps_a_real_row() {
    let mut view = DownloadsTab {
        selected: 5,
        preview_focused: true,
        ..Default::default()
    };

    view.clamp(3);
    assert_eq!(view.selected, 2);
    assert!(
        view.preview_focused,
        "a non-empty list keeps the pane focus"
    );

    view.clamp(0);
    assert_eq!(view.selected, 0);
    assert!(
        !view.preview_focused,
        "an empty list has nothing to preview"
    );
}
