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
        .build_mirror_list()
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

    let mirrors = home.build_mirror_list();
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].kind(), MirrorKind::Nerinyan);
}

#[test]
fn build_mirror_list_empty_when_none_selected() {
    let config = Config::default();
    let home = home_all_off(&config);

    let mirrors = home.build_mirror_list();
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

    let mirrors = home.build_mirror_list();
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

    let standalone = home.build_mirror_list();
    let request = home
        .build_request(ArchiveValidation::Magic, true, 60)
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
        .build_request(ArchiveValidation::Magic, true, 60)
        .unwrap();
    assert_eq!(magic.config.archive_validation, ArchiveValidation::Magic);

    let eocd = home
        .build_request(ArchiveValidation::Eocd, true, 60)
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
        .build_request(ArchiveValidation::Magic, true, 60)
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
        .build_request(ArchiveValidation::Magic, true, 60)
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
    assert_eq!(home.mirror_latency_range(), None);

    home.mirror_probe_started(); // all in-flight (Some(None))
    assert_eq!(
        home.mirror_latency_range(),
        None,
        "in-flight probes must not produce a range"
    );
}

/// A single numeric ping collapses to `(n, n)`.
#[test]
fn latency_range_single_value_collapses() {
    let mut home = home_three_enabled();
    home.set_mirror_latency(MirrorKind::Nerinyan, ProbeResult::Ms(42));
    assert_eq!(home.mirror_latency_range(), Some((42, 42)));
}

/// Min and max span the numeric pings; timeout / error are ignored.
#[test]
fn latency_range_spans_numeric_and_ignores_non_numeric() {
    let mut home = home_three_enabled();
    home.set_mirror_latency(MirrorKind::Nerinyan, ProbeResult::Ms(42));
    home.set_mirror_latency(MirrorKind::OsuDirect, ProbeResult::Ms(118));
    home.set_mirror_latency(MirrorKind::Sayobot, ProbeResult::Timeout);
    assert_eq!(home.mirror_latency_range(), Some((42, 118)));
}

/// A ping on a disabled mirror is excluded from the range.
#[test]
fn latency_range_excludes_disabled_mirror() {
    let mut home = home_three_enabled();
    home.set_mirror_latency(MirrorKind::Nerinyan, ProbeResult::Ms(50));
    // Nekoha has a faster ping but is disabled → must not widen the range.
    home.nekoha = false;
    home.set_mirror_latency(MirrorKind::Nekoha, ProbeResult::Ms(5));
    assert_eq!(home.mirror_latency_range(), Some((50, 50)));
}

/// The adaptive collection download button reads `download (N)` only for a
/// proper nonempty subset of the *currently-resolved* collection; all/none
/// picked, or a browse left over from a different collection, reads `download`.
#[test]
fn collection_subset_picked_gates_on_current_collection() {
    use crate::app::BrowseRow;
    let config = Config::default();
    let mut home = HomeTab::new(&config);

    // No browse opened yet → whole-collection download.
    assert!(!home.collection_subset_picked());

    // Browse&pick collection 42 and uncheck one of its two sets → subset.
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

    // A resolve to a different collection makes the left-over pick stale.
    home.set_resolved_collection(99, vec![30, 40, 50]);
    assert!(
        !home.collection_subset_picked(),
        "a pick from collection 42 must not label/dispatch collection 99"
    );
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
