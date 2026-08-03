use crate::{
    app::CustomMirrorList,
    app::home::{HomeField, HomeTab, InputField},
    config::Config,
    download::ArchiveValidation,
    mirrors::MirrorKind,
};

fn home_all_off(config: &Config) -> HomeTab {
    let mut home = HomeTab::new(config);
    home.nerinyan = false;
    home.osu_direct = false;
    home.sayobot = false;
    home.nekoha = false;
    home.beatconnect = false;
    home.osudl = false;
    home.catboy = false;
    home.hinamizawa = false;
    home.osu_official = false;
    home.custom_mirrors = CustomMirrorList::from_templates(&[]);
    home
}
#[test]
fn home_defaults_to_every_default_on_mirror() {
    let config = Config::default();
    let home = HomeTab::new(&config);

    // Order follows the canonical `MirrorKind::BUILTINS` (the order the TUI
    // lists and the pipeline tries). hinamizawa + osu! official are default-off,
    // so they are absent here.
    let mirror_kinds: Vec<_> = home
        .build_mirror_list(false)
        .iter()
        .map(|mirror| mirror.kind())
        .collect();
    assert_eq!(
        mirror_kinds,
        vec![
            MirrorKind::OsuDirect,
            MirrorKind::Nerinyan,
            MirrorKind::Sayobot,
            MirrorKind::Nekoha,
            MirrorKind::Beatconnect,
            MirrorKind::Osudl,
            MirrorKind::Catboy,
        ]
    );
}

#[test]
fn build_mirror_list_returns_selected_mirrors() {
    let config = Config::default();
    let mut home = home_all_off(&config);
    home.nerinyan = true;

    let mirrors = home.build_mirror_list(false);
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].kind(), MirrorKind::Nerinyan);
}

#[test]
fn build_mirror_list_empty_when_none_selected() {
    let config = Config::default();
    let home = home_all_off(&config);

    let mirrors = home.build_mirror_list(false);
    assert!(mirrors.is_empty());
}

#[test]
fn build_mirror_list_includes_custom_mirror() {
    let config = Config::default();
    let mut home = home_all_off(&config);
    home.custom_mirrors
        .row_mut(0)
        .unwrap()
        .set_value("https://example.com/d/{id}");

    let mirrors = home.build_mirror_list(false);
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].kind(), MirrorKind::Custom);
}

#[test]
fn build_request_uses_same_mirrors_as_build_mirror_list() {
    let config = Config::default();
    let mut home = home_all_off(&config);
    home.nerinyan = true;
    home.osu_direct = true;
    home.collection.value = "12345".to_string();

    let standalone = home.build_mirror_list(false);
    let request = home
        .build_request(false, ArchiveValidation::Magic, true, 60)
        .unwrap();
    let request_kinds: Vec<_> = request.config.mirrors.iter().map(|m| m.kind()).collect();
    let standalone_kinds: Vec<_> = standalone.iter().map(|m| m.kind()).collect();
    assert_eq!(request_kinds, standalone_kinds);
}

#[test]
fn build_request_passes_archive_validation_argument() {
    let config = Config::default();
    let mut home = HomeTab::new(&config);
    home.collection.value = "12345".to_string();

    let magic = home
        .build_request(false, ArchiveValidation::Magic, true, 60)
        .unwrap();
    assert_eq!(magic.config.archive_validation, ArchiveValidation::Magic);

    let eocd = home
        .build_request(false, ArchiveValidation::Eocd, true, 60)
        .unwrap();
    assert_eq!(eocd.config.archive_validation, ArchiveValidation::Eocd);
}

#[test]
fn build_request_accepts_thread_count_up_to_100() {
    let config = Config::default();
    let mut home = HomeTab::new(&config);
    home.collection.value = "12345".to_string();
    home.threads.value = "100".to_string();

    let request = home
        .build_request(false, ArchiveValidation::Magic, true, 60)
        .unwrap();
    assert_eq!(request.config.concurrent, 100);
}

#[test]
fn build_request_rejects_thread_count_above_100() {
    let config = Config::default();
    let mut home = HomeTab::new(&config);
    home.collection.value = "12345".to_string();
    home.threads.value = "101".to_string();

    let err = home
        .build_request(false, ArchiveValidation::Magic, true, 60)
        .expect_err("101 threads must be rejected");
    assert_eq!(err, "thread count must be between 1 and 100");
}

#[test]
fn threads_stepper_increments_by_one() {
    let config = Config::default();
    let mut home = HomeTab::new(&config);
    // Start from a known value below the max.
    home.threads.value = "2".to_string();
    home.focus = HomeField::Threads;

    home.step_up();

    assert_eq!(home.resolved_threads(), 3);
}

#[test]
fn threads_stepper_decrements_by_one() {
    let config = Config::default();
    let mut home = HomeTab::new(&config);
    home.threads.value = "4".to_string();
    home.focus = HomeField::Threads;

    home.step_down();

    assert_eq!(home.resolved_threads(), 3);
}

#[test]
fn threads_stepper_does_not_go_below_one() {
    let config = Config::default();
    let mut home = HomeTab::new(&config);
    home.threads.value = "1".to_string();
    home.focus = HomeField::Threads;

    home.step_down();

    assert_eq!(home.resolved_threads(), 1);
}

#[test]
fn threads_stepper_does_not_exceed_default_threads() {
    let config = Config::default();
    let mut home = HomeTab::new(&config);
    let max = home.default_threads;
    home.threads.value = max.to_string();

    home.step_up();

    assert_eq!(home.resolved_threads(), max);
}

#[test]
fn threads_digit_key_does_not_mutate_value() {
    let config = Config::default();
    let mut home = HomeTab::new(&config);
    home.focus = HomeField::Threads;
    home.threads.value = "3".to_string();

    home.handle_char('5');

    // Value must remain "3" — digit keys are ignored on the stepper.
    assert_eq!(home.threads.value, "3");
}

#[test]
fn threads_field_is_not_text_input() {
    assert!(!HomeField::Threads.is_text_input());
    assert!(HomeField::Threads.is_stepper());
}

// ── InputField caret model ──────────────────────────────────────────────────

#[test]
fn new_field_parks_caret_at_end() {
    let field = InputField::new("label", "hello", "ph");
    assert_eq!(field.caret(), 5);

    let empty = InputField::new("label", "", "ph");
    assert_eq!(empty.caret(), 0, "empty value parks the caret at 0");
}

#[test]
fn set_value_resets_caret_to_end() {
    let mut field = InputField::new("label", "hello", "ph");
    field.caret_home();
    field.set_value("re-routed");
    assert_eq!(field.caret(), "re-routed".chars().count());
}

#[test]
fn insert_at_caret_lands_mid_string() {
    let mut field = InputField::new("label", "ac", "ph");
    field.caret_left(); // caret between 'a' and 'c'
    field.insert_char('b');
    assert_eq!(field.value, "abc");
    assert_eq!(field.caret(), 2, "caret advances past the inserted char");
}

#[test]
fn insert_str_lands_mid_string_and_advances_caret() {
    let mut field = InputField::new("label", "ad", "ph");
    field.caret_left(); // caret between 'a' and 'd'
    field.insert_str("bc");
    assert_eq!(field.value, "abcd");
    assert_eq!(field.caret(), 3, "caret advances past the whole insert");
}

#[test]
fn insert_str_drops_control_chars() {
    let mut field = InputField::new("label", "", "ph");
    field.insert_str("a\nb\tc\r");
    assert_eq!(field.value, "abc", "newlines/tabs/CR are stripped");
    assert_eq!(field.caret(), 3);
}

#[test]
fn backspace_deletes_char_before_caret() {
    let mut field = InputField::new("label", "abc", "ph");
    field.caret_left(); // caret between 'b' and 'c'
    field.delete_before_caret();
    assert_eq!(field.value, "ac");
    assert_eq!(field.caret(), 1);

    // No-op at the start of the value.
    field.caret_home();
    field.delete_before_caret();
    assert_eq!(field.value, "ac");
    assert_eq!(field.caret(), 0);
}

#[test]
fn delete_at_caret_removes_forward_char() {
    let mut field = InputField::new("label", "abc", "ph");
    field.caret_home();
    field.delete_at_caret();
    assert_eq!(field.value, "bc");
    assert_eq!(field.caret(), 0, "delete leaves the caret in place");

    // No-op at the end of the value.
    field.caret_end();
    field.delete_at_caret();
    assert_eq!(field.value, "bc");
}

#[test]
fn word_delete_acts_left_of_caret_only() {
    let mut field = InputField::new("label", "foo bar baz", "ph");
    // Park the caret right after "bar" (index 7).
    field.caret_left();
    field.caret_left();
    field.caret_left();
    field.caret_left();
    assert_eq!(field.caret(), 7);
    field.delete_word_before_caret();
    assert_eq!(
        field.value, "foo  baz",
        "only the word left of the caret goes"
    );
    assert_eq!(field.caret(), 4, "caret lands at the deletion start");
}

#[test]
fn caret_ops_respect_char_boundaries() {
    let mut field = InputField::new("label", "café", "ph");
    assert_eq!(field.caret(), 4);
    field.caret_left(); // between 'f' and 'é'
    field.insert_char('x');
    assert_eq!(field.value, "cafxé");
    assert_eq!(field.caret(), 4);
    field.delete_at_caret(); // removes 'é'
    assert_eq!(field.value, "cafx");
    field.delete_before_caret(); // removes 'x'
    assert_eq!(field.value, "caf");
}

#[test]
fn caret_movement_clamps_to_bounds() {
    let mut field = InputField::new("label", "ab", "ph");
    field.caret_right();
    field.caret_right();
    assert_eq!(field.caret(), 2, "right clamps at the value end");
    field.caret_left();
    field.caret_left();
    field.caret_left();
    assert_eq!(field.caret(), 0, "left clamps at the value start");
}

// ── mirror_latency_range: Get Maps summary min–max over enabled builtins ──────

use crate::app::runtime::ProbeResult;

/// A home tab with only the three default-on mirrors enabled, no pings yet.
fn home_three_enabled() -> HomeTab {
    let mut home = home_all_off(&Config::default());
    home.nerinyan = true;
    home.osu_direct = true;
    home.sayobot = true;
    home
}

/// No numeric pings yet → no range (unprobed and in-flight contribute nothing).
#[test]
fn latency_range_none_without_numeric_pings() {
    let mut home = home_three_enabled();
    assert_eq!(home.mirror_latency_range(false), None);

    home.mirror_probe_started(); // all in-flight (Some(None))
    assert_eq!(
        home.mirror_latency_range(false),
        None,
        "in-flight probes must not produce a range"
    );
}

/// A single numeric ping collapses to `(n, n)`.
#[test]
fn latency_range_single_value_collapses() {
    let mut home = home_three_enabled();
    home.set_mirror_latency(MirrorKind::Nerinyan, ProbeResult::Ms(42));
    assert_eq!(home.mirror_latency_range(false), Some((42, 42)));
}

/// Min and max span the numeric pings; timeout / error are ignored.
#[test]
fn latency_range_spans_numeric_and_ignores_non_numeric() {
    let mut home = home_three_enabled();
    home.set_mirror_latency(MirrorKind::Nerinyan, ProbeResult::Ms(42));
    home.set_mirror_latency(MirrorKind::OsuDirect, ProbeResult::Ms(118));
    home.set_mirror_latency(MirrorKind::Sayobot, ProbeResult::Timeout);
    assert_eq!(home.mirror_latency_range(false), Some((42, 118)));
}

/// A ping on a disabled mirror is excluded from the range.
#[test]
fn latency_range_excludes_disabled_mirror() {
    let mut home = home_three_enabled();
    home.set_mirror_latency(MirrorKind::Nerinyan, ProbeResult::Ms(50));
    // Nekoha has a faster ping but is disabled → must not widen the range.
    home.nekoha = false;
    home.set_mirror_latency(MirrorKind::Nekoha, ProbeResult::Ms(5));
    assert_eq!(home.mirror_latency_range(false), Some((50, 50)));
}

/// A locked osu! official mirror (toggled on but no valid `*`-scope token) has
/// a numeric ping stored from the probe, yet it must not contribute to the
/// summary range — the download won't use it, so advertising its latency would
/// mislead. The range derives from the same `mirror_enabled` gate as the count.
#[test]
fn latency_range_excludes_locked_osu_official() {
    let mut home = home_all_off(&Config::default());
    home.nerinyan = true;
    home.osu_official = true;
    home.set_mirror_latency(MirrorKind::Nerinyan, ProbeResult::Ms(50));
    // OsuApi has a faster ping but is locked (osu_official_unlocked = false).
    home.set_mirror_latency(MirrorKind::OsuApi, ProbeResult::Ms(5));
    assert_eq!(
        home.mirror_latency_range(false),
        Some((50, 50)),
        "a locked osu! official must not widen the range"
    );
    // Unlocked, its ping joins the range.
    assert_eq!(home.mirror_latency_range(true), Some((5, 50)));
}

/// The adaptive collection download button reads `download (N)` only for a
/// proper nonempty subset of the *currently-resolved* collection; all/none
/// picked, or a browse left over from a different collection, reads `download`.
///
/// Every leg keeps the URL field naming whatever is resolved, because that is
/// now the only pairing the app can produce: the settle drops a snapshot the
/// field moved off, and a landing for a collection the field does not name is
/// dropped before it installs. A fixture holding a resolve under a field naming
/// something else would leave this — the test whose name is about gating on the
/// current collection — unable to catch the gate becoming field-sensitive.
#[test]
fn collection_subset_picked_gates_on_current_collection() {
    use crate::app::BrowseRow;
    let config = Config::default();
    let mut home = HomeTab::new(&config);

    // No browse opened yet → whole-collection download.
    assert!(!home.collection_subset_picked());

    // Browse&pick collection 42 and uncheck one of its two sets → subset.
    home.collection.set_value("42");
    home.set_resolved_collection(42, vec![10, 20]);
    home.collection_browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
        ],
        &home.meta_cache,
    );
    home.collection_browse.set_all_selected(true);
    home.collection_browse_id = Some(42);
    assert!(
        !home.collection_subset_picked(),
        "all selected is download-all"
    );
    home.collection_browse.toggle_selected(); // drop the row under the cursor
    assert!(
        home.collection_subset_picked(),
        "a proper subset flips the label"
    );

    // The user retypes to 99 and its resolve lands: the field and the snapshot
    // move together (the only way they can), leaving the browse bound to 42.
    home.collection.set_value("99");
    home.set_resolved_collection(99, vec![30, 40, 50]);
    assert!(
        !home.collection_subset_picked(),
        "a pick from collection 42 must not label/dispatch collection 99"
    );
}

// ── a retyped collection id vs the loaded resolve ─────────────────────────────
//
// `schedule_resolve` clears the snapshot only on an UNPARSEABLE field, so
// retyping one valid id over another left the previous collection standing
// through the debounce and permanently past a failed fetch. Both sides of
// `collection_subset_picked` had drifted together, so it stayed true and a press
// dispatched the OLD collection's picks under the new id.

/// A form exactly as a landed resolve for 42 plus an opened browse leaves it:
/// the field naming 42, the snapshot and everything derived from it, the browse
/// bound to 42, and one of its two sets unchecked — a proper subset.
fn collection_42_with_a_picked_subset() -> HomeTab {
    use crate::app::BrowseRow;
    use crate::app::home::ResolveState;

    let mut home = HomeTab::new(&Config::default());
    home.collection.set_value("42");
    home.set_collection_resolve(ResolveState::Success, "\"Farm\" · 2 mapsets");
    home.set_resolved_collection(42, vec![10, 20]);
    home.resolved_enrich_pairs = vec![(10, 101), (20, 202)];
    home.resolved_folder_name = Some("Farm-42".to_string());
    home.collection_browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
        ],
        &home.meta_cache,
    );
    home.collection_browse.set_all_selected(true);
    home.collection_browse_id = Some(42);
    home.collection_browse.toggle_selected(); // drop the cursor row → 1 of 2
    home
}

/// Retyping a different valid id drops the snapshot and everything derived from
/// it, so no surface is left naming 42 — the label, the browse button, the
/// folder and the dispatch all read the same cleared state in one frame. The
/// picks are PARKED, not destroyed: they stay bound to the collection they came
/// from, which is what the mistype case below rides on.
#[test]
fn retyping_a_different_valid_id_drops_the_resolve_it_moved_off() {
    let mut home = collection_42_with_a_picked_subset();
    assert!(
        home.collection_subset_picked(),
        "precondition: a proper subset of the resolved collection"
    );

    home.collection.set_value("99");
    home.settle_collection_resolve();

    assert_eq!(home.resolved_collection, None, "snapshot");
    assert_eq!(home.resolved_folder_name, None, "folder name");
    assert!(home.resolved_enrich_pairs.is_empty(), "enrichment seeds");
    assert!(
        home.collection_resolve.is_none(),
        "the status line still naming 42"
    );
    assert_eq!(home.picked_collection_id(), None, "the picks are not 99's");
    assert!(
        !home.collection_subset_picked(),
        "so the button cannot read `download (1)` over 99"
    );
    assert!(
        !home.button_enabled(HomeField::CollectionBrowse, false),
        "`view N mapsets` has no collection left to open"
    );
    assert_eq!(
        home.planned_folder_name(false),
        "<collection>",
        "the hint must not name 42's folder under 99"
    );

    assert_eq!(home.collection_browse.rows.len(), 2, "rows");
    assert_eq!(home.collection_browse.selected_count(), 1, "checks");
    assert_eq!(
        home.collection_browse_id,
        Some(42),
        "the rows still belong to 42 — that pairing was never the stale one"
    );
}

/// The mistype: one wrong character typed and immediately backspaced away passes
/// through a DIFFERENT parsed id and back. Clearing the picks on that pass would
/// cost a selection nothing can rebuild — `open_collection_browse` re-selects
/// every set once the browse id stops matching, and no cache restores a choice
/// that was never fetched. Parking them costs nothing: the round trip ends where
/// it started.
#[test]
fn a_mistyped_id_backspaced_away_keeps_the_picks() {
    let mut home = collection_42_with_a_picked_subset();
    let picked = home.collection_browse.selected_ids();
    assert_eq!(picked.len(), 1, "fixture: exactly one of the two checked");

    // A stray `1` on the end — a valid id, and not 42.
    home.collection.set_value("421");
    home.settle_collection_resolve();
    assert!(
        !home.collection_subset_picked(),
        "42's picks must not dispatch while the field names 421"
    );

    // Backspace, then the debounced resolve for 42 lands again.
    home.collection.set_value("42");
    home.settle_collection_resolve();
    home.set_resolved_collection(42, vec![10, 20]);

    assert_eq!(home.picked_collection_id(), Some(42));
    assert!(
        home.collection_subset_picked(),
        "the picks come back with the collection that owns them"
    );
    assert_eq!(
        home.collection_browse.selected_ids(),
        picked,
        "and they are the SAME picks, not a re-selected default"
    );
}

/// The negative that keeps this from being a form that clears itself on any
/// edit: the settle compares PARSED ids, so respelling 42 as its collector URL
/// changes the field text without naming a different collection. Same fixture,
/// same kind of edit, differing only in whether it moves the collection.
#[test]
fn respelling_the_same_collection_is_not_a_move() {
    let mut home = collection_42_with_a_picked_subset();

    home.collection
        .set_value("https://osucollector.com/collections/42");
    // Reports the collection as already resolved, so the caller skips the fetch.
    // The fixture holds no cached payload, which is what isolates this to the
    // "already current" early return rather than the cache re-arm below it.
    assert!(
        home.settle_collection_resolve(),
        "a respelling of the resolved id owes no request"
    );

    assert_eq!(
        home.resolved_collection,
        Some((42, vec![10, 20])),
        "snapshot"
    );
    assert_eq!(home.resolved_folder_name.as_deref(), Some("Farm-42"));
    assert_eq!(home.resolved_enrich_pairs, vec![(10, 101), (20, 202)]);
    assert!(
        home.collection_resolve.is_some(),
        "the status line describes a collection the field still names"
    );
    assert!(
        home.collection_subset_picked(),
        "a respelling must not drop the picks"
    );
    assert_eq!(home.planned_folder_name(false), "update-42");
}

/// The payload collection 42's own resolve parked in the session cache: the two
/// sets the fixture's browse rows came from, one diff each.
fn cached_collection_42() -> crate::core::collection::Collection {
    use crate::core::collection::{test_beatmapset, test_collection};
    test_collection(
        42,
        vec![test_beatmapset(10, &["aaa"]), test_beatmapset(20, &["bbb"])],
    )
}

/// The mistype, with 42 still in the session cache — which is what a landed
/// resolve for 42 always leaves. The keystroke itself re-arms: no debounce, no
/// request, and the snapshot is the full one `adopt_collection` derives, not a
/// stub standing in for one.
///
/// Parking the picks is only half the design; without this they would sit dead
/// until a fresh network reply landed, and dead for good if it failed.
#[test]
fn a_cached_id_rearms_the_picks_on_the_keystroke() {
    use crate::app::home::ResolveState;

    let mut home = collection_42_with_a_picked_subset();
    home.collection_cache.insert(42, cached_collection_42());
    let picked = home.collection_browse.selected_ids();
    assert_eq!(picked.len(), 1, "fixture: one of the two checked");

    // 421 is not cached, so the settle reports a fetch is still owed.
    home.collection.set_value("421");
    assert!(
        !home.settle_collection_resolve(),
        "an uncached id owes a fetch"
    );
    assert!(!home.collection_subset_picked(), "unarmed while off 42");

    // Backspace: 42 is cached, so nothing is owed and the picks are live again.
    home.collection.set_value("42");
    assert!(
        home.settle_collection_resolve(),
        "a cached id is resolved without a fetch"
    );
    assert!(
        home.collection_subset_picked(),
        "the picks re-arm on the keystroke, not on a reply"
    );
    assert_eq!(home.collection_browse.selected_ids(), picked);

    // The whole snapshot comes back, derived the same way a landed fetch derives
    // it — a re-arm that installed only the id would leave the folder name and
    // the enrichment seeds behind.
    assert_eq!(home.resolved_collection, Some((42, vec![10, 20])));
    assert_eq!(
        home.resolved_folder_name.as_deref(),
        Some("collection-42-42")
    );
    assert_eq!(home.resolved_enrich_pairs, vec![(10, 0), (20, 0)]);
    assert!(
        matches!(home.collection_resolve, Some((ResolveState::Success, ref t)) if t.contains("collection-42") && t.contains("2 mapsets")),
        "the status line names the collection again: {:?}",
        home.collection_resolve
    );
}

/// A cache miss must fall through to today's behaviour rather than fabricate a
/// snapshot. Staleness is the same branch by construction: `get_fresh` is the
/// only reader of the store and holds the TTL inside it, so the settle cannot
/// see an expired entry at all — that boundary is pinned where it lives, in
/// `expired_entry_misses` (`tests/unit/collection_cache.rs`).
#[test]
fn an_uncached_id_does_not_rearm() {
    let mut home = collection_42_with_a_picked_subset();
    home.collection_cache.insert(42, cached_collection_42());

    home.collection.set_value("99");
    assert!(
        !home.settle_collection_resolve(),
        "99 was never fetched — the caller still owes a request"
    );
    assert_eq!(
        home.resolved_collection, None,
        "a miss must not invent a snapshot"
    );
    assert!(home.collection_resolve.is_none(), "nor a status line");
    assert!(home.resolved_folder_name.is_none(), "nor a folder name");
    assert!(!home.collection_subset_picked(), "nor re-arm 42's picks");
}

/// A settle that finds nothing resolved leaves the status line alone. Once the
/// snapshot is gone the line is fetch-scoped — `resolving…` for the request the
/// last keystroke started, or the error that request ended in, which is the only
/// surface reporting a resolve failure — so a further keystroke must not reset
/// it. The `resolved_collection.is_some()` conjunct is the whole of what stops
/// that, since `clear_collection_resolve` nils the line too.
#[test]
fn a_settle_with_nothing_resolved_leaves_the_status_line_alone() {
    use crate::app::home::ResolveState;

    let mut home = collection_42_with_a_picked_subset();
    home.collection.set_value("421");
    home.settle_collection_resolve();
    assert!(
        home.collection_resolve.is_none(),
        "the first keystroke drops 42's line with 42's snapshot"
    );

    // 421's own fetch reports itself, then the user types again.
    home.set_collection_resolve(ResolveState::Loading, "resolving…");
    home.collection.set_value("4210");
    home.settle_collection_resolve();
    let Some((state, ref text)) = home.collection_resolve else {
        panic!("the in-flight fetch's busy cue must survive the next keystroke");
    };
    assert_eq!(state, ResolveState::Loading);
    assert_eq!(text, "resolving…");

    home.set_collection_resolve(ResolveState::Error, "collection not found");
    home.collection.set_value("42100");
    home.settle_collection_resolve();
    assert!(
        matches!(home.collection_resolve, Some((ResolveState::Error, _))),
        "a failed fetch's reason has no second copy anywhere"
    );
}

/// Emptying the field is a move too, and the settle reports it in the same press
/// rather than waiting on the async `Cleared` — one predicate covers both an
/// unparseable field and a different id, so there is no second path to forget.
#[test]
fn an_emptied_field_drops_the_resolve_without_waiting_for_the_clear_event() {
    let mut home = collection_42_with_a_picked_subset();

    home.collection.set_value("");
    home.settle_collection_resolve();

    assert_eq!(home.resolved_collection, None);
    assert!(home.collection_resolve.is_none());
    assert!(!home.collection_subset_picked());
}

// ── supporter gate on the find form ───────────────────────────────────────────

use crate::app::GetMapsSource;

/// Walk the whole tab order from the source strip, returning the fields reached.
/// Drives `next_field` the way `↓` does rather than reading the const list, so a
/// row hidden from the render but left in the list still shows up here.
fn tab_order(home: &mut HomeTab, supporter: bool) -> Vec<HomeField> {
    home.focus = HomeField::Source;
    let mut seen = vec![HomeField::Source];
    loop {
        home.next_field(supporter);
        if home.focus == HomeField::Source {
            return seen;
        }
        assert!(seen.len() < 64, "tab order never wrapped: {seen:?}");
        seen.push(home.focus);
    }
}

const SUPPORTER_ROWS: [HomeField; 6] = [
    HomeField::FindExplicit,
    HomeField::FindGenre,
    HomeField::FindLanguage,
    HomeField::FindExtra,
    HomeField::FindRank,
    HomeField::FindPlayed,
];

fn find_home(open_advanced: bool) -> HomeTab {
    let mut home = HomeTab::new(&Config::default());
    home.source = GetMapsSource::Find;
    if open_advanced {
        home.find.toggle_advanced_filters();
    }
    home
}

/// A hidden row must not be reachable by tab. Both legs of the gate, and both
/// disclosure states — the two dimensions are independent, so a filter keyed on
/// only one of them passes the other leg by accident.
#[test]
fn supporter_rows_leave_the_tab_order_for_a_non_supporter() {
    for open_advanced in [false, true] {
        let mut home = find_home(open_advanced);
        let gated = tab_order(&mut home, false);
        for row in SUPPORTER_ROWS {
            assert!(
                !gated.contains(&row),
                "{row:?} is tab-reachable without supporter (advanced open: {open_advanced})"
            );
        }
        // The rows around them are untouched, so the gate removed exactly the six.
        assert!(gated.contains(&HomeField::FindStatus) && gated.contains(&HomeField::FindSpecial));
    }
}

#[test]
fn supporter_rows_join_the_tab_order_for_a_supporter() {
    // Collapsed: only `explicit` (it lives in the main filters block); the other
    // five are behind the disclosure and stay out until it opens.
    let mut home = find_home(false);
    let collapsed = tab_order(&mut home, true);
    assert!(collapsed.contains(&HomeField::FindExplicit));
    for row in [
        HomeField::FindGenre,
        HomeField::FindLanguage,
        HomeField::FindExtra,
        HomeField::FindRank,
        HomeField::FindPlayed,
    ] {
        assert!(
            !collapsed.contains(&row),
            "{row:?} before the disclosure opens"
        );
    }

    let mut home = find_home(true);
    let open = tab_order(&mut home, true);
    for row in SUPPORTER_ROWS {
        assert!(
            open.contains(&row),
            "{row:?} missing from the supporter tab order"
        );
    }
    // Order follows the render: explicit between categories and special; the
    // five facets open the advanced section, ahead of `stars`.
    let at = |field: HomeField| {
        open.iter()
            .position(|&f| f == field)
            .expect("field in order")
    };
    assert!(at(HomeField::FindStatus) < at(HomeField::FindExplicit));
    assert!(at(HomeField::FindExplicit) < at(HomeField::FindSpecial));
    assert!(at(HomeField::FindAdvanced) < at(HomeField::FindGenre));
    assert!(at(HomeField::FindGenre) < at(HomeField::FindLanguage));
    assert!(at(HomeField::FindLanguage) < at(HomeField::FindExtra));
    assert!(at(HomeField::FindExtra) < at(HomeField::FindRank));
    assert!(at(HomeField::FindRank) < at(HomeField::FindPlayed));
    assert!(at(HomeField::FindPlayed) < at(HomeField::FindStars));
}

/// The gate can close under a focused row (a logout, a `/me` re-probe that comes
/// back negative). Focus must leave, or the caret parks on a row that no longer
/// renders.
#[test]
fn focus_leaves_a_supporter_row_when_the_gate_closes() {
    for row in SUPPORTER_ROWS {
        let mut home = find_home(true);
        home.focus = row;
        home.clamp_supporter_focus(true);
        assert_eq!(home.focus, row, "a confirmed supporter keeps {row:?}");
        home.clamp_supporter_focus(false);
        assert_eq!(home.focus, HomeField::Source, "{row:?} must release focus");
    }
    // A non-supporter row is never displaced by the clamp.
    let mut home = find_home(true);
    home.focus = HomeField::FindStars;
    home.clamp_supporter_focus(false);
    assert_eq!(home.focus, HomeField::FindStars);
}

/// The browse holds the only re-include affordance, so its open gate must read
/// ROWS, not the run's set — a scan whose every find is held back still has to
/// open, or those sets are unreachable for the rest of the session.
#[test]
fn update_browse_opens_when_every_missing_set_is_held_back() {
    use crate::app::GetMapsSource;
    use crate::app::update_source::{MissingBeatmapset, MissingStatus};
    use crate::osu_db::LocalCollection;
    use std::collections::HashMap;

    let config = Config::default();
    let mut home = HomeTab::new(&config);
    home.source = GetMapsSource::Update;
    home.update.set_collections(vec![LocalCollection {
        name: "col - 100".to_string(),
        beatmap_checksums: Box::new([]),
    }]);
    home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 10,
            status: MissingStatus::NotInstalled,
            collection_id: 100,
            collection_name: "col".to_string(),
            included: false,
            previously_deleted: true,
            checksums: Box::new([]),
            enrich_diff_id: None,
        }],
        &HashMap::new(),
    );

    assert_eq!(home.update.total_new_count(), 0, "nothing to fetch");
    assert!(
        !home.button_enabled(HomeField::Download, false),
        "download stays dead — the run would enqueue nothing"
    );
    assert!(
        home.button_enabled(HomeField::UpdateBrowse, false),
        "`view N mapsets` still opens, or the held-back row cannot be reached"
    );
}

/// With zero mirrors configured the Download button must be dead on every source
/// arm, not just the collection-whole one. Each arm's selection condition is met,
/// so the only thing standing between the user and a press that warns "no mirrors"
/// is the shared mirror-count gate.
#[test]
fn download_button_disabled_with_no_mirrors_on_every_source() {
    use crate::app::GetMapsSource;
    use crate::app::find_source::BrowseRow;
    use crate::app::update_source::{MissingBeatmapset, MissingStatus};
    use crate::osu_db::LocalCollection;
    use std::collections::HashMap;

    let config = Config::default();
    let mut home = home_all_off(&config);
    assert_eq!(home.mirror_count(false), 0);

    // collection-subset arm: a proper subset is picked
    home.source = GetMapsSource::Collection;
    home.collection_browse_id = Some(5);
    home.collection.set_value("5");
    home.set_resolved_collection(5, vec![10, 20, 30]);
    home.collection_browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
            BrowseRow { id: 30, meta: None },
        ],
        &HashMap::new(),
    );
    home.collection_browse.set_all_selected(true);
    home.collection_browse.toggle_selected();
    assert!(
        home.collection_subset_picked(),
        "precondition: a subset is picked"
    );
    assert!(
        !home.button_enabled(HomeField::Download, false),
        "collection-subset arm must die with no mirrors"
    );

    // find arm: selections exist
    home.source = GetMapsSource::Find;
    home.find
        .browse
        .set_rows(vec![BrowseRow { id: 10, meta: None }], &HashMap::new());
    home.find.browse.set_all_selected(true);
    assert!(
        home.find.browse.selected_count() > 0,
        "precondition: find has picks"
    );
    assert!(
        !home.button_enabled(HomeField::Download, false),
        "find arm must die with no mirrors"
    );

    // update arm: missing sets are selected
    home.source = GetMapsSource::Update;
    home.update.set_collections(vec![LocalCollection {
        name: "col - 100".to_string(),
        beatmap_checksums: Box::new([]),
    }]);
    home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 10,
            status: MissingStatus::NotInstalled,
            collection_id: 100,
            collection_name: "col".to_string(),
            included: true,
            previously_deleted: false,
            checksums: Box::new([]),
            enrich_diff_id: None,
        }],
        &HashMap::new(),
    );
    assert!(
        home.update.selected_new_count() > 0,
        "precondition: update has missing sets"
    );
    assert!(
        !home.button_enabled(HomeField::Download, false),
        "update arm must die with no mirrors"
    );

    // Positive legs: enable one mirror and verify each arm comes alive. Without
    // these, a `HomeField::Download => false` mutation in `button_enabled` stays
    // green — the test only ever sees `false`, so it cannot tell the arm works.
    home.nerinyan = true;
    assert_eq!(home.mirror_count(false), 1);

    home.source = GetMapsSource::Collection;
    assert!(
        home.collection_subset_picked(),
        "precondition still holds: subset picked"
    );
    assert!(
        home.button_enabled(HomeField::Download, false),
        "collection-subset arm is live with a mirror"
    );

    home.source = GetMapsSource::Find;
    assert!(
        home.button_enabled(HomeField::Download, false),
        "find arm is live with a mirror"
    );

    home.source = GetMapsSource::Update;
    assert!(
        home.button_enabled(HomeField::Download, false),
        "update arm is live with a mirror"
    );
}

/// Pins the pre-computed non-supporter field lists to the runtime filter
/// `is_supporter_only` defines. [`HomeTab::active_fields`] selects between
/// static slices instead of filtering per keypress, so a contributor editing
/// `FIND_FIELDS` or `is_supporter_only` without updating the `_NOSUPPORTER`
/// variants would silently re-introduce tab-reachable rows the render hides.
/// This test fails before that ships.
#[test]
fn non_supporter_field_lists_match_runtime_filter() {
    use crate::app::home::{
        FIND_FIELDS, FIND_FIELDS_COLLAPSED, FIND_FIELDS_COLLAPSED_NOSUPPORTER,
        FIND_FIELDS_NOSUPPORTER,
    };
    assert_eq!(
        FIND_FIELDS_NOSUPPORTER,
        FIND_FIELDS
            .iter()
            .copied()
            .filter(|f| !f.is_supporter_only())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        FIND_FIELDS_COLLAPSED_NOSUPPORTER,
        FIND_FIELDS_COLLAPSED
            .iter()
            .copied()
            .filter(|f| !f.is_supporter_only())
            .collect::<Vec<_>>()
    );
}
