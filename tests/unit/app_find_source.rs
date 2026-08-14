use super::*;
use crate::app::FindBackend;
use osu_downloader::filter::{
    BeatmapDetails, FilterDirection, FilterMode, FilterRange, FilterSort, FilterSpecial,
    FilterStatus,
};
use osu_downloader::search::{
    Extra, ExtraSet, Genre, Language, PlayedFilter, QueryRange, Rank, RankSet, SearchMode,
    SearchStatus, SortField, SortOrder,
};
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
fn degenerate_range_text_emits_no_term_and_forces_no_route() {
    use crate::app::FindRoute;
    // A bare `-` (or `..`) is a range with neither bound: it emits no `keys` term,
    // so it must not force the osu route either. Routing reads the PARSED
    // criterion, not the raw text.
    for degenerate in ["-", "..", " - ", " .. "] {
        let mut source = FindSource::new();
        source.keys.set_value(degenerate);
        assert_eq!(
            osu(&source).keys,
            None,
            "{degenerate:?} emitted a keys term"
        );

        // Paired with a nzbasic-forcer it no longer conflicts: a field contributing
        // nothing to either query can't block the run.
        set_special(&mut source, "farm");
        assert_eq!(
            source.resolved_route(),
            FindRoute::Nzbasic,
            "{degenerate:?} still forced osu"
        );
    }
}

#[test]
fn degenerate_range_text_forces_no_route_for_every_osu_only_field() {
    use crate::app::FindRoute;
    // Same rule across all three osu-only forcer fields. `ranked` has its own
    // date grammar, so `..` is its only degenerate form (`-` is a date separator).
    type Field = fn(&mut FindSource) -> &mut InputField;
    let cases: [(Field, &str); 3] = [
        (|s| &mut s.keys, "-"),
        (|s| &mut s.favourites, ".."),
        (|s| &mut s.ranked, ".."),
    ];
    for (field, degenerate) in cases {
        let mut source = FindSource::new();
        field(&mut source).set_value(degenerate);
        assert_eq!(source.resolved_route(), FindRoute::Osu, "default route");
        set_special(&mut source, "farm");
        assert_eq!(
            source.resolved_route(),
            FindRoute::Nzbasic,
            "{degenerate:?} forced osu"
        );
    }
}

#[test]
fn unparseable_osu_only_field_still_forces_osu() {
    use crate::app::FindRoute;
    // A broken value emits no term either, but the user typed a criterion — it must
    // never be silently dropped onto a backend that has no column for it. So it
    // forces its route: alone that surfaces the parse error, and against a
    // nzbasic-forcer it raises an honest conflict naming the field.
    let mut source = FindSource::new();
    source.keys.set_value("abc");
    assert_eq!(source.resolved_route(), FindRoute::Osu);
    let err = source
        .build_plan(None)
        .expect_err("bad keys must not build");
    assert!(err.contains("keys"), "error names the field: {err}");

    set_special(&mut source, "farm");
    assert_eq!(
        source.resolved_route(),
        FindRoute::Conflict {
            nzbasic: "farm".to_string(),
            osu: "keys".to_string(),
        }
    );
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
    // `Catch` for "osu!catch").
    let mut source = FindSource::new();
    source.set_mode_idx(3); // "osu!catch"
    assert_eq!(source.mode_label(), "osu!catch");
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
    // string (the staleness key). Moved ONE AT A TIME: together, limit alone
    // satisfies the `assert_ne!` and leaves sort's own contribution unpinned.
    let criteria_before = a.criteria_string();

    let inputs_before = a.inputs_string();
    a.cycle_sort(true);
    assert_eq!(
        a.criteria_string(),
        criteria_before,
        "sort does not change the criteria string"
    );
    assert_ne!(
        a.inputs_string(),
        inputs_before,
        "sort alone changes the staleness key"
    );

    let inputs_before = a.inputs_string();
    a.limit.set_value("100");
    assert_eq!(
        a.criteria_string(),
        criteria_before,
        "limit does not change the criteria string"
    );
    assert_ne!(
        a.inputs_string(),
        inputs_before,
        "limit alone changes the staleness key"
    );
}

/// Sort reaches the staleness key by INDEX, not by its display label — the same
/// rule the criteria chips follow. Lower stakes than the folder tag (this key is
/// never persisted), but one label left in a canonical string reads as "the rule
/// is mostly true", which is how it comes back.
#[test]
fn sort_rename_cannot_move_the_staleness_key() {
    let mut source = FindSource::new();
    set_sort(&mut source, "stars ↓");
    let inputs = source.inputs_string();
    let value = inputs
        .split('|')
        .find_map(|part| part.strip_prefix("sort="))
        .unwrap_or_else(|| panic!("no sort field in {inputs}"));
    assert!(
        value.parse::<usize>().is_ok(),
        "sort carries the display text {value:?}, not its index: {inputs}"
    );
    assert!(
        !inputs.contains(source.sort_label()),
        "the sort LABEL leaked into the staleness key: {inputs}"
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
    // → `=8`, `<=4` → `<=4`); ranked/texts stay raw-trimmed. Chips contribute
    // their index (`farm` = special 1, `osu!catch` = mode 3, `approved` =
    // status 3), never their display text. The six supporter facets trail at
    // their defaults here — `supporter_facets_are_byte_pinned_in_the_criteria`
    // pins them moved.
    assert_eq!(
        source.criteria_string(),
        "special=1|mode=3|status=3|q=tekno|stars=>=6 <=7|ar=>=9|cs=<=4|od==8|hp=>=5.5 <=6.5|bpm=>=180|len=>=90 <=300|keys==7|fav=>=100|ranked=2024|artist=cam|creator=toby|title=night|explicit=0|genre=0|language=0|extra=0|rank=0|played=0"
    );
}

/// Renaming a chip must NOT move the folder tag: the canonical string carries
/// indices, so a label edit is invisible to the on-disk layout.
#[test]
fn chip_rename_cannot_move_the_criteria_string() {
    let mut source = FindSource::new();
    set_status(&mut source, "has leaderboard");
    let canonical = source.criteria_string();
    for key in ["special", "mode", "status"] {
        let value = canonical
            .split('|')
            .find_map(|part| part.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("no {key} field in {canonical}"));
        assert!(
            value.parse::<usize>().is_ok(),
            "{key} carries the display text {value:?}, not its index: {canonical}"
        );
    }
    assert!(canonical.starts_with("special=0|mode=0|status=1|"));
}

/// Byte-pin of the folder-tag hash: FNV-1a-32 of the canonical criteria string,
/// rendered as 8 lowercase hex. A hash-fn or canonical-shape change breaks this.
///
/// The canonical string carries CHIP INDICES, so this pin moves only on a
/// hash-fn, field-order, or chip-ORDER change — never on a reword.
#[test]
fn folder_tag_hash_is_byte_pinned() {
    let mut source = FindSource::new();
    source.stars.set_value("<=5");
    // FNV-1a-32 of `special=0|mode=0|status=1|q=|stars=<=5|ar=|cs=|od=|hp=|bpm=
    // |len=|keys=|fav=|ranked=|artist=|creator=|title=|explicit=0|genre=0
    // |language=0|extra=0|rank=0|played=0`.
    assert_eq!(source.folder_tag(), "c3d0edd8");
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

// ── the route moving off the loaded results' backend ──────────────────────────
//
// The indicator beside the `find` CTA reads the live criteria; the dispatch and
// the output folder read the backend that produced the loaded rows. Left alone
// the two drift, so the rows are dropped whenever the route moves off them.

/// A find source exactly as a landed nzbasic run leaves it: the `special` chip
/// that forced the route, rows, both picked, the recorded backend, the
/// fresh-inputs snapshot, and the count the response reported.
fn loaded_nzbasic_results() -> FindSource {
    let mut source = FindSource::new();
    set_special(&mut source, "farm");
    source.browse.set_rows(rows(&[10, 20]), &HashMap::new());
    source.browse.set_all_selected(true);
    source.browse.descend();
    source.status_msg = FindStatusMsg::ReadyFilter {
        sets: 2,
        total_bytes: 4096,
    };
    source.note_results_backend(FindBackend::Nzbasic);
    source.mark_results_current();
    source
}

/// The same, for a landed osu run: no forcing chip (osu is the default route),
/// and the paging cursor a search response carries.
fn loaded_osu_results() -> FindSource {
    let mut source = FindSource::new();
    source.browse.set_rows(rows(&[10, 20]), &HashMap::new());
    source.browse.set_all_selected(true);
    source.browse.descend();
    source.next_cursor = Some("page-2".to_string());
    source.status_msg = FindStatusMsg::ReadySearch { total: 900 };
    source.note_results_backend(FindBackend::Osu);
    source.mark_results_current();
    source
}

/// Everything keyed to the dropped rows, in one place: the rows and their
/// checks, the browse's descend, the paging cursor, the snapshot, the recorded
/// backend, and the status line that counted them.
fn assert_results_dropped(source: &FindSource, case: &str) {
    assert!(source.browse.rows.is_empty(), "{case}: rows");
    assert_eq!(source.browse.selected_count(), 0, "{case}: checks");
    assert!(!source.browse.is_browsing(), "{case}: browse still open");
    assert!(source.next_cursor.is_none(), "{case}: osu paging cursor");
    assert_eq!(source.results_backend(), None, "{case}: recorded backend");
    assert!(!source.results_current(), "{case}: inputs snapshot");
    assert!(
        matches!(source.status_msg, FindStatusMsg::Idle),
        "{case}: status line still counts the dropped rows"
    );
}

#[test]
fn a_route_change_drops_the_results_the_other_backend_produced() {
    // nzbasic → osu: clear the chip that forced nzbasic.
    let mut source = loaded_nzbasic_results();
    assert_eq!(source.resolved_route(), FindRoute::Nzbasic, "precondition");
    set_special(&mut source, "none");
    assert_eq!(
        source.settle_route(),
        Some(FindBackend::Osu),
        "nzbasic → osu"
    );
    assert_results_dropped(&source, "nzbasic → osu");
    assert_eq!(
        source.run_backend(),
        FindBackend::Osu,
        "the dispatch has to name the backend the form now shows"
    );

    // osu → nzbasic: add a criterion only nzbasic can express.
    let mut source = loaded_osu_results();
    assert_eq!(source.resolved_route(), FindRoute::Osu, "precondition");
    set_special(&mut source, "farm");
    assert_eq!(
        source.settle_route(),
        Some(FindBackend::Nzbasic),
        "osu → nzbasic"
    );
    assert_results_dropped(&source, "osu → nzbasic");
    assert_eq!(source.run_backend(), FindBackend::Nzbasic);
}

/// The negative that keeps this from being a form that clears itself: the same
/// kind of edit on the same fixture, differing only in whether it moves the
/// route. The results go STALE (`view N mapsets` goes inert) but they are still
/// there — staleness and invalidation are not the same state.
#[test]
fn a_criteria_edit_that_keeps_the_route_keeps_the_results() {
    let mut source = loaded_nzbasic_results();
    // `mode` is expressible on both backends, so `special = farm` still decides.
    source.cycle_mode(true);
    assert_eq!(source.resolved_route(), FindRoute::Nzbasic, "precondition");

    assert_eq!(source.settle_route(), None);
    assert_eq!(source.browse.rows.len(), 2, "rows");
    assert_eq!(source.browse.selected_count(), 2, "checks");
    assert_eq!(source.results_backend(), Some(FindBackend::Nzbasic));
    assert!(
        !source.results_current(),
        "the edit still stales the snapshot — that is the pre-existing contract"
    );
}

/// A conflict resolves to no backend, so it is not a move: the `find` CTA
/// refuses to dispatch it and the rows still match the directory they would land
/// in. Dropping them here would also cost the user their results on the first
/// keystroke of every two-step edit that passes through a conflict.
#[test]
fn a_routing_conflict_keeps_the_results_until_the_route_settles() {
    let mut source = loaded_nzbasic_results();
    source.query.set_value("hello"); // free text — an osu forcer over `farm`
    assert!(matches!(
        source.resolved_route(),
        FindRoute::Conflict { .. }
    ));
    assert_eq!(source.settle_route(), None, "a conflict is not a move");
    assert_eq!(source.browse.rows.len(), 2);

    // Backing out of the conflict the way it came leaves the results untouched…
    source.query.set_value("");
    assert_eq!(source.settle_route(), None, "back on the loaded backend");
    assert_eq!(source.browse.rows.len(), 2);

    // …and they are still there to drop once the route settles on the other one.
    source.query.set_value("hello");
    set_special(&mut source, "none");
    assert_eq!(source.settle_route(), Some(FindBackend::Osu));
    assert_results_dropped(&source, "conflict resolved to osu");

    // The mirror leg. Both are needed: a conflict silently read as ONE concrete
    // backend is invisible from the side that already loaded that backend, so a
    // single-direction fixture leaves half the carve-out unpinned.
    let mut source = loaded_osu_results();
    source.query.set_value("hello"); // an osu forcer over the osu default
    assert_eq!(source.settle_route(), None, "still the loaded backend");
    set_special(&mut source, "farm"); // …now an nzbasic forcer on top of it
    assert!(matches!(
        source.resolved_route(),
        FindRoute::Conflict { .. }
    ));
    assert_eq!(source.settle_route(), None, "a conflict is not a move");
    assert_eq!(source.browse.rows.len(), 2);
}

/// The status line splits by what it reports ON. Row-counting statuses describe
/// the dropped result set and go with it (`assert_results_dropped` pins that);
/// fetch-scoped ones describe a request and survive. Both legs move the same
/// fixture the same way and differ only in that dimension.
///
/// `Loading` is still in flight — resetting it drops the busy cue and re-arms the
/// CTA mid-request. `Error` is that same fetch's terminal state and is the ONLY
/// surface reporting a find failure: neither `HomeSearchEvent::Failed` nor
/// `HomeFilterEvent::Failed` toasts, they set this line and nothing else, so
/// clearing it destroys the reason with no other copy anywhere.
#[test]
fn a_route_change_keeps_the_fetch_scoped_statuses() {
    let mut source = loaded_nzbasic_results();
    source.status_msg = FindStatusMsg::Loading;
    set_special(&mut source, "none");
    assert_eq!(source.settle_route(), Some(FindBackend::Osu));
    assert!(source.browse.rows.is_empty(), "the rows still go");
    assert!(
        matches!(source.status_msg, FindStatusMsg::Loading),
        "a fetch in flight still owns the status line"
    );

    let mut source = loaded_nzbasic_results();
    source.status_msg = FindStatusMsg::Error("nzbasic unreachable".to_string());
    set_special(&mut source, "none");
    assert_eq!(source.settle_route(), Some(FindBackend::Osu));
    assert!(source.browse.rows.is_empty(), "the rows still go");
    assert!(
        matches!(source.status_msg, FindStatusMsg::Error(reason) if reason == "nzbasic unreachable"),
        "the sole report of a failed find must survive the rows it outlived"
    );
}

/// The size cache outlives the rows on purpose: a set's download size is a fact
/// about the beatmapset, not about the run that found it, so a re-find never
/// re-probes nekoha for an id it already answered for.
#[test]
fn a_route_change_keeps_the_size_cache() {
    let mut source = loaded_osu_results();
    source.record_size(10, Some(20 * 1024 * 1024));
    set_special(&mut source, "farm");

    assert!(source.settle_route().is_some());
    assert_eq!(
        source.known_sizes_for(&[10]).get(&10).copied(),
        Some(20 * 1024 * 1024),
        "a kept size seeds the next run's estimate"
    );
    assert_eq!(
        source.checked_known_bytes(),
        0,
        "with nothing checked it cannot lend the button a size it would not download"
    );
}

/// A re-run that returned nothing leaves the recorded backend standing over zero
/// rows — `Empty` clears the rows and the snapshot, never the backend. A later
/// route move still has to settle the STATE, or [`FindSource::run_backend`] keeps
/// naming a backend the form stopped showing; but with nothing on screen to have
/// lost, it earns no cue.
#[test]
fn a_route_change_after_an_empty_result_settles_without_a_cue() {
    let mut source = loaded_nzbasic_results();
    // What `HomeFilterEvent::Empty` leaves behind on a re-run.
    source.status_msg = FindStatusMsg::Empty;
    source.browse.set_rows(Vec::new(), &HashMap::new());
    source.clear_results_snapshot();
    assert_eq!(source.results_backend(), Some(FindBackend::Nzbasic));

    set_special(&mut source, "none");
    assert_eq!(
        source.settle_route(),
        None,
        "no rows were on screen to lose"
    );
    assert_eq!(
        source.results_backend(),
        None,
        "the state settles regardless, or the dispatch keeps naming nzbasic"
    );
    assert_eq!(source.run_backend(), FindBackend::Osu);
    assert!(matches!(source.status_msg, FindStatusMsg::Idle));
}

/// With nothing loaded there is nothing to invalidate, so the settle returns
/// before touching the form at all. The reachable case is a FIRST search that
/// came back empty: `HomeSearchEvent::Empty` sets this status and clears the
/// snapshot without ever recording a backend, so a later chip edit forcing the
/// other route arrives with `results_backend` still unset.
///
/// The status here MUST be a row-counting one. It is the only field on a fresh
/// `FindSource` that `clear_results` would move — every other one it touches is
/// already at its default — so it is the whole discriminating power of this
/// test. A fetch-scoped status (`Loading`/`Error`) survives `clear_results` by
/// design, which would make the guarded and unguarded paths indistinguishable
/// and quietly retire the pin. That survival is pinned separately, by
/// `a_route_change_keeps_the_fetch_scoped_statuses`.
#[test]
fn settling_with_no_results_loaded_leaves_the_form_alone() {
    let mut source = FindSource::new();
    source.status_msg = FindStatusMsg::Empty;
    set_special(&mut source, "farm"); // forces the route away from the osu default
    assert_eq!(source.resolved_route(), FindRoute::Nzbasic, "precondition");
    assert_eq!(source.results_backend(), None, "precondition");

    assert_eq!(source.settle_route(), None);
    assert!(
        matches!(source.status_msg, FindStatusMsg::Empty),
        "nothing was loaded, so the settle had no business rewriting the form"
    );
}

/// Self-disarming: the settle clears the very field it reads, so every caller
/// can run it unconditionally instead of working out whether its own edit was
/// the one that moved the route — and the user gets one cue, not one per key.
#[test]
fn settling_the_route_twice_reports_the_move_once() {
    let mut source = loaded_nzbasic_results();
    set_special(&mut source, "none");
    assert_eq!(source.settle_route(), Some(FindBackend::Osu));
    assert_eq!(source.settle_route(), None, "already settled");
    assert_results_dropped(&source, "still cleared");
}

// ── supporter-only facets ─────────────────────────────────────────────────────

/// One supporter facet under test: its user-facing name plus the edit that moves
/// it off default.
type FacetCase = (&'static str, fn(&mut FindSource));
/// Same, plus whether moving it should auto-expand the advanced disclosure.
type AdvancedFacetCase = (&'static str, bool, fn(&mut FindSource));

/// Step a single-select supporter chip to `slot` from its default (slot 0 =
/// `any`), through the same cycle the key handler calls.
fn step(cycle: impl Fn(&mut FindSource, bool), source: &mut FindSource, slot: usize) {
    for _ in 0..slot {
        cycle(source, true);
    }
}

/// Pick multi-select members by chip index, walking the row's own cursor exactly
/// as `←`/`→` do while the row is descended — so these drive the shipped control
/// rather than reaching past it into the mask.
fn pick(chips: &mut ChipSet, indices: &[usize]) {
    for &want in indices {
        // Forward-only, wrapping, so this reaches any chip from any start; 64 is
        // an upper bound on any row's chip count, not a wrap count.
        for _ in 0..64 {
            if chips.cursor() == want {
                break;
            }
            chips.move_cursor(true);
        }
        assert_eq!(chips.cursor(), want, "chip cursor never reached {want}");
        chips.toggle();
    }
}

/// A chip label and its variant's `Debug` name, reduced to what the two are
/// meant to share: the same word, ignoring case and word-split (`VideoGame` vs
/// `"video game"`, `Xh` vs `"XH"`).
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// The one thing binding a chip's DISPLAY text to the value it sends is its
/// position, and the four length asserts on the label tables pass any reorder:
/// swap two entries and the form offers `rock` while sending `g=3` (anime), with
/// the whole suite green and both the results and the on-disk folder tag wrong.
///
/// The expected side is derived from the LIBRARY enum, never from the table
/// under test — the `every_*_maps_to_its_library_variant` tests read
/// `GENRE_LABELS[idx]` on both sides, so they only ever proved the cycle steps
/// by one.
#[test]
fn every_chip_label_names_its_own_library_variant() {
    // `any_slots` is how many leading "no parameter" entries the label table
    // carries before it starts tracking `ALL` position for position.
    let rows: [(&str, &[&str], Vec<String>, usize); 4] = [
        (
            "genre",
            GENRE_LABELS,
            Genre::ALL.iter().map(|v| format!("{v:?}")).collect(),
            1,
        ),
        (
            "language",
            LANGUAGE_LABELS,
            Language::ALL.iter().map(|v| format!("{v:?}")).collect(),
            1,
        ),
        (
            "extra",
            EXTRA_LABELS,
            Extra::ALL.iter().map(|v| format!("{v:?}")).collect(),
            0,
        ),
        (
            "rank",
            RANK_LABELS,
            Rank::ALL.iter().map(|v| format!("{v:?}")).collect(),
            0,
        ),
    ];
    for (chip, labels, variants, any_slots) in rows {
        assert_eq!(
            labels.len(),
            variants.len() + any_slots,
            "{chip}: label table and `ALL` disagree on length"
        );
        for (slot, label) in labels.iter().take(any_slots).enumerate() {
            assert_eq!(*label, "any", "{chip}: slot {slot} sends no parameter");
        }
        for (slot, variant) in variants.iter().enumerate() {
            let label = labels[slot + any_slots];
            assert_eq!(
                squash(label),
                squash(variant),
                "{chip} chip {} reads {label:?} but sends {variant}",
                slot + any_slots
            );
        }
        // Two labels that normalize alike would make a swap between them
        // invisible here, so the pin asserts it can still fail.
        let mut normalized: Vec<String> = labels.iter().map(|l| squash(l)).collect();
        let count = normalized.len();
        normalized.sort();
        normalized.dedup();
        assert_eq!(
            normalized.len(),
            count,
            "{chip}: two labels normalize alike, so this test cannot see them swap"
        );
    }
}

/// Every genre slot must resolve to the library variant at the same position in
/// [`Genre::ALL`]. Enumerated rather than spot-checked: osu!'s genre ids skip 8,
/// so an off-by-one against `ALL` is invisible until the exact slot is asked for.
#[test]
fn every_genre_slot_maps_to_its_library_variant() {
    assert_eq!(osu(&FindSource::new()).genre, None, "slot 0 sends no `g`");
    for (slot, expected) in Genre::ALL.iter().enumerate() {
        let mut source = FindSource::new();
        step(FindSource::cycle_genre, &mut source, slot + 1);
        assert_eq!(source.genre_label(), GENRE_LABELS[slot + 1]);
        assert_eq!(
            osu(&source).genre,
            Some(*expected),
            "genre slot {} ({})",
            slot + 1,
            GENRE_LABELS[slot + 1]
        );
    }
}

#[test]
fn every_language_slot_maps_to_its_library_variant() {
    assert_eq!(
        osu(&FindSource::new()).language,
        None,
        "slot 0 sends no `l`"
    );
    for (slot, expected) in Language::ALL.iter().enumerate() {
        let mut source = FindSource::new();
        step(FindSource::cycle_language, &mut source, slot + 1);
        assert_eq!(source.language_label(), LANGUAGE_LABELS[slot + 1]);
        assert_eq!(
            osu(&source).language,
            Some(*expected),
            "language slot {} ({})",
            slot + 1,
            LANGUAGE_LABELS[slot + 1]
        );
    }
}

/// `any` must send NO `nsfw` parameter — that is what makes it the account's own
/// profile default rather than a third server-side value.
#[test]
fn explicit_chip_maps_any_hide_show_onto_the_nsfw_parameter() {
    for (slot, expected) in [(0, None), (1, Some(false)), (2, Some(true))] {
        let mut source = FindSource::new();
        step(FindSource::cycle_explicit, &mut source, slot);
        assert_eq!(source.explicit_label(), EXPLICIT_LABELS[slot]);
        assert_eq!(osu(&source).nsfw, expected, "explicit slot {slot}");
    }
}

#[test]
fn played_chip_maps_any_played_unplayed_onto_the_played_parameter() {
    for (slot, expected) in [
        (0, None),
        (1, Some(PlayedFilter::Played)),
        (2, Some(PlayedFilter::Unplayed)),
    ] {
        let mut source = FindSource::new();
        step(FindSource::cycle_played, &mut source, slot);
        assert_eq!(source.played_label(), PLAYED_LABELS[slot]);
        assert_eq!(osu(&source).played, expected, "played slot {slot}");
    }
}

/// Each multi-select chip must reach the library variant at its own position.
/// Enumerated per chip, so a whole-row shift and a single mis-indexed chip are
/// both caught.
#[test]
fn every_extra_chip_maps_to_its_library_variant() {
    for (slot, expected) in Extra::ALL.iter().enumerate() {
        let mut source = FindSource::new();
        pick(&mut source.extra, &[slot]);
        assert_eq!(
            osu(&source).extra,
            [*expected].into_iter().collect::<ExtraSet>(),
            "extra chip {slot} ({})",
            EXTRA_LABELS[slot]
        );
    }
}

#[test]
fn every_rank_chip_maps_to_its_library_variant() {
    for (slot, expected) in Rank::ALL.iter().enumerate() {
        let mut source = FindSource::new();
        pick(&mut source.rank, &[slot]);
        assert_eq!(
            osu(&source).rank,
            [*expected].into_iter().collect::<RankSet>(),
            "rank chip {slot} ({})",
            RANK_LABELS[slot]
        );
    }
}

/// The point of the multi-select shape: several members on at once, which is
/// what the `e=video.storyboard` / `r=XH.X` parameters express. The library joins
/// them in `ALL` order, so the set value — not the pick order — is the contract.
#[test]
fn several_members_ride_one_multi_select_row() {
    let mut source = FindSource::new();
    pick(&mut source.extra, &[0, 1]);
    pick(&mut source.rank, &[0, 1, 3]);
    let query = osu(&source);
    assert_eq!(
        query.extra,
        [Extra::Video, Extra::Storyboard]
            .into_iter()
            .collect::<ExtraSet>()
    );
    assert_eq!(
        query.rank,
        [Rank::Xh, Rank::X, Rank::S]
            .into_iter()
            .collect::<RankSet>()
    );
}

/// Picking the same members in the reverse order must land on the identical set,
/// so the emitted parameter is byte-stable whatever the user clicked first.
#[test]
fn multi_select_membership_ignores_pick_order() {
    let mut forward = FindSource::new();
    pick(&mut forward.rank, &[0, 2, 5]);
    let mut backward = FindSource::new();
    pick(&mut backward.rank, &[5, 2, 0]);
    assert_eq!(osu(&forward).rank, osu(&backward).rank);
    assert_eq!(forward.criteria_string(), backward.criteria_string());
}

/// Toggling one member off leaves every other member standing — the failure a
/// row built as a cycle-over-a-bitmask would have.
#[test]
fn untoggling_one_member_leaves_the_rest() {
    let mut source = FindSource::new();
    pick(&mut source.rank, &[0, 1, 3]);
    // Second press on the same chip clears just that one.
    pick(&mut source.rank, &[1]);
    assert_eq!(
        osu(&source).rank,
        [Rank::Xh, Rank::S].into_iter().collect::<RankSet>()
    );
    assert!(source.rank.contains(0) && !source.rank.contains(1) && source.rank.contains(3));
    assert!(!source.rank.is_empty());
}

/// `contains` is public and takes a bare index. Every chip a full row can hold
/// must answer, and anything past the row must answer `false` rather than shift
/// a `u8` out of range — which panics in debug and, in release, wraps to bit 0
/// and reports the FIRST chip's state under another chip's name.
#[test]
fn contains_answers_for_every_addressable_index() {
    let mut source = FindSource::new();
    pick(&mut source.rank, &[0]);
    assert!(source.rank.contains(0));
    for idx in RANK_LABELS.len()..=ChipSet::MAX_MEMBERS + 1 {
        assert!(
            !source.rank.contains(idx),
            "chip {idx} is past the row and must read as unpicked"
        );
    }
    // The narrow row is where an unguarded shift is least likely to be noticed:
    // `extra` has two chips, so six of the mask's eight bits are out of range.
    let mut source = FindSource::new();
    pick(&mut source.extra, &[0]);
    for idx in EXTRA_LABELS.len()..=ChipSet::MAX_MEMBERS + 1 {
        assert!(!source.extra.contains(idx), "extra chip {idx}");
    }
}

/// The chip cursor wraps at both ends and never leaves the row's chip count.
#[test]
fn chip_cursor_wraps_within_the_row() {
    let mut source = FindSource::new();
    assert_eq!(source.extra.cursor(), 0);
    source.extra.move_cursor(false);
    assert_eq!(
        source.extra.cursor(),
        EXTRA_LABELS.len() - 1,
        "← from the first chip wraps to the last"
    );
    source.extra.move_cursor(true);
    assert_eq!(source.extra.cursor(), 0, "→ wraps back");
    // Moving the cursor alone picks nothing.
    assert!(source.extra.is_empty());
    assert_eq!(osu(&source).extra, ExtraSet::new());
}

/// All six are inexpressible on nzbasic, so each one alone must route osu and
/// name itself as the forcer when it meets a nzbasic-forcing criterion.
#[test]
fn every_supporter_facet_forces_the_osu_route() {
    let cases: [FacetCase; 6] = [
        ("explicit", |s| step(FindSource::cycle_explicit, s, 1)),
        ("genre", |s| step(FindSource::cycle_genre, s, 1)),
        ("language", |s| step(FindSource::cycle_language, s, 1)),
        ("extra", |s| pick(&mut s.extra, &[0])),
        ("rank", |s| pick(&mut s.rank, &[0])),
        ("played", |s| step(FindSource::cycle_played, s, 1)),
    ];
    for (field, apply) in cases {
        // Alone: the plan routes osu and the indicator agrees.
        let mut source = FindSource::new();
        apply(&mut source);
        assert!(
            matches!(source.build_plan(None), Ok(FindPlan::Osu(_))),
            "{field} alone must route osu"
        );
        assert_eq!(
            source.resolved_route(),
            crate::app::FindRoute::Osu,
            "{field}"
        );
        assert_eq!(source.planned_backend(), FindBackend::Osu, "{field}");

        // Against a nzbasic forcer: a hard conflict naming BOTH fields.
        let mut source = FindSource::new();
        set_special(&mut source, "farm");
        apply(&mut source);
        let err = source
            .build_plan(None)
            .expect_err("{field} + farm must conflict");
        assert_eq!(err, format!("farm needs nzbasic · {field} needs osu! api"));
        assert_eq!(
            source.resolved_route(),
            crate::app::FindRoute::Conflict {
                nzbasic: "farm".to_string(),
                osu: field.to_string(),
            },
            "the indicator mirrors the conflict for {field}"
        );
    }
}

/// Byte-pin of the six facets inside the canonical criteria string: indices for
/// the single-selects, a member BITMASK over the library's `ALL` order for the
/// two multi-selects. No display text on either — the folder tag hashes this.
#[test]
fn supporter_facets_are_byte_pinned_in_the_criteria() {
    let mut source = FindSource::new();
    step(FindSource::cycle_explicit, &mut source, 1); // hide
    step(FindSource::cycle_genre, &mut source, 3); // anime
    step(FindSource::cycle_language, &mut source, 2); // english
    pick(&mut source.extra, &[1]); // storyboard → bit 1
    pick(&mut source.rank, &[0, 2]); // XH + SH → bits 0 and 2
    step(FindSource::cycle_played, &mut source, 2); // unplayed
    let canonical = source.criteria_string();
    assert!(
        canonical.ends_with("|explicit=1|genre=3|language=2|extra=2|rank=5|played=2"),
        "{canonical}"
    );
    for key in ["explicit", "genre", "language", "extra", "rank", "played"] {
        let value = canonical
            .split('|')
            .find_map(|part| part.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("no {key} field in {canonical}"));
        assert!(
            value.parse::<u32>().is_ok(),
            "{key} carries the display text {value:?}, not a stable index: {canonical}"
        );
    }
}

/// The chip cursor is pure UI, so walking it must not reach the canonical string
/// (and through it a user's on-disk download directory).
#[test]
fn the_chip_cursor_never_reaches_the_criteria_string() {
    let mut source = FindSource::new();
    let before = source.criteria_string();
    let tag_before = source.folder_tag();
    source.rank.move_cursor(true);
    source.rank.move_cursor(true);
    source.extra.move_cursor(false);
    assert_eq!(source.criteria_string(), before);
    assert_eq!(source.folder_tag(), tag_before);
}

/// A preset is a full reset — it must clear the six too, or a nzbasic-seeding
/// preset (`farm`) inherits a stray osu forcer and lands the form in a conflict.
#[test]
fn a_preset_resets_every_supporter_facet() {
    let mut source = FindSource::new();
    step(FindSource::cycle_explicit, &mut source, 2);
    step(FindSource::cycle_genre, &mut source, 4);
    step(FindSource::cycle_language, &mut source, 5);
    step(FindSource::cycle_played, &mut source, 1);
    pick(&mut source.extra, &[0, 1]);
    pick(&mut source.rank, &[2, 4]);

    // `farm` is the nzbasic-seeding preset — the one a leftover forcer breaks.
    for _ in 0..PRESET_LABELS.len() {
        source.cycle_preset(true);
        if source.preset_label() == "farm" {
            break;
        }
    }
    assert_eq!(source.preset_label(), "farm");
    assert!(
        matches!(source.build_plan(None), Ok(FindPlan::Nzbasic(_))),
        "a reset preset must not carry an osu forcer into the plan: {:?}",
        source.resolved_route()
    );
    assert_eq!(source.explicit_label(), "any");
    assert_eq!(source.genre_label(), "any");
    assert_eq!(source.language_label(), "any");
    assert_eq!(source.played_label(), "any");
    assert!(source.extra.is_empty() && source.rank.is_empty());
    assert_eq!(source.extra.cursor(), 0);
    assert_eq!(source.rank.cursor(), 0);
}

/// The five facets that live behind the disclosure auto-expand it, so a live
/// value is never hidden. `explicit` renders in the main block and must NOT.
#[test]
fn an_advanced_facet_auto_expands_the_disclosure() {
    let cases: [AdvancedFacetCase; 6] = [
        ("genre", true, |s| step(FindSource::cycle_genre, s, 1)),
        ("language", true, |s| step(FindSource::cycle_language, s, 1)),
        ("extra", true, |s| pick(&mut s.extra, &[0])),
        ("rank", true, |s| pick(&mut s.rank, &[0])),
        ("played", true, |s| step(FindSource::cycle_played, s, 1)),
        ("explicit", false, |s| {
            step(FindSource::cycle_explicit, s, 1)
        }),
    ];
    for (field, expands, apply) in cases {
        let mut source = FindSource::new();
        assert!(!source.show_advanced_filters());
        apply(&mut source);
        assert_eq!(
            source.show_advanced_filters(),
            expands,
            "{field} auto-expand should be {expands}"
        );
    }
}

/// A facet moves the folder tag (it changes which maps match) and stales a
/// loaded result set. Guards the same class the range fields already pin.
#[test]
fn a_supporter_facet_moves_the_tag_and_stales_the_results() {
    let mut source = FindSource::new();
    let plain = source.folder_tag();
    source.mark_results_current();
    assert!(source.results_current());

    pick(&mut source.rank, &[0]);
    assert_ne!(
        source.folder_tag(),
        plain,
        "a picked rank is a new criteria set"
    );
    assert!(
        !source.results_current(),
        "a facet edit stales the snapshot"
    );
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

// ── details-driven enrichment seeding (nzbasic find) ──────────────────────────

fn detail_row(id: u32, set_id: u32, stars: f64) -> BeatmapDetails {
    BeatmapDetails {
        id,
        set_id,
        title: format!("title {set_id}"),
        artist: "artist".to_string(),
        creator: "mapper".to_string(),
        version: "Insane".to_string(),
        stars,
        bpm: 180.0,
        ar: 9.0,
        cs: 4.0,
        od: 8.0,
        hp: 6.0,
        status: None,
        mode: None,
        total_length: 210,
        favourite_count: 100,
        play_count: 1000,
        size: 0,
        hash: String::new(),
        tags: String::new(),
        source: String::new(),
        genre: String::new(),
        language: String::new(),
        max_combo: 1000,
        hit_length: 118,
        pass_count: 500,
        approved_date: 0,
        last_update: 0,
    }
}

#[test]
fn representative_seeds_pick_the_first_diff_of_each_set() {
    let seeds = representative_seeds(&[
        detail_row(1, 10, 5.0),
        detail_row(2, 10, 6.2),
        detail_row(3, 20, 4.0),
        detail_row(4, 10, 6.1),
        detail_row(5, 20, 3.9),
    ]);
    assert_eq!(
        seeds,
        vec![(1, Some(10)), (3, Some(20))],
        "one diff per set, first row in page order wins"
    );
}

#[test]
fn details_walk_pages_raw_ids_and_feeds_the_cue() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[10]), &HashMap::new());
    browse.seed_details_walk((0..600).collect());
    assert!(browse.has_more_enrichment(), "`m` sees the walk");

    let first = browse.next_details_page().expect("walk page 1");
    assert_eq!(first.len(), ENRICH_PAGE);
    assert_eq!(first[0], 0);
    let _second = browse.next_details_page().expect("walk page 2");
    let third = browse.next_details_page().expect("walk page 3");
    assert_eq!(third.len(), 600 - 2 * ENRICH_PAGE);
    // The osu-batch pager is empty in this browse, so `m`'s view reads as the
    // walk's own dryness.
    assert!(!browse.has_more_enrichment());

    // The walk's dispatch/settle counters drive the same loading cue the
    // osu-batch pager drives.
    browse.mark_details_dispatched();
    assert!(browse.is_enriching());
    browse.mark_details_settled();
    assert!(!browse.is_enriching());
}

#[test]
fn queued_derived_seeds_extend_the_pager_without_bumping_the_generation() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[10, 20]), &HashMap::new());
    let generation = browse.enrich_generation();

    let queued = browse.queue_details_seeds(
        &[detail_row(1, 10, 5.0), detail_row(3, 20, 4.0)],
        &HashMap::new(),
    );
    assert_eq!(queued, 2);
    assert_eq!(
        browse.enrich_generation(),
        generation,
        "queueing extends, it must not orphan an in-flight page"
    );
    assert!(browse.has_unpaged_enrichment());
    assert_eq!(browse.next_enrich_page(), Some(vec![1, 3]));
}

#[test]
fn set_rows_clears_the_details_walk_and_the_seeded_set_dedup() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[10]), &HashMap::new());
    browse.seed_details_walk(vec![1, 2]);
    assert_eq!(
        browse.queue_details_seeds(&[detail_row(1, 10, 5.0)], &HashMap::new()),
        1
    );
    let _ = browse.next_enrich_page();

    // New rows are a new identity: the walk, the pager, and the seeded-set
    // dedup all reset, so the same set may seed again for the new run.
    browse.set_rows(rows(&[10]), &HashMap::new());
    assert!(!browse.has_more_enrichment(), "walk and pager both reset");
    assert_eq!(
        browse.queue_details_seeds(&[detail_row(1, 10, 5.0)], &HashMap::new()),
        1,
        "a fresh run's first landing seeds set 10 again"
    );
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

#[test]
fn a_failed_probe_is_retried_but_a_sizeless_answer_is_not() {
    let mut find = find_with_results(&[1, 2], &[1, 2]);
    assert_eq!(find.claim_size_probes(), vec![1, 2]);
    // Neither id is claimable while its probe is in flight.
    assert!(find.claim_size_probes().is_empty());

    // The mirror answered "no size for this set" — a stable answer, so id 1 is
    // settled and never asked again.
    find.record_size(1, None);
    // id 2's probe never reached the mirror, which says nothing about the set, so
    // its claim is released and the next selection change picks it back up.
    find.release_size_probe(2);
    assert_eq!(find.claim_size_probes(), vec![2]);

    // The retry lands: id 2 gets a size, id 1 stays sizeless.
    find.record_size(2, Some(8 * 1024 * 1024));
    assert_eq!(find.checked_known_bytes(), 8 * 1024 * 1024);
    assert!(find.claim_size_probes().is_empty());
}

#[test]
fn releasing_a_settled_probe_never_reopens_it() {
    let mut find = find_with_results(&[1, 2], &[1, 2]);
    find.claim_size_probes();
    // A late failure for an id whose answer already landed must not discard it:
    // the mirror's answer outranks a failure, whichever arrives last.
    find.record_size(1, Some(4 * 1024 * 1024));
    find.record_size(2, None);
    find.release_size_probe(1);
    find.release_size_probe(2);
    assert!(
        find.claim_size_probes().is_empty(),
        "a settled id is never re-claimed"
    );
    assert_eq!(find.checked_known_bytes(), 4 * 1024 * 1024);
}

#[test]
fn paging_a_focused_preview_scrolls_it_instead_of_the_list() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2, 3]), &HashMap::new());
    browse.descend();
    browse.focus_preview();

    browse.page_down();
    assert_eq!(
        browse.preview_offset.get(),
        10,
        "a focused preview pages by the same LIST_PAGE rows the list does"
    );
    assert_eq!(
        browse.list_cursor(),
        Some(0),
        "the list never moves under a focused preview"
    );

    browse.page_up();
    assert_eq!(browse.preview_offset.get(), 0);
    browse.page_up();
    assert_eq!(
        browse.preview_offset.get(),
        0,
        "the top clamps here; only the render knows where the bottom is"
    );
}

#[test]
fn moving_the_list_cursor_returns_the_preview_to_the_top() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1, 2, 3]), &HashMap::new());
    browse.descend();
    browse.focus_preview();
    browse.page_down();

    browse.focus_list();
    browse.scroll_down();
    assert_eq!(
        browse.preview_offset.get(),
        0,
        "another row is another preview, read from its top"
    );
}

#[test]
fn a_page_key_after_a_jump_to_the_bottom_steps_back_one_page() {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows(&[1]), &HashMap::new());
    browse.descend();
    browse.focus_preview();
    // What the render reports after drawing the pane: 40 rows of content in a
    // 28-row pane leaves 12 to scroll.
    browse.preview_max_offset.set(12);

    browse.scroll_to_edge(false);
    assert_eq!(
        browse.preview_offset.get(),
        12,
        "`G` lands on the bottom row"
    );
    // No frame runs between two keys in one coalesced batch, so the page key has
    // to step back from a real row index — an offset parked past the end would
    // still read as the bottom after subtracting a page, swallowing this key.
    browse.page_up();
    assert_eq!(browse.preview_offset.get(), 2);
}
