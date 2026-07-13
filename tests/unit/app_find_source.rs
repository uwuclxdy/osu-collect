use super::*;
use crate::app::FindBackend;
use osu_downloader::filter::{
    FilterDirection, FilterMode, FilterRange, FilterSort, FilterSpecial, FilterStatus,
};
use osu_downloader::search::{QueryRange, SearchMode, SearchStatus, SortField, SortOrder};
use std::collections::HashMap;

fn row(id: u32) -> BrowseRow {
    BrowseRow { id, meta: None }
}

fn rows(ids: &[u32]) -> Vec<BrowseRow> {
    ids.iter().copied().map(row).collect()
}

/// Cycle a chip until the wanted label is active (public surface only).
fn set_status(source: &mut FindSource, target: &str) {
    for _ in 0..20 {
        if source.status_label() == target {
            return;
        }
        source.cycle_status(true);
    }
    panic!("status label {target} not reachable");
}

fn set_sort(source: &mut FindSource, target: &str) {
    for _ in 0..20 {
        if source.sort_label() == target {
            return;
        }
        source.cycle_sort(true);
    }
    panic!("sort label {target} not reachable");
}

fn set_special(source: &mut FindSource, target: &str) {
    for _ in 0..8 {
        if source.special_label() == target {
            return;
        }
        source.cycle_special(true);
    }
    panic!("special label {target} not reachable");
}

fn osu(source: &FindSource) -> SearchQuery {
    match source.build_plan(None) {
        Ok(FindPlan::Osu(query)) => query,
        other => panic!("expected osu route, got {other:?}"),
    }
}

fn nzbasic(source: &FindSource) -> FilterQuery {
    match source.build_plan(None) {
        Ok(FindPlan::Nzbasic(query)) => query,
        other => panic!("expected nzbasic route, got {other:?}"),
    }
}

// ── SetBrowse ─────────────────────────────────────────────────────────────────

#[test]
fn set_rows_homes_cursor_and_drops_stale_selections() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2, 3]), &HashMap::new());
    browse.set_all_selected(true);
    assert_eq!(browse.selected_count(), 3);

    // A fresh result set with different ids drops the old selections entirely.
    browse.set_rows(rows(&[4, 5]), &HashMap::new());
    assert_eq!(browse.selected_count(), 0);
    assert_eq!(browse.list_cursor(), Some(0));
}

#[test]
fn set_rows_keeps_selection_for_surviving_ids() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2, 3]), &HashMap::new());
    browse.set_all_selected(true);
    // 2 survives into the next set; 1 and 3 drop out.
    browse.set_rows(rows(&[2, 9]), &HashMap::new());
    assert!(browse.is_selected(2));
    assert!(!browse.is_selected(9));
    assert_eq!(browse.selected_count(), 1);
}

#[test]
fn append_rows_dedups_by_id() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2]), &HashMap::new());
    // Page overlap: 2 repeats, only 3 and 4 are new.
    browse.append_rows(rows(&[2, 3, 4]));
    let ids: Vec<u32> = browse.rows.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4]);
}

#[test]
fn append_rows_keeps_existing_selection() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2]), &HashMap::new());
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
    browse.set_rows(rows(&[10, 20]), &HashMap::new());
    browse.descend(); // cursor at 0
    browse.toggle_selected();
    assert_eq!(browse.selected_ids(), vec![10]);
    browse.toggle_selected();
    assert_eq!(browse.selected_ids(), Vec::<u32>::new());
}

#[test]
fn scroll_wraps_within_rows() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2]), &HashMap::new());
    browse.descend();
    // A pure selector: every list position is a real row (no action bar), so a
    // row is always highlighted.
    assert!(browse.highlighted_row().is_some());
    browse.scroll_down(); // → row 1
    browse.scroll_down(); // wraps back to row 0
    assert_eq!(browse.list_cursor(), Some(0));
    assert!(browse.highlighted_row().is_some());
}

#[test]
fn paging_clamps_at_the_ends_without_wrapping() {
    let ids: Vec<u32> = (1..=25).collect();
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&ids), &HashMap::new());
    browse.descend();

    // LIST_PAGE is 10 rows; paging steps by a page and stops at the last row.
    browse.page_down();
    assert_eq!(browse.list_cursor(), Some(10));
    browse.page_down();
    assert_eq!(browse.list_cursor(), Some(20));
    browse.page_down();
    assert_eq!(
        browse.list_cursor(),
        Some(24),
        "page clamps to the last row"
    );
    // A second page at the bottom must NOT wrap to the top (unlike a step).
    browse.page_down();
    assert_eq!(browse.list_cursor(), Some(24), "paging never wraps");

    browse.page_up();
    assert_eq!(browse.list_cursor(), Some(14));
}

#[test]
fn ascend_steps_preview_then_list_then_form() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1]), &HashMap::new());
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
fn selected_ids_follow_row_order() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[30, 10, 20]), &HashMap::new());
    browse.set_all_selected(true);
    // Order matches the rows, not the id value or hash order.
    assert_eq!(browse.selected_ids(), vec![30, 10, 20]);
}

// ── routing matrix ────────────────────────────────────────────────────────────

#[test]
fn defaults_route_osu_leaderboard() {
    let query = osu(&FindSource::new());
    assert_eq!(query.text, "");
    assert_eq!(query.mode, None);
    // The union default status is `leaderboard` (matches both backends), not the
    // old search `None`/server-default.
    assert_eq!(query.status, Some(SearchStatus::Leaderboard));
    assert_eq!(query.sort, Some((SortField::Ranked, SortOrder::Desc)));
    assert!(query.cursor.is_none());
}

#[test]
fn free_text_forces_osu() {
    let mut source = FindSource::new();
    source.query.set_value("blue zenith");
    let query = osu(&source);
    assert_eq!(query.text, "blue zenith");
}

#[test]
fn special_forces_nzbasic() {
    let mut source = FindSource::new();
    set_special(&mut source, "farm");
    let query = nzbasic(&source);
    assert_eq!(query.special, Some(FilterSpecial::Farm));
}

#[test]
fn nzbasic_only_sorts_force_nzbasic() {
    for label in ["bpm ↓", "length ↑"] {
        let mut source = FindSource::new();
        set_sort(&mut source, label);
        assert!(
            matches!(source.build_plan(None), Ok(FindPlan::Nzbasic(_))),
            "{label} must route nzbasic"
        );
    }
}

#[test]
fn osu_only_sorts_force_osu() {
    for (label, field) in [
        ("relevance", SortField::Relevance),
        ("title ↑", SortField::Title),
        ("artist ↑", SortField::Artist),
    ] {
        let mut source = FindSource::new();
        set_sort(&mut source, label);
        let query = osu(&source);
        assert_eq!(
            query.sort.map(|(f, _)| f),
            Some(field),
            "{label} routes osu"
        );
    }
}

#[test]
fn unranked_status_forces_nzbasic_qualified_forces_osu() {
    let mut source = FindSource::new();
    set_status(&mut source, "unranked");
    assert_eq!(nzbasic(&source).status, Some(FilterStatus::Unranked));

    let mut source = FindSource::new();
    set_status(&mut source, "qualified");
    assert_eq!(osu(&source).status, Some(SearchStatus::Qualified));
}

#[test]
fn approved_status_forces_neither() {
    // Alone → the default osu route, emitting the osu approved status.
    let mut source = FindSource::new();
    set_status(&mut source, "approved");
    assert_eq!(osu(&source).status, Some(SearchStatus::Approved));

    // Combined with a nzbasic-forcer → nzbasic, no conflict (both express it).
    let mut source = FindSource::new();
    set_status(&mut source, "approved");
    set_special(&mut source, "farm");
    assert_eq!(nzbasic(&source).status, Some(FilterStatus::Approved));
}

#[test]
fn osu_only_new_criteria_force_osu_and_serialize() {
    // keys → exact q term; forces osu.
    let mut source = FindSource::new();
    source.keys.set_value("7");
    let query = osu(&source);
    assert_eq!(query.keys, Some(QueryRange::Exact(7)));

    // favourites → a min-only range (`+` = at least); forces osu.
    let mut source = FindSource::new();
    source.favourites.set_value("10000+");
    let query = osu(&source);
    assert_eq!(query.favourites, Some(QueryRange::at_least(10000)));

    // ranked date → exact term; forces osu.
    let mut source = FindSource::new();
    source.ranked.set_value("2024");
    let query = osu(&source);
    assert_eq!(query.ranked, Some(QueryRange::Exact("2024".to_string())));
}

#[test]
fn ranked_date_range_uses_dotdot_separator() {
    // A single token stays an exact term (server tolerance is n/a for dates).
    let mut source = FindSource::new();
    source.ranked.set_value("2020-06-01");
    assert_eq!(
        osu(&source).ranked,
        Some(QueryRange::Exact("2020-06-01".to_string()))
    );

    // `a..b` / `a..` / `..b` emit the range — `..` is the separator because the
    // date token itself uses `-` (`yyyy-mm-dd`).
    let mut source = FindSource::new();
    source.ranked.set_value("2020..2024");
    assert_eq!(
        osu(&source).ranked,
        Some(QueryRange::between("2020".to_string(), "2024".to_string()))
    );

    let mut source = FindSource::new();
    source.ranked.set_value("2020-06-01..");
    assert_eq!(
        osu(&source).ranked,
        Some(QueryRange::at_least("2020-06-01".to_string()))
    );

    let mut source = FindSource::new();
    source.ranked.set_value("..2024");
    assert_eq!(
        osu(&source).ranked,
        Some(QueryRange::at_most("2024".to_string()))
    );
}

#[test]
fn ranked_rejects_non_dates_and_dash_ranges() {
    // A `-`-joined pair is NOT a range (that separator is `..`); `2020-2024` is a
    // malformed token (`yyyy-mm` with a 4-digit month), so build_plan errors and
    // names the field.
    let mut source = FindSource::new();
    source.ranked.set_value("2020-2024");
    let err = source.build_plan(None).expect_err("malformed date");
    assert!(err.contains("ranked"), "error names the field: {err}");

    // Junk is rejected too.
    let mut source = FindSource::new();
    source.ranked.set_value("soon");
    assert!(source.build_plan(None).is_err());
}

#[test]
fn ranked_rejects_inverted_range_comparing_shared_precision() {
    // Plain year inversion: rejected, naming the field.
    let mut source = FindSource::new();
    source.ranked.set_value("2024..2020");
    let err = source.build_plan(None).expect_err("inverted year range");
    assert!(err.contains("ranked"), "error names the field: {err}");

    // Equal bounds are a valid (degenerate) range.
    let mut source = FindSource::new();
    source.ranked.set_value("2020..2020");
    assert!(source.build_plan(None).is_ok());

    // Differing precision: the month isn't comparable on both sides, so the
    // shared (year-only) prefix reads as equal — not inverted.
    let mut source = FindSource::new();
    source.ranked.set_value("2020..2020-06");
    assert_eq!(
        osu(&source).ranked,
        Some(QueryRange::between(
            "2020".to_string(),
            "2020-06".to_string()
        ))
    );

    // Same year, comparable month: inverted, rejected.
    let mut source = FindSource::new();
    source.ranked.set_value("2020-06..2020-01");
    let err = source.build_plan(None).expect_err("inverted month range");
    assert!(err.contains("ranked"), "error names the field: {err}");
}

#[test]
fn bare_float_value_emits_exact_on_osu() {
    // A bare value emits `key=value` (server tolerance band), not a `>=`/`<=`
    // pair — so the server widens it, matching osu's own exact-search behaviour.
    let mut source = FindSource::new();
    source.cs.set_value("4");
    assert_eq!(osu(&source).cs, Some(QueryRange::Exact(4.0)));
    // A two-sided `..` range stays an inclusive range.
    source.cs.set_value("4..5");
    assert_eq!(osu(&source).cs, Some(QueryRange::between(4.0, 5.0)));
}

#[test]
fn conflicting_forcers_error_naming_both_fields() {
    // farm (nzbasic) + free text (osu).
    let mut source = FindSource::new();
    set_special(&mut source, "farm");
    source.query.set_value("tekno");
    let err = source.build_plan(None).expect_err("conflict");
    assert_eq!(err, "farm needs nzbasic · free text needs osu! api");

    // nzbasic-only sort + osu-only keys criterion.
    let mut source = FindSource::new();
    set_sort(&mut source, "bpm ↓");
    source.keys.set_value("7");
    let err = source.build_plan(None).expect_err("conflict");
    assert!(err.contains("bpm ↓"), "names the sort forcer: {err}");
    assert!(err.contains("keys"), "names the osu forcer: {err}");
    assert!(
        err.contains("needs nzbasic") && err.contains("needs osu! api"),
        "{err}"
    );
}

#[test]
fn resolved_route_mirrors_the_plan_for_the_indicator() {
    use crate::app::FindRoute;
    // Default (untouched) form → the osu route.
    assert_eq!(FindSource::new().resolved_route(), FindRoute::Osu);

    // A nzbasic-forcer → the nzbasic route.
    let mut source = FindSource::new();
    set_special(&mut source, "farm");
    assert_eq!(source.resolved_route(), FindRoute::Nzbasic);

    // Conflicting forcers → a conflict naming both offending fields (what the
    // read-only indicator renders inline instead of a route).
    let mut source = FindSource::new();
    set_special(&mut source, "farm");
    source.query.set_value("tekno");
    assert_eq!(
        source.resolved_route(),
        FindRoute::Conflict {
            nzbasic: "farm".to_string(),
            osu: "free text".to_string(),
        }
    );

    // Routing ignores parse errors — a mid-edit bad range still shows the route
    // it would take (osu here, since keys forces osu).
    let mut source = FindSource::new();
    source.keys.set_value("not-a-number");
    assert_eq!(source.resolved_route(), FindRoute::Osu);
}

#[test]
fn shared_criteria_ride_the_resolved_route() {
    // Texts + ranges force nothing; a nzbasic-forcer pulls them into the filter
    // query as substring `like` texts + `FilterRange` bounds.
    let mut source = FindSource::new();
    set_special(&mut source, "stream");
    source.artist.set_value("camellia");
    source.stars.set_value("6..7");
    let query = nzbasic(&source);
    assert_eq!(query.artist, "camellia");
    assert_eq!(
        query.stars,
        FilterRange {
            min: Some(6.0),
            max: Some(7.0)
        }
    );
    // `set_special` leaves the mode chip untouched → "any".
    assert_eq!(query.mode, None);
    assert_eq!(query.special, Some(FilterSpecial::Stream));
    // The union default sort maps to the nzbasic column on this route.
    assert_eq!(
        query.sort,
        Some((FilterSort::ApprovedDate, FilterDirection::Desc))
    );
}

#[test]
fn shared_mode_maps_per_backend() {
    // The same chip index maps to the backend-specific mode enum (`Fruits` vs
    // `Catch` for "catch").
    let mut source = FindSource::new();
    source.set_mode_idx(3); // "catch"
    assert_eq!(source.mode_label(), "catch");
    assert_eq!(osu(&source).mode, Some(SearchMode::Fruits));

    let mut source = FindSource::new();
    source.set_mode_idx(3);
    set_special(&mut source, "farm"); // force nzbasic
    assert_eq!(nzbasic(&source).mode, Some(FilterMode::Catch));
}

// ── canonical string / staleness ──────────────────────────────────────────────

#[test]
fn canonical_criteria_string_is_stable_and_sort_limit_independent() {
    let mut a = FindSource::new();
    a.stars.set_value("6..7");
    a.artist.set_value("camellia");
    let mut b = FindSource::new();
    b.stars.set_value("6..7");
    b.artist.set_value("camellia");
    assert_eq!(
        a.criteria_string(),
        b.criteria_string(),
        "identical criteria yield identical canonical strings"
    );

    // Sort/limit are excluded from the criteria string but present in the inputs
    // string (the staleness key).
    let criteria_before = a.criteria_string();
    let inputs_before = a.inputs_string();
    a.cycle_sort(true);
    a.limit.set_value("100");
    assert_eq!(
        a.criteria_string(),
        criteria_before,
        "sort/limit do not change the criteria string"
    );
    assert_ne!(
        a.inputs_string(),
        inputs_before,
        "sort/limit change the staleness key"
    );
}

/// Byte-pin of the canonical criteria shape: any field-order or representation
/// change must surface here as a diff, not as silently changed folder tags.
#[test]
fn criteria_string_is_byte_pinned() {
    let mut source = FindSource::new();
    set_special(&mut source, "farm");
    source.set_mode_idx(3); // catch
    set_status(&mut source, "approved");
    source.query.set_value("tekno");
    source.stars.set_value("6..7");
    source.ar.set_value("9+");
    source.cs.set_value("<=4");
    source.od.set_value("8");
    source.hp.set_value("5.5..6.5");
    source.bpm.set_value("180+");
    source.length.set_value("90..300");
    source.keys.set_value("7");
    source.favourites.set_value("100+");
    source.ranked.set_value("2024");
    source.artist.set_value("cam");
    source.creator.set_value("toby");
    source.title.set_value("night");
    // Ranges render from parsed bounds in operator form (`9+` → `>=9`, bare `8`
    // → `=8`, `<=4` → `<=4`); ranked/texts stay raw-trimmed.
    assert_eq!(
        source.criteria_string(),
        "special=farm|mode=catch|status=approved|q=tekno|stars=>=6 <=7|ar=>=9|cs=<=4|od==8|hp=>=5.5 <=6.5|bpm=>=180|len=>=90 <=300|keys==7|fav=>=100|ranked=2024|artist=cam|creator=toby|title=night"
    );
}

/// Byte-pin of the folder-tag hash: FNV-1a-32 of the canonical criteria string,
/// rendered as 8 lowercase hex. A hash-fn or canonical-shape change breaks this.
#[test]
fn folder_tag_hash_is_byte_pinned() {
    let mut source = FindSource::new();
    source.stars.set_value("<=5");
    assert_eq!(source.folder_tag(), "7f963648");
}

/// Fix for raw-input hashing: equivalent numeric spellings share a folder tag
/// and never stale the loaded results; an invalid mid-edit value still does.
#[test]
fn equivalent_range_spellings_share_tag_and_stay_current() {
    let mut a = FindSource::new();
    a.stars.set_value("6..7");
    let mut b = FindSource::new();
    b.stars.set_value("6.0..7.0");
    assert_eq!(a.folder_tag(), b.folder_tag());

    a.mark_results_current();
    a.stars.set_value("6.0..7.0");
    assert!(
        a.results_current(),
        "an equivalent spelling is not a divergence"
    );
    // An unparseable value falls back to the raw string → reads as diverged.
    a.stars.set_value("6.0..x");
    assert!(!a.results_current());
}

#[test]
fn results_snapshot_goes_stale_on_any_input_edit() {
    let mut source = FindSource::new();
    source.mark_results_current();
    assert!(source.results_current());
    source.limit.set_value("100");
    assert!(
        !source.results_current(),
        "limit edit must stale the snapshot"
    );
    source.mark_results_current();
    source.cycle_status(true);
    assert!(
        !source.results_current(),
        "chip edit must stale the snapshot"
    );
}

#[test]
fn note_results_backend_round_trips() {
    let mut source = FindSource::new();
    assert_eq!(source.results_backend(), None);
    source.note_results_backend(FindBackend::Nzbasic);
    assert_eq!(source.results_backend(), Some(FindBackend::Nzbasic));
    // A stale snapshot does not clear the recorded backend (rows still came
    // from it until a re-fetch).
    source.clear_results_snapshot();
    assert_eq!(source.results_backend(), Some(FindBackend::Nzbasic));
}

// ── folder_tag ────────────────────────────────────────────────────────────────

#[test]
fn folder_tag_is_the_preset_label_while_seed_is_untouched() {
    let mut source = FindSource::new();
    for _ in 0..3 {
        source.cycle_preset(true); // farm
    }
    assert_eq!(source.folder_tag(), "farm");
    // Sort/limit edits keep the preset tag (they don't change the criteria).
    source.cycle_sort(true);
    source.limit.set_value("100");
    assert_eq!(source.folder_tag(), "farm");
}

#[test]
fn folder_tag_becomes_a_stable_hash_once_edited() {
    let mut source = FindSource::new();
    for _ in 0..3 {
        source.cycle_preset(true); // farm
    }
    source.stars.set_value("6-");
    let tag = source.folder_tag();
    assert_eq!(tag.len(), 8, "8-hex hash, got {tag}");
    assert!(tag.chars().all(|c| c.is_ascii_hexdigit()));
    // Deterministic: an identical form yields the identical tag.
    let mut twin = FindSource::new();
    for _ in 0..3 {
        twin.cycle_preset(true);
    }
    twin.stars.set_value("6-");
    assert_eq!(twin.folder_tag(), tag);
    // A different criterion yields a different dir.
    twin.stars.set_value("6.1-");
    assert_ne!(twin.folder_tag(), tag);
}

#[test]
fn folder_tag_free_text_matches_old_search_shape() {
    // An osu free-text run must land in `search-<query>` like the old search
    // source did (its folder_tag was the query itself).
    let mut source = FindSource::new();
    source.query.set_value("tekno");
    assert_eq!(source.folder_tag(), "tekno");
    assert_eq!(source.run_label(), "tekno");
}

#[test]
fn folder_tag_falls_back_to_first_text_then_hash() {
    // No preset, no free text, but a title criterion → the title.
    let mut source = FindSource::new();
    source.title.set_value("night");
    assert_eq!(source.folder_tag(), "night");

    // No preset/free-text/text criterion → the 8-hex criteria hash.
    let mut source = FindSource::new();
    source.stars.set_value("5-");
    let tag = source.folder_tag();
    assert_eq!(tag.len(), 8);
    assert!(tag.chars().all(|c| c.is_ascii_hexdigit()));
}

// ── run_label ─────────────────────────────────────────────────────────────────

#[test]
fn run_label_prefers_preset_free_text_then_descriptor() {
    let mut source = FindSource::new();
    assert_eq!(source.run_label(), "results");
    source.stars.set_value("7+");
    assert_eq!(source.run_label(), "stars 7+");
    set_special(&mut source, "stream");
    assert_eq!(source.run_label(), "stream");
    source.title.set_value("nhelv");
    assert_eq!(source.run_label(), "nhelv");
    // Free text outranks a text criterion.
    source.query.set_value("camellia");
    assert_eq!(source.run_label(), "camellia");

    let mut preset = FindSource::new();
    for _ in 0..3 {
        preset.cycle_preset(true); // farm
    }
    assert_eq!(preset.run_label(), "farm");
}

// ── presets ───────────────────────────────────────────────────────────────────

#[test]
fn farm_preset_seeds_mode_and_special_and_resets_stray_forcers() {
    let mut source = FindSource::new();
    // A stray osu-forcer set before the preset must be cleared by the reset, so
    // the farm preset routes cleanly to nzbasic rather than a conflict.
    source.query.set_value("leftover");
    for _ in 0..3 {
        source.cycle_preset(true); // none → all ranked → loved → farm
    }
    assert_eq!(source.preset_label(), "farm");
    assert_eq!(source.query.value, "", "preset resets the free text");
    let query = nzbasic(&source);
    assert_eq!(query.mode, Some(FilterMode::Osu));
    assert_eq!(query.special, Some(FilterSpecial::Farm));
}

#[test]
fn preset_resets_a_backend_forcing_sort() {
    let mut source = FindSource::new();
    set_sort(&mut source, "relevance"); // osu-only sort
    for _ in 0..3 {
        source.cycle_preset(true); // farm (a nzbasic-forcer)
    }
    // A pure reset-then-seed macro resets the sort too; a leftover osu-only
    // sort would otherwise turn the farm preset into a routing conflict.
    assert_eq!(source.sort_label(), "ranked ↓");
    assert!(matches!(source.build_plan(None), Ok(FindPlan::Nzbasic(_))));
}

#[test]
fn cycling_back_to_none_resets_seeded_fields() {
    let mut source = FindSource::new();
    for _ in 0..3 {
        source.cycle_preset(true); // farm
    }
    source.cycle_preset(false); // loved
    source.cycle_preset(false); // all ranked
    source.cycle_preset(false); // none
    assert_eq!(source.preset_label(), "none");
    // Back to the untouched default: osu route, leaderboard status, mode any.
    let query = osu(&source);
    assert_eq!(query.mode, None);
    assert_eq!(query.status, Some(SearchStatus::Leaderboard));
    assert_eq!(source.special_label(), "none");
}

#[test]
fn seven_star_preset_seeds_stars_min() {
    let mut source = FindSource::new();
    for _ in 0..5 {
        source.cycle_preset(true);
    }
    assert_eq!(source.preset_label(), "7★+");
    assert_eq!(source.stars.value, "7+");
    // Fresh 7★+ has no forcer → osu route with the inclusive star lower bound.
    let query = osu(&source);
    assert_eq!(query.stars, Some(QueryRange::at_least(7.0)));
}

// ── enrichment pager (per-browse) ─────────────────────────────────────────────

#[test]
fn enrichment_pager_walks_pages_then_dries_up() {
    let mut browse = SetBrowse::new();
    browse.seed_enrichment((0..600).map(|d| (d, None)).collect(), &HashMap::new());
    let first = browse.next_enrich_page().expect("page 1");
    assert_eq!(first.len(), ENRICH_PAGE);
    assert_eq!(first[0], 0);
    let second = browse.next_enrich_page().expect("page 2");
    assert_eq!(second[0], ENRICH_PAGE as u32);
    let third = browse.next_enrich_page().expect("page 3");
    assert_eq!(third.len(), 600 - 2 * ENRICH_PAGE);
    assert!(!browse.has_more_enrichment());
    assert!(browse.next_enrich_page().is_none());
}

#[test]
fn enrichment_pager_rewinds_after_a_failed_page() {
    let mut browse = SetBrowse::new();
    browse.seed_enrichment((0..300).map(|d| (d, None)).collect(), &HashMap::new());
    let before = browse.enrich_cursor();
    let _ = browse.next_enrich_page().expect("page 1");
    browse.rewind_enrichment(before);
    let retry = browse.next_enrich_page().expect("retry page 1");
    assert_eq!(retry[0], 0);
}

#[test]
fn seed_and_set_rows_bump_the_enrichment_generation() {
    let mut browse = SetBrowse::new();
    let g0 = browse.enrich_generation();
    browse.seed_enrichment(vec![(1, None), (2, None), (3, None)], &HashMap::new());
    assert_ne!(
        browse.enrich_generation(),
        g0,
        "reseed must bump generation"
    );
    let g1 = browse.enrich_generation();
    // New rows are a new identity: `set_rows` clears the pager + bumps again.
    browse.set_rows(rows(&[10]), &HashMap::new());
    assert_ne!(browse.enrich_generation(), g1);
    assert!(!browse.has_more_enrichment());
}

#[test]
fn status_msg_defaults_to_idle() {
    assert_eq!(FindSource::new().status_msg, FindStatusMsg::Idle);
}

// ── parsers ───────────────────────────────────────────────────────────────────

#[test]
fn parse_range_criterion_accepts_prefix_and_suffix_operators() {
    let ge5 = RangeCriterion::Bounds {
        lower: Some(NumBound {
            value: 5.0,
            inclusive: true,
        }),
        upper: None,
    };
    // `+` suffix and `>=` prefix/suffix all mean the same inclusive lower bound.
    assert_eq!(parse_range_criterion("stars", "5+").unwrap(), Some(ge5));
    assert_eq!(parse_range_criterion("stars", ">=5").unwrap(), Some(ge5));
    assert_eq!(parse_range_criterion("stars", "5>=").unwrap(), Some(ge5));

    // strict `>` — suffix and prefix parse identically.
    let gt7 = RangeCriterion::Bounds {
        lower: Some(NumBound {
            value: 7.0,
            inclusive: false,
        }),
        upper: None,
    };
    assert_eq!(parse_range_criterion("ar", "7>").unwrap(), Some(gt7));
    assert_eq!(parse_range_criterion("ar", ">7").unwrap(), Some(gt7));

    // `<=` is the inclusive upper form; `<` is the strict upper form.
    assert_eq!(
        parse_range_criterion("cs", "<=4").unwrap(),
        Some(RangeCriterion::Bounds {
            lower: None,
            upper: Some(NumBound {
                value: 4.0,
                inclusive: true
            }),
        })
    );
    assert_eq!(
        parse_range_criterion("cs", "<4").unwrap(),
        Some(RangeCriterion::Bounds {
            lower: None,
            upper: Some(NumBound {
                value: 4.0,
                inclusive: false
            }),
        })
    );

    // `-` and `..` are interchangeable range separators: `2-3` ≡ `2..3`.
    let between_5_7 = RangeCriterion::Bounds {
        lower: Some(NumBound {
            value: 5.0,
            inclusive: true,
        }),
        upper: Some(NumBound {
            value: 7.0,
            inclusive: true,
        }),
    };
    assert_eq!(
        parse_range_criterion("stars", "5..7").unwrap(),
        Some(between_5_7)
    );
    assert_eq!(
        parse_range_criterion("stars", "5-7").unwrap(),
        Some(between_5_7)
    );
    // open forms: `180-` / `180..` = min only; `-4` / `..4` = max only.
    assert_eq!(
        parse_range_criterion("bpm", "180-").unwrap(),
        Some(RangeCriterion::Bounds {
            lower: Some(NumBound {
                value: 180.0,
                inclusive: true
            }),
            upper: None,
        })
    );
    assert_eq!(
        parse_range_criterion("cs", "-4").unwrap(),
        Some(RangeCriterion::Bounds {
            lower: None,
            upper: Some(NumBound {
                value: 4.0,
                inclusive: true
            }),
        })
    );

    // bare value = exact; blank = no criterion.
    assert_eq!(
        parse_range_criterion("cs", "4").unwrap(),
        Some(RangeCriterion::Exact(4.0))
    );
    assert_eq!(parse_range_criterion("od", "  ").unwrap(), None);
}

#[test]
fn strict_bounds_reach_osu_but_collapse_on_nzbasic() {
    let mut source = FindSource::new();
    source.stars.set_value("7>");
    // osu keeps the strict `>`.
    assert_eq!(osu(&source).stars, Some(QueryRange::greater_than(7.0)));
    // nzbasic has no strict bound → it lands as an inclusive lower bound.
    set_special(&mut source, "farm"); // force nzbasic
    assert_eq!(
        nzbasic(&source).stars,
        FilterRange {
            min: Some(7.0),
            max: None
        }
    );
}

#[test]
fn parse_range_criterion_rejects_junk_and_inverted() {
    let err = parse_range_criterion("stars", "abc").expect_err("junk");
    assert!(err.contains("stars"), "error names the field: {err}");
    // inverted bounds reject on either separator (`..` and `-`).
    let err = parse_range_criterion("bpm", "200..100").expect_err("inverted ..");
    assert!(err.contains("greater than max"), "{err}");
    let err = parse_range_criterion("bpm", "3-2").expect_err("inverted -");
    assert!(err.contains("greater than max"), "{err}");
    // f64::parse accepts these; the boundary must not let them reach the wire.
    assert!(parse_range_criterion("ar", "nan").is_err());
    assert!(parse_range_criterion("ar", "inf").is_err());
    assert!(parse_range_criterion("ar", "1..inf").is_err());
    // `-9` is the open max form `..9` (`≤9`), not a negative value.
    assert_eq!(
        parse_range_criterion("ar", "-9").unwrap(),
        Some(RangeCriterion::Bounds {
            lower: None,
            upper: Some(NumBound {
                value: 9.0,
                inclusive: true
            }),
        })
    );
    // an operator with no number is rejected.
    assert!(parse_range_criterion("ar", ">").is_err());
}

#[test]
fn parse_limit_defaults_and_bounds() {
    assert_eq!(parse_limit("").expect("default"), 500);
    assert_eq!(parse_limit(" 1000 ").expect("explicit"), 1000);
    assert!(parse_limit("0").is_err());
    assert!(parse_limit("20000").is_err());
    assert!(parse_limit("many").is_err());
}

// ── nekoha size backfill (osu route) ────────────────────────────────────────

/// A find source with `ids` loaded as osu results and exactly `checked` picked.
/// The browse exposes no select-by-id, so each pick is a cursor walk + toggle.
fn find_with_results(ids: &[u32], checked: &[u32]) -> FindSource {
    let mut find = FindSource::new();
    find.browse.set_rows(rows(ids), &HashMap::new());
    find.note_results_backend(FindBackend::Osu);
    for id in checked {
        let index = ids
            .iter()
            .position(|x| x == id)
            .expect("checked id present");
        find.browse.scroll_to_edge(true);
        for _ in 0..index {
            find.browse.scroll_down();
        }
        find.browse.toggle_selected();
    }
    find
}

#[test]
fn claim_size_probes_returns_checked_unprobed_then_dedupes() {
    let mut find = find_with_results(&[1, 2, 3], &[1, 2]);
    let mut first = find.claim_size_probes();
    first.sort_unstable();
    // Only the checked, un-probed ids are claimed (id 3 is unchecked).
    assert_eq!(first, vec![1, 2]);
    // Claiming marks them `Pending`, so an immediate re-claim (rapid toggling)
    // fetches nothing — the in-flight dedupe.
    assert!(find.claim_size_probes().is_empty());
}

#[test]
fn known_sizes_sum_for_checked_missing_excluded_and_no_reprobe() {
    let mut find = find_with_results(&[1, 2, 3], &[1, 2, 3]);
    assert_eq!(find.claim_size_probes().len(), 3);
    find.record_size(1, Some(20 * 1024 * 1024));
    find.record_size(2, Some(30 * 1024 * 1024));
    find.record_size(3, None); // mirror has no size record
    // Only the known sizes sum; the missing set contributes nothing.
    assert_eq!(find.checked_known_bytes(), 50 * 1024 * 1024);
    // Every id now has a definitive state → probed at most once, nothing re-claimed.
    assert!(find.claim_size_probes().is_empty());
}

#[test]
fn unchecked_sets_are_never_claimed_or_summed() {
    let mut find = find_with_results(&[1, 2], &[1]);
    assert_eq!(find.claim_size_probes(), vec![1]);
    find.record_size(1, Some(15 * 1024 * 1024));
    // A size recorded for an unchecked set never reaches the checked sum.
    find.record_size(2, Some(999 * 1024 * 1024));
    assert_eq!(find.checked_known_bytes(), 15 * 1024 * 1024);
}

#[test]
fn partial_coverage_sums_only_what_landed() {
    let mut find = find_with_results(&[1, 2], &[1, 2]);
    find.claim_size_probes();
    find.record_size(1, Some(10 * 1024 * 1024));
    // id 2 is still `Pending` → adds 0; the `~` on the label owns the partiality.
    assert_eq!(find.checked_known_bytes(), 10 * 1024 * 1024);
}
