use super::{Unwrap, unwrap_envelope};
use crate::ArchiveValidation;
use crate::validation::{ensure_valid_archive, minimal_zip_bytes_for_test};
use crate::worker::finalize_md5;
use md5::{Digest, Md5};
use std::io::Write;
use tempfile::NamedTempFile;

/// The dash-boundary line as served live by direct.nzbasic.com (2026-07-08).
const BOUNDARY_LINE: &str = "--------------------------606737fd14846d6b";

fn envelope_bytes(payload: &[u8], trailing_crlf: bool) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(BOUNDARY_LINE.as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"39804 xi - FREEDOM DiVE.osz\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(payload);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(BOUNDARY_LINE.as_bytes());
    body.extend_from_slice(b"--");
    if trailing_crlf {
        body.extend_from_slice(b"\r\n");
    }
    body
}

fn write_temp(bytes: &[u8]) -> NamedTempFile {
    let mut tmp = NamedTempFile::new().unwrap();
    tmp.write_all(bytes).unwrap();
    tmp
}

fn payload_md5(payload: &[u8]) -> Box<str> {
    let mut hasher = Md5::new();
    hasher.update(payload);
    finalize_md5(hasher)
}

#[tokio::test]
async fn strips_envelope_to_valid_archive() {
    let payload = minimal_zip_bytes_for_test();
    let tmp = write_temp(&envelope_bytes(&payload, true));

    match unwrap_envelope(tmp.path()).await.unwrap() {
        Unwrap::Stripped { md5, len } => {
            assert_eq!(len, payload.len() as u64);
            assert_eq!(md5, payload_md5(&payload));
        }
        other => panic!("expected Stripped, got {other:?}"),
    }

    assert_eq!(std::fs::read(tmp.path()).unwrap(), payload);
    assert!(
        ensure_valid_archive(tmp.path(), ArchiveValidation::Eocd)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn strips_envelope_across_multiple_copy_chunks() {
    // Larger than COPY_BUF and not chunk-aligned, so the copy-down loop runs
    // more than once and ends on a partial chunk — the path every real
    // multi-MB download takes.
    let payload: Vec<u8> = (0..super::COPY_BUF + 12_347)
        .map(|i| (i % 251) as u8)
        .collect();
    let tmp = write_temp(&envelope_bytes(&payload, true));

    match unwrap_envelope(tmp.path()).await.unwrap() {
        Unwrap::Stripped { md5, len } => {
            assert_eq!(len, payload.len() as u64);
            assert_eq!(md5, payload_md5(&payload));
        }
        other => panic!("expected Stripped, got {other:?}"),
    }
    assert_eq!(std::fs::read(tmp.path()).unwrap(), payload);
}

#[tokio::test]
async fn strips_envelope_without_final_crlf() {
    let payload = minimal_zip_bytes_for_test();
    let tmp = write_temp(&envelope_bytes(&payload, false));

    assert!(matches!(
        unwrap_envelope(tmp.path()).await.unwrap(),
        Unwrap::Stripped { .. }
    ));
    assert_eq!(std::fs::read(tmp.path()).unwrap(), payload);
}

#[tokio::test]
async fn bare_archive_passes_through_untouched() {
    let payload = minimal_zip_bytes_for_test();
    let tmp = write_temp(&payload);

    assert!(matches!(
        unwrap_envelope(tmp.path()).await.unwrap(),
        Unwrap::Bare
    ));
    assert_eq!(std::fs::read(tmp.path()).unwrap(), payload);
}

#[tokio::test]
async fn json_error_body_is_a_miss() {
    let tmp = write_temp(br#"{"error":"This Set does not exist."}"#);
    assert!(matches!(
        unwrap_envelope(tmp.path()).await.unwrap(),
        Unwrap::Miss
    ));
}

#[tokio::test]
async fn html_body_is_a_miss() {
    let tmp = write_temp(b"<!doctype html>\n<html><title>Not Found</title></html>");
    assert!(matches!(
        unwrap_envelope(tmp.path()).await.unwrap(),
        Unwrap::Miss
    ));
}

#[tokio::test]
async fn missing_closing_boundary_is_malformed() {
    let payload = minimal_zip_bytes_for_test();
    let mut body = envelope_bytes(&payload, false);
    // Chop the closing delimiter off: a truncated download must not pass as
    // a stripped archive.
    body.truncate(body.len() - (2 + BOUNDARY_LINE.len() + 2));
    let tmp = write_temp(&body);

    let err = unwrap_envelope(tmp.path()).await.unwrap_err();
    assert!(err.contains("closing boundary"), "unexpected error: {err}");
}

#[tokio::test]
async fn unterminated_part_headers_is_malformed() {
    let mut body = Vec::new();
    body.extend_from_slice(BOUNDARY_LINE.as_bytes());
    body.extend_from_slice(b"\r\nContent-Disposition: form-data");
    let tmp = write_temp(&body);

    let err = unwrap_envelope(tmp.path()).await.unwrap_err();
    assert!(err.contains("part headers"), "unexpected error: {err}");
}

/// Network-gated: the whole pipeline (miss rotation + envelope strip +
/// validation + finalize) against the live CDN as the only mirror.
///
/// Run with: `cargo test --manifest-path osu-downloader/Cargo.toml
/// --all-features live_nzbasic -- --ignored`
#[tokio::test]
#[ignore = "network: hits direct.nzbasic.com"]
async fn live_nzbasic_pipeline_downloads_a_cached_set() {
    let dir = tempfile::tempdir().unwrap();
    let downloader = crate::Downloader::builder()
        .mirror(crate::Mirror::nzbasic())
        .archive_validation(ArchiveValidation::Eocd)
        .build()
        .unwrap();

    // 39804 is cached (probed live); u32::MAX is a guaranteed miss that must
    // come back as unavailable-on-mirrors, not a hard failure.
    let summary = downloader
        .download_many([39804, u32::MAX], dir.path())
        .wait()
        .await
        .unwrap();

    assert_eq!(summary.downloaded, vec![39804], "{:?}", summary.failed);
    assert!(matches!(
        summary.skipped.as_slice(),
        [(id, crate::Skip::UnavailableOnMirrors)] if *id == u32::MAX
    ));
    let downloaded: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(downloaded.len(), 1);
    assert!(
        ensure_valid_archive(&downloaded[0], ArchiveValidation::Eocd)
            .await
            .is_ok()
    );
}

/// Network-gated: verifies the live CDN still serves the documented envelope
/// shape end-to-end (fetch → unwrap → EOCD-validate).
///
/// Run with: `cargo test --manifest-path osu-downloader/Cargo.toml
/// --all-features live_nzbasic -- --ignored`
#[tokio::test]
#[ignore = "network: hits direct.nzbasic.com"]
async fn live_nzbasic_envelope_unwraps_to_valid_archive() {
    let body = reqwest::get("https://direct.nzbasic.com/39804.osz")
        .await
        .expect("CDN reachable")
        .bytes()
        .await
        .expect("body streams");
    let tmp = write_temp(&body);

    match unwrap_envelope(tmp.path()).await.unwrap() {
        Unwrap::Stripped { len, .. } => assert!(len > 0),
        other => panic!("expected Stripped from the live CDN, got {other:?}"),
    }
    assert!(
        ensure_valid_archive(tmp.path(), ArchiveValidation::Eocd)
            .await
            .is_ok()
    );
}
