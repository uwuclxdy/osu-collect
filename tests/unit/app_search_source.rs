use super::*;

fn row(id: u32) -> BrowseRow {
    BrowseRow { id, meta: None }
}

fn rows(ids: &[u32]) -> Vec<BrowseRow> {
    ids.iter().copied().map(row).collect()
}

// ── SetBrowse ─────────────────────────────────────────────────────────────────

#[test]
fn set_rows_homes_cursor_and_drops_stale_selections() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2, 3]));
    browse.set_all_selected(true);
    assert_eq!(browse.selected_count(), 3);

    // A fresh result set with different ids drops the old selections entirely.
    browse.set_rows(rows(&[4, 5]));
    assert_eq!(browse.selected_count(), 0);
    assert_eq!(browse.list_cursor(), Some(0));
}

#[test]
fn set_rows_keeps_selection_for_surviving_ids() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2, 3]));
    browse.set_all_selected(true);
    // 2 survives into the next set; 1 and 3 drop out.
    browse.set_rows(rows(&[2, 9]));
    assert!(browse.is_selected(2));
    assert!(!browse.is_selected(9));
    assert_eq!(browse.selected_count(), 1);
}

#[test]
fn append_rows_dedups_by_id() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2]));
    // Page overlap: 2 repeats, only 3 and 4 are new.
    browse.append_rows(rows(&[2, 3, 4]));
    let ids: Vec<u32> = browse.rows.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4]);
}

#[test]
fn append_rows_keeps_existing_selection() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2]));
    browse.set_all_selected(true);
    browse.append_rows(rows(&[3]));
    // Load-more never clears what was already picked.
    assert!(browse.is_selected(1));
    assert!(browse.is_selected(2));
    assert!(!browse.is_selected(3));
}

#[test]
fn toggle_selected_flips_row_under_cursor() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[10, 20]));
    browse.descend(); // cursor at 0
    browse.toggle_selected();
    assert_eq!(browse.selected_ids(), vec![10]);
    browse.toggle_selected();
    assert_eq!(browse.selected_ids(), Vec::<u32>::new());
}

#[test]
fn cursor_on_action_at_virtual_last_row() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2]));
    browse.descend();
    assert!(!browse.cursor_on_action());
    // 2 rows + the action bar = 3 nav positions; step down past both rows.
    browse.scroll_down();
    browse.scroll_down();
    assert!(browse.cursor_on_action());
    // The action bar has no highlighted row.
    assert!(browse.highlighted_row().is_none());
}

#[test]
fn toggle_on_action_bar_is_noop() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1]));
    browse.descend();
    browse.scroll_down(); // onto the action bar
    assert!(browse.cursor_on_action());
    browse.toggle_selected();
    assert!(browse.selected_ids().is_empty());
}

#[test]
fn ascend_steps_preview_then_list_then_form() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1]));
    browse.descend();
    browse.focus_preview();
    assert!(browse.preview_focused());

    assert!(browse.ascend()); // preview → list
    assert!(!browse.preview_focused());
    assert!(browse.is_browsing());

    assert!(browse.ascend()); // list → form
    assert!(!browse.is_browsing());

    assert!(!browse.ascend()); // already on the form
}

#[test]
fn focus_preview_noop_on_action_bar() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1]));
    browse.descend();
    browse.scroll_down(); // action bar, no highlighted row
    browse.focus_preview();
    assert!(!browse.preview_focused());
}

#[test]
fn selected_ids_follow_row_order() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[30, 10, 20]));
    browse.set_all_selected(true);
    // Order matches the rows, not the id value or hash order.
    assert_eq!(browse.selected_ids(), vec![30, 10, 20]);
}

// ── SearchSource ──────────────────────────────────────────────────────────────

#[test]
fn build_query_reflects_form() {
    let mut search = SearchSource::new();
    search.query.set_value("blue zenith");
    let query = search.build_query(None);
    assert_eq!(query.text, "blue zenith");
    // Defaults: mode any (None), status default (None), sort relevance/desc.
    assert!(query.mode.is_none());
    assert!(query.status.is_none());
    assert!(query.sort.is_some());
    assert!(query.cursor.is_none());
}

#[test]
fn build_query_trims_text_and_threads_cursor() {
    let mut search = SearchSource::new();
    search.query.set_value("  tekno  ");
    let query = search.build_query(Some("CURSOR".to_string()));
    assert_eq!(query.text, "tekno");
    assert_eq!(query.cursor.as_deref(), Some("CURSOR"));
}

#[test]
fn run_label_uses_query_then_filters_then_generic() {
    let mut search = SearchSource::new();
    // Non-empty query wins.
    search.query.set_value("nekodex");
    assert_eq!(search.run_label(), "nekodex");

    // Empty query falls back to the active status filter.
    search.query.set_value("");
    search.cycle_status(true); // off "default"
    let status = search.status_label().to_string();
    assert_eq!(search.run_label(), status);

    // No query and default filters → a generic tag, never a bare label.
    let mut fresh = SearchSource::new();
    fresh.query.set_value("");
    assert_eq!(fresh.run_label(), "results");
}

#[test]
fn cycle_mode_wraps() {
    let mut search = SearchSource::new();
    assert_eq!(search.mode_label(), "any");
    search.cycle_mode(false); // wrap backward to the last mode
    assert_eq!(search.mode_label(), "mania");
    search.cycle_mode(true); // wrap forward to the first
    assert_eq!(search.mode_label(), "any");
}
