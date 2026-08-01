use super::{format_eta, overview_lines, rate_limited_message, session_eta, tally_line};
use crate::utils::format_bytes;

#[test]
fn format_bytes_units() {
    // Throughput → SI decimal (KB/s, MB/s); storage → IEC binary (KiB, GiB).
    assert_eq!(format_bytes(500, "B/s"), "500 B/s");
    assert_eq!(format_bytes(1024, "B/s"), "1 KB/s");
    assert_eq!(format_bytes(1024 * 1024, "B/s"), "1.0 MB/s");
    assert_eq!(format_bytes(2 * 1024 * 1024 * 1024, "B"), "2.00 GiB");
    assert_eq!(format_bytes(2048, "B"), "2 KiB");
    assert_eq!(format_bytes(999, "B"), "999 B");
}

// ── format_eta ───────────────────────────────────────────────────────────────

#[test]
fn format_eta_sub_minute() {
    assert_eq!(format_eta(0), "0s");
    assert_eq!(format_eta(1), "1s");
    assert_eq!(format_eta(45), "45s");
    assert_eq!(format_eta(59), "59s");
}

#[test]
fn format_eta_minutes() {
    assert_eq!(format_eta(60), "1m 00s");
    assert_eq!(format_eta(90), "1m 30s");
    assert_eq!(format_eta(150), "2m 30s");
    assert_eq!(format_eta(3599), "59m 59s");
}

#[test]
fn format_eta_hours() {
    assert_eq!(format_eta(3600), "1h 00m");
    assert_eq!(format_eta(3660), "1h 01m");
    assert_eq!(format_eta(4320), "1h 12m");
    assert_eq!(format_eta(7200), "2h 00m");
}

// ── session_eta ───────────────────────────────────────────────────────────────
//
// session_eta now reads speed from cumulative_speed() (the rolling average the
// OVERVIEW panel also uses) rather than recomputing bytes_done / elapsed.
// Tests that need a non-zero speed inject it via set_cached_speed_for_test.

use crate::app::collection::CollectionPage;

fn page_with_speed(speed: f64, bytes_done: u64, total_bytes: Option<u64>) -> CollectionPage {
    let mut page = CollectionPage::new(1, "test".to_string(), 1);
    page.stats.bytes_downloaded = bytes_done;
    page.stats.total_collection_bytes = total_bytes;
    page.set_cached_speed_for_test(speed);
    page
}

/// No ETA when speed is zero (no active downloads yet).
#[test]
fn session_eta_none_when_no_speed() {
    let mut page = CollectionPage::new(1, "test".to_string(), 1);
    page.stats.bytes_downloaded = 1024 * 1024;
    page.stats.total_collection_bytes = Some(10 * 1024 * 1024);
    // cached speed defaults to 0.0 — no active lines
    assert!(session_eta(&page).is_none());
}

/// No ETA when total collection size is unknown.
#[test]
fn session_eta_none_when_no_total() {
    let page = page_with_speed(5.0 * 1024.0 * 1024.0, 1024 * 1024, None);
    assert!(session_eta(&page).is_none());
}

/// When downloaded >= total, remaining is 0 and ETA should be "0s".
#[test]
fn session_eta_zero_remaining_when_done() {
    let total = 5 * 1024 * 1024u64;
    let page = page_with_speed(5.0 * 1024.0 * 1024.0, total + 1024, Some(total));
    let eta = session_eta(&page).expect("should compute ETA");
    assert_eq!(eta, "0s");
}

/// Happy path: a reasonable ETA string is returned, and speed (read separately
/// from `cumulative_speed`) carries the rolling-average value.
#[test]
fn session_eta_returns_eta() {
    // 5 MB/s rolling speed; 450 MB remaining → 90 s ETA.
    let mb = 1024 * 1024u64;
    let speed = 5.0 * mb as f64;
    let page = page_with_speed(speed, 50 * mb, Some(500 * mb));
    let eta = session_eta(&page).expect("should compute ETA");
    assert_eq!(eta, "1m 30s");
    // Speed still comes from cumulative_speed(), the same source the OVERVIEW
    // panel renders — assert that invariant rather than reading it off session_eta.
    let speed_str = format_bytes(page.cumulative_speed() as u64, "B/s");
    assert!(
        speed_str.contains("MB/s"),
        "expected MB/s in speed: {speed_str:?}"
    );
}

/// A re-run where precheck verified every archive already on disk: nothing is
/// downloaded this run (`bytes_downloaded` stays 0), but the bytes readout on
/// this same line counts `verified_bytes` via `total_downloaded_bytes()`. The
/// ETA must reach the same "done" point, or it stays inflated by the verified
/// amount for the whole run.
#[test]
fn session_eta_counts_verified_bytes_like_the_bytes_readout() {
    let total = 5 * 1024 * 1024u64;
    let mut page = page_with_speed(5.0 * 1024.0 * 1024.0, 0, Some(total));
    page.stats.verified_bytes = total;
    let eta = session_eta(&page).expect("should compute ETA");
    assert_eq!(eta, "0s");
}

/// Mid-range arithmetic pin: both terms non-zero and unequal, so the exact sum
/// matters (unlike the "0s"/saturation-endpoint tests above, which pass under
/// any over-count of the numerator). 1 MiB/s over 10 MiB total, 1 MiB
/// downloaded + 2 MiB verified → 7 MiB remaining → 7s.
#[test]
fn session_eta_arithmetic_uses_both_terms_exactly() {
    let mib = 1024 * 1024u64;
    let mut page = page_with_speed(mib as f64, mib, Some(10 * mib));
    page.stats.verified_bytes = 2 * mib;
    let eta = session_eta(&page).expect("should compute ETA");
    assert_eq!(eta, "7s");
}

// ── rate-limited countdown ────────────────────────────────────────────────────
//
// The base status message ends at "...waiting" with no number (events.rs); the
// single live countdown is appended here. Regression guard: there must be exactly
// ONE number, no frozen second value, and no double space.

/// The exact base produced by `events::emit_status` for a `RateLimited` status:
/// `status::RATE_LIMITED` + `RATE_LIMITED_SUFFIX`.
const RATE_LIMITED_BASE: &str = "rate limited on all mirrors, waiting";

#[test]
fn rate_limited_message_has_single_live_countdown() {
    let msg = rate_limited_message(RATE_LIMITED_BASE, 5);
    assert_eq!(msg, "rate limited on all mirrors, waiting 5s");
    // exactly one run of digits — proves no second frozen number lingers.
    let digit_runs = msg
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .count();
    assert_eq!(digit_runs, 1, "expected one number, got {msg:?}");
    assert!(!msg.contains("  "), "no double space, got {msg:?}");
}

#[test]
fn rate_limited_message_zero_when_deadline_passed() {
    // None cooldown collapses to 0 — never a dangling "waiting" with no value.
    let msg = rate_limited_message(RATE_LIMITED_BASE, 0);
    assert_eq!(msg, "rate limited on all mirrors, waiting 0s");
    assert!(msg.ends_with("0s"));
}

// ── overview key-value rows ─────────────────────────────────────────────────────
//
// Form-row rule: no colon; every value left-aligns at a shared column —
// each label is padded to the widest label width + ≥2 spaces.

/// The four overview labels carry no colon and the label cell is padded so all
/// values start at the same column. "collection" (10 chars) is the widest, so
/// each label cell spans 10 + 2 = 12 columns.
#[test]
fn overview_rows_drop_colon_and_column_align() {
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.uploader = Some("someone".to_string());
    let lines = overview_lines(&page);

    // The first three rows are collection / uploader / output; their label cell
    // (the first span) must be the same width and carry no colon.
    let label_cells: Vec<String> = lines
        .iter()
        .take(3)
        .map(|l| l.spans[0].content.to_string())
        .collect();

    for cell in &label_cells {
        assert!(
            !cell.contains(':'),
            "label cell must drop the colon: {cell:?}"
        );
        assert_eq!(
            cell.chars().count(),
            12,
            "label cell must pad to widest label (collection=10) + 2 spaces: {cell:?}"
        );
        assert!(
            cell.ends_with("  "),
            "value gap must be ≥2 spaces: {cell:?}"
        );
    }
    assert!(label_cells[0].starts_with("collection"));
    assert!(label_cells[1].starts_with("uploader"));
    assert!(label_cells[2].starts_with("output"));
}

// ── overview tally ────────────────────────────────────────────────────────────

#[test]
fn tally_line_shows_all_four_counts() {
    let mut page = CollectionPage::new(1, "test".to_string(), 1);
    page.stats.downloaded = 3;
    page.stats.skipped = 2;
    page.stats.failed = 1;
    page.download_target = 4;
    let text: String = tally_line(&page)
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(text.contains("3 downloaded"), "got {text:?}");
    assert!(text.contains("4 queued"), "got {text:?}");
    assert!(text.contains("2 skipped"), "got {text:?}");
    assert!(text.contains("1 failed"), "got {text:?}");
}
