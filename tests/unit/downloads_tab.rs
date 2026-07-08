//! `DownloadsTab` cursor/pane-state tests.

use super::DownloadsTab;

#[test]
fn selection_moves_and_clamps_at_the_ends() {
    let mut view = DownloadsTab::default();

    view.select_prev();
    assert_eq!(view.selected, 0, "no wrap past the top");

    view.select_next(3);
    view.select_next(3);
    assert_eq!(view.selected, 2);
    view.select_next(3);
    assert_eq!(view.selected, 2, "no wrap past the bottom");
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
