//! Toast detail word-wrap.
//!
//! `wrap_to_width` packs a detail line across at most `max_lines` rows, marking
//! the last visible row with `…` when content overflows.

use super::{ellipsize, wrap_to_width};

fn lines(text: &str, budget: u16, max: usize) -> Vec<String> {
    wrap_to_width(text, budget, max)
        .into_iter()
        .map(|(s, _)| s)
        .collect()
}

#[test]
fn short_text_stays_one_line() {
    assert_eq!(lines("all good", 20, 2), vec!["all good"]);
}

#[test]
fn wraps_onto_a_second_line() {
    // budget 12: "one two" (7) fits; "three" would overflow → second line.
    assert_eq!(lines("one two three", 12, 2), vec!["one two", "three"]);
}

#[test]
fn reported_width_matches_the_line() {
    let wrapped = wrap_to_width("one two three", 12, 2);
    for (line, w) in &wrapped {
        assert_eq!(*w as usize, line.chars().count(), "width tracks the line");
    }
}

#[test]
fn overflow_past_the_cap_ellipsizes_the_last_line() {
    // Three words, one line allowed → the single line ends with an ellipsis.
    let out = lines("alpha beta gamma", 8, 1);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].ends_with('…'),
        "truncated line marks the cut: {:?}",
        out[0]
    );
}

#[test]
fn overflow_across_two_lines_marks_the_second() {
    let out = lines("aa bb cc dd ee ff gg", 5, 2);
    assert_eq!(out.len(), 2);
    assert!(
        out[1].ends_with('…'),
        "second line marks the cut: {:?}",
        out[1]
    );
}

#[test]
fn oversized_single_word_is_hard_truncated() {
    let out = lines("supercalifragilistic", 6, 2);
    assert_eq!(out.len(), 1);
    assert!(out[0].ends_with('…'));
    assert!(out[0].chars().count() <= 6);
}

#[test]
fn empty_or_zero_budget_yields_nothing() {
    assert!(lines("", 20, 2).is_empty());
    assert!(lines("text", 0, 2).is_empty());
    assert!(lines("text", 20, 0).is_empty());
}

#[test]
fn ellipsize_appends_when_room_exists() {
    let mut s = "ab".to_string();
    let mut w = 2u16;
    ellipsize(&mut s, &mut w, 6);
    assert_eq!(s, "ab…");
    assert_eq!(w, 3);
}

#[test]
fn ellipsize_cuts_when_full() {
    let mut s = "abcdef".to_string();
    let mut w = 6u16;
    ellipsize(&mut s, &mut w, 6);
    assert!(s.ends_with('…'));
    assert!(w <= 6);
}

#[test]
fn ellipsize_is_idempotent() {
    let mut s = "ab…".to_string();
    let mut w = 3u16;
    ellipsize(&mut s, &mut w, 6);
    assert_eq!(s, "ab…");
}
