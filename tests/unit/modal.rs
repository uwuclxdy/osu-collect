use super::{changelog_body, inline_spans, is_full_changelog_line, strip_html};
use ratatui::style::Style;
use ratatui::text::Line;

fn joined(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn strip_html_removes_tags_keeps_text() {
    assert_eq!(strip_html("a <img src=x> b"), "a  b");
    assert_eq!(strip_html("<!-- note -->keep"), "keep");
    assert_eq!(strip_html("</details>done"), "done");
}

#[test]
fn strip_html_keeps_bare_less_than() {
    assert_eq!(strip_html("a < b and c"), "a < b and c");
    // A tag-like start with no closing `>` is kept verbatim, not eaten.
    assert_eq!(strip_html("x <notclosed"), "x <notclosed");
}

#[test]
fn full_changelog_line_detected_regardless_of_markup() {
    assert!(is_full_changelog_line("**Full Changelog**: https://x/y"));
    assert!(is_full_changelog_line("Full Changelog: https://x/y"));
    assert!(is_full_changelog_line("  **full changelog** ..."));
    assert!(!is_full_changelog_line(
        "- changed the full changelog format"
    ));
}

#[test]
fn changelog_body_drops_footer_and_trailing_blank() {
    let raw = "- one\n- two\n\n**Full Changelog**: https://x/y\n";
    let rendered: Vec<String> = changelog_body(raw).iter().map(joined).collect();
    assert_eq!(rendered, vec!["• one".to_string(), "• two".to_string()]);
}

#[test]
fn changelog_body_strips_inline_html() {
    let rendered: Vec<String> = changelog_body("- see <img src=x> here")
        .iter()
        .map(joined)
        .collect();
    assert_eq!(rendered, vec!["• see  here".to_string()]);
}

#[test]
fn inline_code_and_bold_are_split_out() {
    let spans = inline_spans("press `i` to **mark**", Style::default());
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "press i to mark");
    // The code run `i` and bold run `mark` become their own spans, markers gone.
    assert!(spans.iter().any(|s| s.content.as_ref() == "i"));
    assert!(spans.iter().any(|s| s.content.as_ref() == "mark"));
}

#[test]
fn unmatched_inline_markers_stay_literal() {
    let spans = inline_spans("a `b and **c", Style::default());
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "a `b and **c");
}

#[test]
fn the_help_overlay_advertises_the_preview_scroll() {
    // `↑ ↓` step the difficulty in a focused browse preview, so the page keys are
    // the only way to reach the rows past its bottom edge. With no footer hint
    // for them, the overlay is where that key is discoverable at all.
    let global: String = super::build_help_lines(crate::app::Tab::Home, false, false)
        .iter()
        .map(joined)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        global.contains("pgup pgdn") && global.contains("scroll preview"),
        "help must name the keys that scroll a preview:\n{global}"
    );
}
