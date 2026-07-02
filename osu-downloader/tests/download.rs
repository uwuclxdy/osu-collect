use super::{
    BeatmapsetDownloadCallbacks, BeatmapsetDownloadOutcome, DownloadParams, FinalizeResult,
    InlineWait, download_beatmapset, finalize_download, inline_wait, is_archive_content_type,
    parse_retry_after, probe_download_size, sanitize_filename, size_from_content_range,
    sleep_cancelable,
};
use crate::config::INLINE_WAIT_MAX;
use crate::mirrors::pool::MirrorPool;
use crate::validation::minimal_zip_bytes_for_test;
use crate::{ArchiveValidation, Mirror, MirrorKind, OnExists, Skip, Status};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn default_params<'a>(
    beatmapset_id: u32,
    output_dir: &'a Path,
    client: &'a reqwest::Client,
    mirror_pool: &'a MirrorPool,
    cancel_rx: tokio::sync::watch::Receiver<bool>,
) -> DownloadParams<'a> {
    DownloadParams {
        beatmapset_id,
        output_dir,
        client,
        mirror_pool,
        archive_validation: ArchiveValidation::Off,
        progress_timeout: Duration::from_secs(1),
        sanitize_filenames: true,
        on_exists: OnExists::Skip,
        callbacks: BeatmapsetDownloadCallbacks::default(),
        cancel_rx,
        defer_signal: Arc::new(tokio::sync::Notify::new()),
        drop_signal: Arc::new(tokio::sync::Notify::new()),
        inline_wait_max: INLINE_WAIT_MAX,
        rate_limit_skip_after: None,
        #[cfg(feature = "instrument")]
        attempt_observer: None,
        #[cfg(feature = "instrument")]
        session_start: Instant::now(),
    }
}

#[test]
fn test_sanitize_filename() {
    assert_eq!(sanitize_filename(None, 123), "123.osz");
    assert_eq!(
        sanitize_filename(Some("test/file.osz"), 456),
        "test_file.osz"
    );
    assert_eq!(sanitize_filename(Some(".."), 789), "789.osz");
    assert_eq!(sanitize_filename(Some("."), 789), "789.osz");
    assert_eq!(sanitize_filename(Some(""), 789), "789.osz");
    assert_eq!(sanitize_filename(Some("./map.osz"), 789), "789.osz");
    assert_eq!(sanitize_filename(Some("../etc/passwd"), 789), "789.osz");
    assert_eq!(
        sanitize_filename(Some("normal map.osz"), 789),
        "normal map.osz"
    );
    assert_eq!(
        sanitize_filename(Some("ユニコード.osz"), 789),
        "ユニコード.osz"
    );
    // single-char inputs: clean char passes through, forbidden char is replaced (not fallback)
    assert_eq!(sanitize_filename(Some("a"), 1), "a");
    assert_eq!(sanitize_filename(Some("/"), 1), "_");
    // all nine forbidden chars in one string
    assert_eq!(sanitize_filename(Some("/\\:*?\"<>|"), 1), "_________");
    // multibyte UTF-8 mixed with forbidden ASCII
    assert_eq!(
        sanitize_filename(Some("héllo:wörld.osz"), 1),
        "héllo_wörld.osz"
    );
    // longest expected input (~200 chars) — no forbidden chars
    let long = "9999999 A Very Long Artist Name With Spaces - A Very Long Song Title \
                That Goes On And On Including Extra Details [Expert Difficulty].osz";
    assert_eq!(sanitize_filename(Some(long), 9_999_999), long);
}

#[test]
fn test_extract_filename() {
    use super::parse_content_disposition;
    assert_eq!(
        parse_content_disposition("attachment; filename=\"test.osz\""),
        Some("test.osz".to_string())
    );

    assert_eq!(
        parse_content_disposition("attachment; filename*=UTF-8''test%20file.osz"),
        Some("test file.osz".to_string())
    );

    assert_eq!(
        parse_content_disposition(r#"attachment; filename="foo\"bar.osz""#),
        Some(r#"foo"bar.osz"#.to_string())
    );

    assert_eq!(
        parse_content_disposition(r#"attachment; filename="foo\\bar.osz""#),
        Some(r#"foo\bar.osz"#.to_string())
    );

    assert_eq!(
        parse_content_disposition("attachment; filename=plain.osz"),
        Some("plain.osz".to_string())
    );

    assert_eq!(
        parse_content_disposition(r#"attachment; filename="artist - title; diff.osz""#),
        Some("artist - title; diff.osz".to_string())
    );

    assert_eq!(
        parse_content_disposition(
            "attachment; filename=plain.osz; filename*=utf-8''encoded%20name.osz"
        ),
        Some("encoded name.osz".to_string())
    );

    assert_eq!(
        parse_content_disposition(
            "attachment; filename=fallback.osz; filename*=iso-8859-1''ignored.osz"
        ),
        Some("fallback.osz".to_string())
    );

    assert_eq!(
        parse_content_disposition("attachment; FILENAME=upper.osz"),
        Some("upper.osz".to_string())
    );
}

#[test]
fn archive_content_type_accepts_known_archive_mimes() {
    assert!(is_archive_content_type("application/x-osu-beatmap-archive"));
    assert!(is_archive_content_type(
        "application/octet-stream; charset=binary"
    ));
    assert!(is_archive_content_type("binary/octet-stream"));
    assert!(is_archive_content_type("application/zip"));
    assert!(is_archive_content_type("application/x-zip-compressed"));
    assert!(!is_archive_content_type("text/html"));
    assert!(!is_archive_content_type("application/json"));
    // mixed-case variants must be accepted without prior lowercasing
    assert!(is_archive_content_type("Application/Zip"));
    assert!(is_archive_content_type("APPLICATION/OCTET-STREAM"));
    assert!(is_archive_content_type("Binary/Octet-Stream"));
    assert!(is_archive_content_type(
        "Application/X-Osu-Beatmap-Archive; charset=binary"
    ));
    // wrong type must still be rejected regardless of case
    assert!(!is_archive_content_type("Text/HTML"));
}

#[test]
fn size_from_content_range_uses_complete_length() {
    assert_eq!(
        size_from_content_range("bytes 0-0/24413678"),
        Some(24_413_678)
    );
    assert_eq!(size_from_content_range("bytes 0-3/*"), None);
    assert_eq!(size_from_content_range("invalid"), None);
}

#[tokio::test]
async fn range_probe_discovers_redirected_download_size() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            if request.starts_with("GET /mirror/") {
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 302 Found\r\nLocation: http://{addr}/archive/42\r\nContent-Length: 0\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .unwrap();
            } else if request.starts_with("GET /archive/") {
                stream.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/10000000\r\nContent-Length: 1\r\n\r\nP").unwrap();
            }
        }
    });

    let client = reqwest::Client::new();
    let mirror = Mirror::custom(format!("http://{addr}/mirror/{{id}}")).unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror.clone()]);
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let dir = tempfile::tempdir().unwrap();
    let params = default_params(42, dir.path(), &client, &mirror_pool, cancel_rx);

    assert_eq!(
        probe_download_size(&mirror, &params).await,
        Some(10_000_000)
    );
    server.join().unwrap();
}

#[tokio::test]
async fn probe_preserves_range_across_multiple_redirects() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            if request.starts_with("GET /api/") {
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 302 Found\r\nLocation: http://{addr}/dl/997762\r\nContent-Length: 0\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .unwrap();
            } else if request.starts_with("GET /dl/") {
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 302 Found\r\nLocation: http://{addr}/s3/997762.osz\r\nContent-Length: 0\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .unwrap();
            } else if request.starts_with("GET /s3/") {
                // hyper sends header names lowercase on the wire; compare case-insensitively.
                assert!(request.to_lowercase().contains("range: bytes=0-0"));
                stream.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/44911016\r\nContent-Length: 1\r\n\r\nP").unwrap();
            }
        }
    });

    let client = reqwest::Client::new();
    let mirror = Mirror::custom(format!("http://{addr}/api/{{id}}")).unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror.clone()]);
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let dir = tempfile::tempdir().unwrap();
    let params = default_params(997762, dir.path(), &client, &mirror_pool, cancel_rx);

    assert_eq!(
        probe_download_size(&mirror, &params).await,
        Some(44_911_016)
    );
    server.join().unwrap();
}

#[tokio::test]
async fn completion_uses_probed_size_when_download_is_chunked() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            if request.contains("Range: bytes=0-0") {
                stream.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/262144\r\nContent-Length: 1\r\n\r\nP").unwrap();
            } else {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=42.osz\r\nTransfer-Encoding: chunked\r\n\r\n40000\r\n").unwrap();
                stream.write_all(&vec![b'a'; 262_144]).unwrap();
                let _ = stream.write_all(b"\r\n0\r\n\r\n");
            }
        }
    });

    let client = reqwest::Client::new();
    let mirror = Mirror::custom(format!("http://{addr}/download/{{id}}")).unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let progress = Arc::new(Mutex::new(Vec::new()));
    let progress_events = progress.clone();
    let dir = tempfile::tempdir().unwrap();

    let (outcome, _) = download_beatmapset(DownloadParams {
        callbacks: BeatmapsetDownloadCallbacks {
            progress: Some(Arc::new(move |downloaded, total| {
                progress_events.lock().unwrap().push((downloaded, total));
            })),
            status: None,
        },
        ..default_params(42, dir.path(), &client, &mirror_pool, cancel_rx)
    })
    .await;

    assert!(matches!(
        outcome,
        BeatmapsetDownloadOutcome::Success {
            size_bytes: 262_144,
            ..
        }
    ));
    server.join().unwrap();
}

#[tokio::test]
async fn skip_existing_file_does_not_emit_downloading() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=custom.osz\r\nContent-Length: 0\r\n\r\n",
            )
            .unwrap();
    });

    let client = reqwest::Client::new();
    let mirror = Mirror::custom(format!("http://{addr}/download/{{id}}")).unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let status_events = statuses.clone();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("custom.osz"), b"existing").unwrap();

    let (outcome, _) = download_beatmapset(DownloadParams {
        callbacks: BeatmapsetDownloadCallbacks {
            progress: None,
            status: Some(Arc::new(move |status| {
                status_events.lock().unwrap().push(status);
            })),
        },
        on_exists: OnExists::Skip,
        ..default_params(42, dir.path(), &client, &mirror_pool, cancel_rx)
    })
    .await;

    assert!(matches!(
        outcome,
        BeatmapsetDownloadOutcome::Skipped {
            reason: Skip::AlreadyExists
        }
    ));
    assert!(
        !statuses
            .lock()
            .unwrap()
            .iter()
            .any(|status| matches!(status, Status::Downloading { .. }))
    );
    server.join().unwrap();
}

#[tokio::test]
async fn finalize_download_preserves_existing_output() {
    let dir = std::env::temp_dir().join(format!(
        "osu-downloader-finalize-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
    ));
    tokio::fs::create_dir(&dir).await.unwrap();

    let temp_path = dir.join("123.osz.tmp");
    let output_path = dir.join("123.osz");
    tokio::fs::write(&temp_path, b"new").await.unwrap();
    tokio::fs::write(&output_path, b"old").await.unwrap();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let finalized = finalize_download(&temp_path, &output_path, &cancel_rx).await;

    assert!(matches!(finalized, FinalizeResult::AlreadyExists));
    assert_eq!(tokio::fs::read(&output_path).await.unwrap(), b"old");
    assert!(!tokio::fs::try_exists(&temp_path).await.unwrap());

    tokio::fs::remove_dir_all(&dir).await.unwrap();
}

#[tokio::test]
async fn rate_limit_status_suppressed_when_other_mirror_succeeds() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            if request.starts_with("GET /rate/") {
                stream
                    .write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            } else {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=321.osz\r\nContent-Length: 4\r\n\r\ndata").unwrap();
            }
        }
    });

    let rate_limited =
        Mirror::with_kind_and_template(MirrorKind::Nerinyan, format!("http://{addr}/rate/{{id}}"));
    let healthy =
        Mirror::with_kind_and_template(MirrorKind::OsuDirect, format!("http://{addr}/ok/{{id}}"));
    let mirror_pool = MirrorPool::new(vec![rate_limited, healthy]);
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let statuses: Arc<Mutex<Vec<Status>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = statuses.clone();
    let callbacks = BeatmapsetDownloadCallbacks {
        progress: None,
        status: Some(Arc::new(move |event| {
            recorder.lock().unwrap().push(event);
        })),
    };

    let (outcome, _) = download_beatmapset(DownloadParams {
        callbacks,
        ..default_params(321, dir.path(), &client, &mirror_pool, cancel_rx)
    })
    .await;

    assert!(matches!(outcome, BeatmapsetDownloadOutcome::Success { .. }));
    let recorded = statuses.lock().unwrap();
    assert!(
        !recorded
            .iter()
            .any(|event| matches!(event, Status::RateLimited { .. })),
        "rate-limit status must not flash when a sibling mirror succeeds: {recorded:?}"
    );
    server.join().unwrap();
}

#[tokio::test]
async fn rate_limit_status_emitted_once_when_all_mirrors_throttled() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let rate_a_hits = Arc::new(AtomicUsize::new(0));
    let rate_b_hits = Arc::new(AtomicUsize::new(0));
    let server_a = rate_a_hits.clone();
    let server_b = rate_b_hits.clone();
    let server = thread::spawn(move || {
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            if request.starts_with("GET /a/") {
                let hit = server_a.fetch_add(1, Ordering::SeqCst);
                if hit >= 1 {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=555.osz\r\nContent-Length: 4\r\n\r\ndata").unwrap();
                    break;
                }
                stream
                    .write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            } else if request.starts_with("GET /b/") {
                server_b.fetch_add(1, Ordering::SeqCst);
                stream
                    .write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
        }
    });

    let mirror_a =
        Mirror::with_kind_and_template(MirrorKind::Nerinyan, format!("http://{addr}/a/{{id}}"));
    let mirror_b =
        Mirror::with_kind_and_template(MirrorKind::OsuDirect, format!("http://{addr}/b/{{id}}"));
    let mirror_pool = MirrorPool::new(vec![mirror_a, mirror_b]);
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let statuses: Arc<Mutex<Vec<Status>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = statuses.clone();
    let callbacks = BeatmapsetDownloadCallbacks {
        progress: None,
        status: Some(Arc::new(move |event| {
            recorder.lock().unwrap().push(event);
        })),
    };

    let (outcome, _) = download_beatmapset(DownloadParams {
        callbacks,
        ..default_params(555, dir.path(), &client, &mirror_pool, cancel_rx)
    })
    .await;

    assert!(matches!(outcome, BeatmapsetDownloadOutcome::Success { .. }));
    let recorded = statuses.lock().unwrap();
    let rate_limit_events: Vec<_> = recorded
        .iter()
        .filter(|event| matches!(event, Status::RateLimited { .. }))
        .collect();
    assert_eq!(
        rate_limit_events.len(),
        1,
        "exactly one rate-limit event expected once every mirror is throttled: {recorded:?}"
    );
    server.join().unwrap();
}

#[tokio::test]
async fn auto_defer_fires_once_cumulative_inline_budget_is_reached() {
    // A slot pre-cooled below the inline threshold is waited out inline; a tiny
    // budget crosses during that wait, so the map defers itself (returned to the
    // queue) instead of being dropped. The mirror is never contacted.
    let mirror = Mirror::custom("http://127.0.0.1:1/d/{id}").unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    // Cooldown well above the budget so it stays live through the async preamble;
    // the budget crosses first and defers the map long before the cooldown ends.
    mirror_pool.mark_rate_limited(0, Some(Duration::from_millis(500)));
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let start = Instant::now();
    let (outcome, _) = download_beatmapset(DownloadParams {
        rate_limit_skip_after: Some(Duration::from_millis(5)),
        // Explicit threshold so the 500 ms cooldown stays inline-waitable no
        // matter where the global INLINE_WAIT_MAX moves.
        inline_wait_max: Duration::from_secs(2),
        ..default_params(555, dir.path(), &client, &mirror_pool, cancel_rx)
    })
    .await;

    assert!(
        matches!(outcome, BeatmapsetDownloadOutcome::Deferred { .. }),
        "expected auto-defer once the cumulative inline wait crossed the budget, got {outcome:?}"
    );
    assert!(
        start.elapsed() < Duration::from_millis(300),
        "auto-defer must not wait out the whole cooldown"
    );
}

#[tokio::test]
async fn spacing_waits_never_count_toward_the_auto_defer_budget() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=9.osz\r\nContent-Length: 4\r\n\r\ndata").unwrap();
    });

    let mirror = Mirror::custom(format!("http://{addr}/d/{{id}}")).unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    // Spend the slot's send token ~1 s into the future: the map faces a pure
    // spacing wait (no cooldown) far longer than the zero budget. If spacing
    // counted toward the budget the map would defer; it must instead sleep the
    // spacing out and succeed. The window is generous so the async preamble
    // cannot swallow it under parallel test load.
    let _ = mirror_pool.acquire_at(&[0], Instant::now() + Duration::from_secs(1));
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let start = Instant::now();
    let (outcome, _) = download_beatmapset(DownloadParams {
        rate_limit_skip_after: Some(Duration::ZERO),
        inline_wait_max: Duration::from_secs(5),
        ..default_params(9, dir.path(), &client, &mirror_pool, cancel_rx)
    })
    .await;

    assert!(
        matches!(outcome, BeatmapsetDownloadOutcome::Success { .. }),
        "a pure spacing wait must not trip the auto-defer budget, got {outcome:?}"
    );
    assert!(
        start.elapsed() >= Duration::from_millis(500),
        "the spacing wait itself must still be honored"
    );
    server.join().unwrap();
}

#[tokio::test]
async fn spacing_wait_is_immune_to_defer_and_drop_signals() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=11.osz\r\nContent-Length: 4\r\n\r\ndata").unwrap();
    });

    let mirror = Mirror::custom(format!("http://{addr}/d/{{id}}")).unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    // A healthy map waiting out a ~300 ms send-token spacing (no cooldown).
    let _ = mirror_pool.acquire_at(&[0], Instant::now() + Duration::from_millis(300));
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let params = DownloadParams {
        inline_wait_max: Duration::from_secs(2),
        ..default_params(11, dir.path(), &client, &mirror_pool, cancel_rx)
    };
    let defer = params.defer_signal.clone();
    let drop_signal = params.drop_signal.clone();

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Both presses land mid-spacing-wait; a healthy map must ignore them.
        // A drop here would terminally discard a download that was never
        // rate-limited.
        defer.notify_waiters();
        drop_signal.notify_waiters();
    });

    let (outcome, _) = download_beatmapset(params).await;
    assert!(
        matches!(outcome, BeatmapsetDownloadOutcome::Success { .. }),
        "defer/drop must not touch a map in a plain spacing wait, got {outcome:?}"
    );
    server.join().unwrap();
}

#[tokio::test]
async fn rate_limited_mirror_is_retried_after_other_mirrors_fail() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let rate_hits = Arc::new(AtomicUsize::new(0));
    let missing_hits = Arc::new(AtomicUsize::new(0));
    let server_rate_hits = rate_hits.clone();
    let server_missing_hits = missing_hits.clone();
    let server = thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            if request.starts_with("GET /rate/") {
                let hit = server_rate_hits.fetch_add(1, Ordering::SeqCst);
                if hit == 0 {
                    stream
                        .write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n")
                        .unwrap();
                } else {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=123.osz\r\nContent-Length: 4\r\n\r\ndata").unwrap();
                }
            } else if request.starts_with("GET /missing/") {
                server_missing_hits.fetch_add(1, Ordering::SeqCst);
                stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            } else {
                stream
                    .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
        }
    });

    let rate_limited_then_ok =
        Mirror::with_kind_and_template(MirrorKind::Nerinyan, format!("http://{addr}/rate/{{id}}"));
    let missing = Mirror::with_kind_and_template(
        MirrorKind::OsuDirect,
        format!("http://{addr}/missing/{{id}}"),
    );
    let mirror_pool = MirrorPool::new(vec![rate_limited_then_ok, missing]);
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let (outcome, _) = download_beatmapset(default_params(
        123,
        dir.path(),
        &client,
        &mirror_pool,
        cancel_rx,
    ))
    .await;

    assert!(matches!(outcome, BeatmapsetDownloadOutcome::Success { .. }));
    assert_eq!(rate_hits.load(Ordering::SeqCst), 2);
    assert_eq!(missing_hits.load(Ordering::SeqCst), 1);
    server.join().unwrap();
}

#[tokio::test]
async fn verify_archive_records_nonzero_duration_when_enabled() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let zip_bytes = minimal_zip_bytes_for_test();
    let len = zip_bytes.len();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=99.osz\r\nContent-Length: {len}\r\n\r\n"
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(&zip_bytes).unwrap();
    });

    let client = reqwest::Client::new();
    let mirror = Mirror::custom(format!("http://{addr}/dl/{{id}}")).unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let dir = tempfile::tempdir().unwrap();

    let (outcome, _) = download_beatmapset(DownloadParams {
        archive_validation: ArchiveValidation::Eocd,
        ..default_params(99, dir.path(), &client, &mirror_pool, cancel_rx)
    })
    .await;

    match outcome {
        BeatmapsetDownloadOutcome::Success {
            verify_duration_us, ..
        } => assert!(
            verify_duration_us > 0,
            "verify_duration_us must be non-zero when verification runs (got {verify_duration_us}us)"
        ),
        other => panic!("expected Success outcome, got {other:?}"),
    }
    server.join().unwrap();
}

#[tokio::test]
async fn backoff_cancelled_before_expiry() {
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = cancel_tx.send(true);
    });

    let start = Instant::now();
    assert!(sleep_cancelable(Duration::from_secs(1), &cancel_rx).await);

    assert!(
        start.elapsed() < Duration::from_millis(200),
        "backoff should have been cut short by cancel signal"
    );
}

#[test]
fn parse_retry_after_reads_delta_seconds() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "120".parse().unwrap());
    assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(120)));
}

#[test]
fn parse_retry_after_reads_http_date() {
    // An HTTP-date a minute out yields roughly a minute; a past date yields None.
    let future = httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_secs(60));
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, future.parse().unwrap());
    let parsed = parse_retry_after(&headers).expect("future date parses");
    assert!(
        parsed <= Duration::from_secs(61) && parsed >= Duration::from_secs(55),
        "expected ~60s, got {parsed:?}"
    );

    let past = httpdate::fmt_http_date(std::time::SystemTime::now() - Duration::from_secs(60));
    let mut past_headers = reqwest::header::HeaderMap::new();
    past_headers.insert(reqwest::header::RETRY_AFTER, past.parse().unwrap());
    assert_eq!(parse_retry_after(&past_headers), None);
}

#[test]
fn parse_retry_after_absent_is_none() {
    assert_eq!(parse_retry_after(&reqwest::header::HeaderMap::new()), None);
}

#[tokio::test]
async fn inline_wait_drop_wins_over_defer() {
    use std::future::Future;

    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let defer = Arc::new(tokio::sync::Notify::new());
    let drop_signal = Arc::new(tokio::sync::Notify::new());

    let fut = inline_wait(Duration::from_secs(60), &cancel_rx, &defer, &drop_signal);
    tokio::pin!(fut);
    // Register every select branch with one poll, then fire BOTH signals before
    // the next poll (defer first, adversarially). The biased order must still
    // resolve to the drop.
    std::future::poll_fn(|cx| {
        assert!(fut.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    defer.notify_waiters();
    drop_signal.notify_waiters();

    assert!(matches!(fut.await, InlineWait::Dropped));
}

#[tokio::test]
async fn inline_wait_defer_wakes_a_parked_map() {
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let defer = Arc::new(tokio::sync::Notify::new());
    let drop_signal = Arc::new(tokio::sync::Notify::new());
    let d = defer.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        d.notify_waiters();
    });

    let outcome = inline_wait(Duration::from_secs(60), &cancel_rx, &defer, &drop_signal).await;
    assert!(matches!(outcome, InlineWait::Deferred));
}

#[tokio::test]
async fn inline_wait_cancel_aborts_a_parked_map() {
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let defer = Arc::new(tokio::sync::Notify::new());
    let drop_signal = Arc::new(tokio::sync::Notify::new());
    let _ = cancel_tx.send(true);

    let outcome = inline_wait(Duration::from_secs(60), &cancel_rx, &defer, &drop_signal).await;
    assert!(matches!(outcome, InlineWait::Cancelled));
}

#[tokio::test]
async fn defer_signal_requeues_a_parked_map() {
    // The map parks on a long-but-inline cooldown; the defer signal returns it
    // to the queue rather than dropping it.
    let mirror = Mirror::custom("http://127.0.0.1:1/d/{id}").unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    mirror_pool.mark_rate_limited(0, Some(Duration::from_secs(1)));
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let params = default_params(1, dir.path(), &client, &mirror_pool, cancel_rx);
    let defer = params.defer_signal.clone();

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        defer.notify_waiters();
    });

    let (outcome, _) = download_beatmapset(params).await;
    assert!(matches!(
        outcome,
        BeatmapsetDownloadOutcome::Deferred { .. }
    ));
}

#[tokio::test]
async fn drop_signal_discards_a_parked_map() {
    let mirror = Mirror::custom("http://127.0.0.1:1/d/{id}").unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    mirror_pool.mark_rate_limited(0, Some(Duration::from_secs(1)));
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let params = default_params(1, dir.path(), &client, &mirror_pool, cancel_rx);
    let drop_signal = params.drop_signal.clone();

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop_signal.notify_waiters();
    });

    let (outcome, _) = download_beatmapset(params).await;
    assert!(matches!(
        outcome,
        BeatmapsetDownloadOutcome::Skipped {
            reason: Skip::RateLimitSkipped
        }
    ));
}

#[tokio::test]
async fn long_cooldown_defers_to_the_batch() {
    // A cooldown longer than inline_wait_max returns Deferred without waiting.
    let mirror = Mirror::custom("http://127.0.0.1:1/d/{id}").unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    mirror_pool.mark_rate_limited(0, Some(Duration::from_secs(1)));
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let start = Instant::now();
    let (outcome, _) = download_beatmapset(DownloadParams {
        inline_wait_max: Duration::ZERO,
        ..default_params(1, dir.path(), &client, &mirror_pool, cancel_rx)
    })
    .await;

    assert!(matches!(
        outcome,
        BeatmapsetDownloadOutcome::Deferred {
            rate_limited: true,
            ..
        }
    ));
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "a deferred map must not park on the cooldown"
    );
}

#[tokio::test]
async fn long_spacing_wait_defers_without_a_rate_limit_flag() {
    // A grown send-token spacing longer than inline_wait_max defers the map, but
    // as a healthy (never-429'd) map: `rate_limited` must be false so the batch
    // never advances its drop-eligibility pass counter for it.
    let mirror = Mirror::custom("http://127.0.0.1:1/d/{id}").unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    // Spend the send token ~1 s out with no cooldown: a pure spacing wait.
    let _ = mirror_pool.acquire_at(&[0], Instant::now() + Duration::from_secs(1));
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let start = Instant::now();
    let (outcome, _) = download_beatmapset(DownloadParams {
        inline_wait_max: Duration::from_millis(100),
        ..default_params(1, dir.path(), &client, &mirror_pool, cancel_rx)
    })
    .await;

    assert!(
        matches!(
            outcome,
            BeatmapsetDownloadOutcome::Deferred {
                rate_limited: false,
                ..
            }
        ),
        "a pure spacing defer must not be flagged rate-limited, got {outcome:?}"
    );
    assert!(
        start.elapsed() < Duration::from_millis(300),
        "a spacing defer past the inline max must not park on the wait"
    );
}

#[cfg(feature = "instrument")]
#[tokio::test]
async fn validation_failure_records_definitive_not_success() {
    use crate::instrument::{AttemptObserver, AttemptOutcome, AttemptRecord};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    // 200 OK carrying a non-archive body: it clears the HTTP check but fails ZIP
    // validation, so the attempt must be recorded Definitive, never Success.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=1.osz\r\nContent-Length: 8\r\n\r\nnotazip!").unwrap();
    });

    let mirror = Mirror::custom(format!("http://{addr}/d/{{id}}")).unwrap();
    let mirror_pool = MirrorPool::new(vec![mirror]);
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let outcomes: Arc<Mutex<Vec<AttemptOutcome>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = outcomes.clone();
    let observer = AttemptObserver::new(move |record: AttemptRecord| {
        sink.lock().unwrap().push(record.outcome);
    });

    let (outcome, _) = download_beatmapset(DownloadParams {
        archive_validation: ArchiveValidation::Magic,
        attempt_observer: Some(observer),
        ..default_params(1, dir.path(), &client, &mirror_pool, cancel_rx)
    })
    .await;

    assert!(
        matches!(outcome, BeatmapsetDownloadOutcome::Failed { .. }),
        "a validation failure is a definitive failure, got {outcome:?}"
    );
    let recorded = outcomes.lock().unwrap();
    assert!(
        recorded.contains(&AttemptOutcome::Definitive),
        "the 2xx-then-invalid attempt must record Definitive, got {recorded:?}"
    );
    assert!(
        !recorded.contains(&AttemptOutcome::Success),
        "a validation failure must not record Success, got {recorded:?}"
    );
    server.join().unwrap();
}
