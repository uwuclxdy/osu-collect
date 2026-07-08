use super::{FilterSource, FilterStatusMsg, parse_limit, parse_range};
use osu_downloader::filter::{FilterMode, FilterRange, FilterSpecial, FilterStatus};
use std::collections::HashMap;

fn set_special(source: &mut FilterSource, target: &str) {
    // Cycle the chip until the wanted label is active (public surface only).
    for _ in 0..8 {
        if source.special_label() == target {
            return;
        }
        source.cycle_special(true);
    }
    panic!("special label {target} not reachable");
}

#[test]
fn defaults_build_a_leaderboard_query_with_limit_500() {
    let query = FilterSource::new().build_query().expect("default query");
    assert_eq!(query.status, Some(FilterStatus::Leaderboard));
    assert_eq!(query.mode, None);
    assert_eq!(query.special, None);
    assert_eq!(query.limit, Some(500));
    assert!(query.sort.is_some());
    assert_eq!(query.stars, FilterRange::default());
}

#[test]
fn farm_preset_seeds_mode_and_special() {
    let mut source = FilterSource::new();
    // Cycle to "farm" (none → all ranked → loved → farm).
    for _ in 0..3 {
        source.cycle_preset(true);
    }
    assert_eq!(source.preset_label(), "farm");
    let query = source.build_query().expect("farm query");
    assert_eq!(query.mode, Some(FilterMode::Osu));
    assert_eq!(query.special, Some(FilterSpecial::Farm));
}

#[test]
fn seven_star_preset_seeds_stars_min() {
    let mut source = FilterSource::new();
    for _ in 0..5 {
        source.cycle_preset(true);
    }
    assert_eq!(source.preset_label(), "7★+");
    assert_eq!(source.stars.value, "7-");
    let query = source.build_query().expect("7★+ query");
    assert_eq!(query.stars.min, Some(7.0));
    assert_eq!(query.stars.max, None);
}

#[test]
fn cycling_back_to_none_resets_seeded_fields() {
    let mut source = FilterSource::new();
    for _ in 0..3 {
        source.cycle_preset(true); // farm
    }
    source.cycle_preset(false); // loved
    source.cycle_preset(false); // all ranked
    source.cycle_preset(false); // none
    assert_eq!(source.preset_label(), "none");
    let query = source.build_query().expect("reset query");
    assert_eq!(query.mode, None);
    assert_eq!(query.special, None);
    assert_eq!(query.status, Some(FilterStatus::Leaderboard));
}

#[test]
fn parse_range_accepts_all_pair_shapes() {
    assert_eq!(
        parse_range("stars", "5.5-7").expect("both"),
        FilterRange {
            min: Some(5.5),
            max: Some(7.0)
        }
    );
    assert_eq!(
        parse_range("bpm", "180-").expect("min only"),
        FilterRange {
            min: Some(180.0),
            max: None
        }
    );
    assert_eq!(
        parse_range("ar", "-9").expect("max only"),
        FilterRange {
            min: None,
            max: Some(9.0)
        }
    );
    assert_eq!(
        parse_range("cs", "4").expect("exact"),
        FilterRange {
            min: Some(4.0),
            max: Some(4.0)
        }
    );
    assert_eq!(
        parse_range("od", "  ").expect("blank"),
        FilterRange::default()
    );
}

#[test]
fn parse_range_rejects_junk_and_inverted_bounds() {
    let err = parse_range("stars", "abc").expect_err("junk");
    assert!(err.contains("stars"), "error names the field: {err}");
    let err = parse_range("bpm", "200-100").expect_err("inverted");
    assert!(err.contains("greater than max"), "{err}");
    // f64::parse accepts these; the boundary must not let them reach the wire.
    assert!(parse_range("ar", "nan").is_err());
    assert!(parse_range("ar", "inf").is_err());
    assert!(parse_range("ar", "1-inf").is_err());
}

#[test]
fn parse_limit_defaults_and_bounds() {
    assert_eq!(parse_limit("").expect("default"), 500);
    assert_eq!(parse_limit(" 1000 ").expect("explicit"), 1000);
    assert!(parse_limit("0").is_err());
    assert!(parse_limit("20000").is_err());
    assert!(parse_limit("many").is_err());
}

#[test]
fn folder_tag_is_the_preset_label_while_seed_is_untouched() {
    let mut source = FilterSource::new();
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
    let mut source = FilterSource::new();
    for _ in 0..3 {
        source.cycle_preset(true); // farm
    }
    source.stars.set_value("6-");
    let tag = source.folder_tag();
    assert_eq!(tag.len(), 8, "8-hex hash, got {tag}");
    assert!(tag.chars().all(|c| c.is_ascii_hexdigit()));
    // Deterministic: an identical form yields the identical tag.
    let mut twin = FilterSource::new();
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
fn run_label_prefers_preset_then_text_then_descriptor() {
    let mut source = FilterSource::new();
    assert_eq!(source.run_label(), "results");
    source.stars.set_value("7-");
    assert_eq!(source.run_label(), "stars 7-");
    set_special(&mut source, "stream");
    assert_eq!(source.run_label(), "stream");
    source.artist.set_value("camellia");
    assert_eq!(source.run_label(), "camellia");
    let mut preset = FilterSource::new();
    for _ in 0..3 {
        preset.cycle_preset(true);
    }
    assert_eq!(preset.run_label(), "farm");
}

#[test]
fn details_pager_walks_pages_then_dries_up() {
    let mut source = FilterSource::new();
    source.set_results((0..600).collect(), HashMap::new());
    let first = source.next_details_page().expect("page 1");
    assert_eq!(first.len(), 250);
    assert_eq!(first[0], 0);
    let second = source.next_details_page().expect("page 2");
    assert_eq!(second[0], 250);
    let third = source.next_details_page().expect("page 3");
    assert_eq!(third.len(), 100);
    assert!(!source.has_more_details());
    assert!(source.next_details_page().is_none());
}

#[test]
fn details_pager_rewinds_after_a_failed_page() {
    let mut source = FilterSource::new();
    source.set_results((0..300).collect(), HashMap::new());
    let before = source.details_cursor();
    let _ = source.next_details_page().expect("page 1");
    source.rewind_details(before);
    let retry = source.next_details_page().expect("retry page 1");
    assert_eq!(retry[0], 0);
}

#[test]
fn results_snapshot_goes_stale_on_any_input_edit() {
    let mut source = FilterSource::new();
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
fn status_msg_defaults_to_idle() {
    assert_eq!(FilterSource::new().status_msg, FilterStatusMsg::Idle);
}
