use super::{SECTION_MISSING, SECTION_NONE, count_selected, updates_section};
use crate::app::UpdateField;
use crate::app::update_source::{MissingBeatmapset, MissingStatus};

fn beatmap(id: u32, collection_id: u32, selected: bool) -> MissingBeatmapset {
    MissingBeatmapset {
        id,
        status: MissingStatus::NotInstalled,
        collection_id,
        collection_name: "test".to_string(),
        selected,
        previously_deleted: false,
    }
}

#[test]
fn counts_zero_when_no_entries_for_collection() {
    let cached = vec![beatmap(1, 10, true), beatmap(2, 10, false)];
    let (n, total) = count_selected(&cached, 99);
    assert_eq!(total, 0, "no entries for collection 99");
    assert_eq!(n, 0, "no selected entries for collection 99");
}

#[test]
fn counts_all_selected_when_every_entry_is_selected() {
    let cached = vec![
        beatmap(1, 42, true),
        beatmap(2, 42, true),
        beatmap(3, 99, true),
    ];
    let (n, total) = count_selected(&cached, 42);
    assert_eq!(total, 2, "two entries belong to collection 42");
    assert_eq!(n, 2, "both are selected");
}

#[test]
fn counts_none_selected_when_all_deselected() {
    let cached = vec![beatmap(1, 42, false), beatmap(2, 42, false)];
    let (n, total) = count_selected(&cached, 42);
    assert_eq!(total, 2);
    assert_eq!(n, 0, "none selected");
}

#[test]
fn counts_partial_selection() {
    let cached = vec![
        beatmap(1, 5, true),
        beatmap(2, 5, false),
        beatmap(3, 5, true),
        beatmap(4, 7, true),
    ];
    let (n, total) = count_selected(&cached, 5);
    assert_eq!(total, 3, "three entries in collection 5");
    assert_eq!(n, 2, "two of three are selected");
}

#[test]
fn counts_ignore_other_collections() {
    let cached = vec![
        beatmap(1, 1, true),
        beatmap(2, 2, true),
        beatmap(3, 1, false),
    ];
    let (n, total) = count_selected(&cached, 1);
    assert_eq!(total, 2);
    assert_eq!(n, 1);
    let (n2, total2) = count_selected(&cached, 2);
    assert_eq!(total2, 1);
    assert_eq!(n2, 1);
}

// --- diff indicator show/hide ---

/// The diff visibility rule is: render the diff span only when removed_count > 0.
/// This helper encodes the rule and is tested for each show/hide case.
/// It mirrors the condition in `collection_item` without depending on ratatui internals.
fn diff_is_visible(removed_count: usize) -> bool {
    removed_count > 0
}

#[test]
fn diff_hidden_when_removed_is_zero() {
    assert!(!diff_is_visible(0), "zero removed must be hidden");
}

#[test]
fn diff_shows_when_removed_is_nonzero() {
    assert!(diff_is_visible(7), "-N removed must show");
}

// --- active-section cue ---
//
// The "download selected" button sits below all sections, so focusing it must
// underline NO section header. `section_header` only underlines when the active
// string equals a (non-empty) header label; `SECTION_NONE` ("") never matches.

#[test]
fn download_button_underlines_no_section() {
    let active = updates_section(UpdateField::Download);
    assert_eq!(active, SECTION_NONE, "Download must map to SECTION_NONE");
    assert!(active.is_empty(), "SECTION_NONE must be empty");
    // and it must not match the missing-beatmaps header it used to underline.
    assert_ne!(
        active, SECTION_MISSING,
        "focusing Download must not underline the missing-beatmaps header"
    );
}

#[test]
fn beatmap_list_still_underlines_missing_section() {
    // The list field keeps its section cue — only the button changed.
    assert_eq!(updates_section(UpdateField::BeatmapList), SECTION_MISSING);
}
