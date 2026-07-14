#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

/// The mirror returns `file_size` as a JSON **string** (verified live
/// 2026-07-14: `{"file_size": "7678841", …}`), so the plain-number path is the
/// tolerant one, not the expected one. All four shapes must land as a value
/// rather than an error — a parse failure here is indistinguishable from an
/// unreachable mirror at the [`SizeFetcher::fetch_size`] boundary, which would
/// undo the whole point of separating those two.
#[test]
fn file_size_parses_from_every_shape_the_mirror_sends() {
    let cases = [
        // The real wire shape today.
        (r#"{"file_size":"7678841"}"#, Some(7_678_841)),
        // Tolerated: a future build could drop the string quoting.
        (r#"{"file_size":7678841}"#, Some(7_678_841)),
        // The mirror knows the set but has no size for it → a real `None`, which
        // settles the id. Distinct from a failed request, which never gets here.
        (r#"{"file_size":null}"#, None),
        // Column absent entirely (`#[serde(default)]`).
        (r#"{}"#, None),
        // A record with other columns still yields its size.
        (r#"{"id":75,"file_size":"12","artist":"x"}"#, Some(12)),
    ];
    for (raw, expected) in cases {
        let parsed: BeatmapsetResponse =
            serde_json::from_str(raw).unwrap_or_else(|err| panic!("{raw} must parse: {err}"));
        assert_eq!(parsed.file_size, expected, "{raw}");
    }
}

#[test]
fn a_non_numeric_file_size_is_an_error_not_a_silent_zero() {
    // Junk must fail loudly: coercing it to 0 or `None` would report a real set
    // as sizeless forever (`fetch_size`'s `Ok(None)` is never re-probed).
    for raw in [r#"{"file_size":"not a number"}"#, r#"{"file_size":"-5"}"#] {
        assert!(
            serde_json::from_str::<BeatmapsetResponse>(raw).is_err(),
            "{raw} must not deserialize"
        );
    }
}

/// The aggregate estimate folds a probe failure and a sizeless set together into
/// `missing_count` — `fetch_size` exists precisely because this shape can't tell
/// those apart. Pin the arithmetic so routing `fetch_sizes` through the per-id
/// path didn't move the estimate.
#[test]
fn unknown_sets_are_billed_at_the_average_of_what_landed() {
    // Two known (10 + 20 = 30, average 15) and two unknown → 30 + 2×15.
    let result = estimate_from_probes(&[Some(10), Some(20), None, None]);
    assert_eq!(result.total_bytes, 60);
    assert_eq!(result.missing_count, 2);
}

#[test]
fn a_fully_known_batch_is_summed_exactly() {
    let result = estimate_from_probes(&[Some(10), Some(20)]);
    assert_eq!(
        result.total_bytes, 30,
        "nothing missing → no estimate at all"
    );
    assert_eq!(result.missing_count, 0);
}

#[test]
fn a_fully_unknown_batch_estimates_nothing_rather_than_guessing() {
    // No sample to average over, so the total stays 0 and the caller's `~X` label
    // drops out entirely — better than inventing a number from no data.
    let result = estimate_from_probes(&[None, None, None]);
    assert_eq!(result.total_bytes, 0);
    assert_eq!(result.missing_count, 3);
}

#[test]
fn an_empty_batch_is_zero_not_a_divide_by_zero() {
    let result = estimate_from_probes(&[]);
    assert_eq!(result.total_bytes, 0);
    assert_eq!(result.missing_count, 0);
}
