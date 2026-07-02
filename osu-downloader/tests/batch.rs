use super::{BatchConfig, BatchQueue, Lease, download_batch};
use crate::mirrors::pool::MirrorPool;
use crate::{ArchiveValidation, Event, Mirror, OnExists, Skip};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::{Notify, mpsc, watch};

fn test_config(concurrent_downloads: usize, inline_wait_max: Duration) -> BatchConfig {
    BatchConfig {
        concurrent_downloads,
        archive_validation: ArchiveValidation::Off,
        progress_timeout: Duration::from_secs(1),
        network_retry_attempts: 0,
        sanitize_filenames: true,
        on_exists: OnExists::Skip,
        rate_limit_skip_after: None,
        inline_wait_max,
        #[cfg(feature = "instrument")]
        attempt_observer: None,
        #[cfg(feature = "instrument")]
        session_start: Instant::now(),
    }
}

fn rate_limited() -> Vec<u8> {
    b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n".to_vec()
}

fn ok_archive(id: u32) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename={id}.osz\r\nContent-Length: 4\r\n\r\ndata"
    )
    .into_bytes()
}

/// Answer `responses.len()` sequential requests with the given canned replies.
fn spawn_responder(responses: Vec<Vec<u8>>) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for resp in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&resp);
        }
    });
    (addr, handle)
}

/// Path-routed sequential responder for flows whose request order is fixed.
fn spawn_path_router(
    connections: usize,
    respond: impl Fn(&str, &mut TcpStream) + Send + 'static,
) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for _ in 0..connections {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
            respond(&path, &mut stream);
        }
    });
    (addr, handle)
}

fn drain_deferred_passes(event_rx: &mut mpsc::UnboundedReceiver<Event>) -> Vec<u32> {
    let mut passes = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        if let Event::BeatmapsetDeferred { pass, .. } = event {
            passes.push(pass);
        }
    }
    passes
}

#[tokio::test]
async fn cancel_mid_batch_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let client = reqwest::Client::new();
    let mirror_pool = Arc::new(MirrorPool::new(vec![Mirror::nerinyan()]));

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = cancel_tx.send(true);
    });

    let ids: Vec<u32> = (1u32..=5).collect();
    let summary = download_batch(
        ids,
        dir.path(),
        client,
        mirror_pool,
        test_config(2, Duration::from_secs(2)),
        event_tx,
        cancel_rx,
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
    )
    .await;

    assert!(summary.downloaded.len() + summary.skipped.len() + summary.failed.len() <= 5);
}

#[tokio::test]
async fn deferred_map_is_requeued_and_succeeds_on_a_later_pass() {
    // inline_wait_max = 0 forces any cooldown to defer. The mirror 429s once,
    // deferring the map; the requeued pass finds the cooldown expired and 200s.
    let (addr, server) = spawn_responder(vec![rate_limited(), ok_archive(42)]);
    let mirror = Mirror::custom(format!("http://{addr}/d/{{id}}")).unwrap();
    let pool = Arc::new(MirrorPool::new(vec![mirror]));
    let dir = tempfile::tempdir().unwrap();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let summary = download_batch(
        vec![42],
        dir.path(),
        reqwest::Client::new(),
        pool,
        test_config(2, Duration::ZERO),
        event_tx,
        cancel_rx,
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
    )
    .await;

    assert_eq!(summary.downloaded, vec![42]);
    assert_eq!(drain_deferred_passes(&mut event_rx), vec![1]);
    server.join().unwrap();
}

#[tokio::test]
async fn map_is_dropped_after_the_deferral_pass_cap() {
    // The mirror 429s forever, so the map defers every pass. It terminates as
    // RateLimitSkipped once the pass cap is hit rather than looping indefinitely.
    let (addr, server) = spawn_responder(vec![rate_limited(), rate_limited(), rate_limited()]);
    let mirror = Mirror::custom(format!("http://{addr}/d/{{id}}")).unwrap();
    let pool = Arc::new(MirrorPool::new(vec![mirror]));
    let dir = tempfile::tempdir().unwrap();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let start = Instant::now();
    let summary = download_batch(
        vec![7],
        dir.path(),
        reqwest::Client::new(),
        pool,
        test_config(1, Duration::ZERO),
        event_tx,
        cancel_rx,
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
    )
    .await;

    assert_eq!(summary.skipped, vec![(7, Skip::RateLimitSkipped)]);
    assert!(summary.downloaded.is_empty());
    // Two deferrals (passes 1 and 2), then dropped on the third.
    assert_eq!(drain_deferred_passes(&mut event_rx), vec![1, 2]);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "pass cap must terminate promptly"
    );
    server.join().unwrap();
}

#[test]
fn drain_deferred_drops_requeued_items_and_keeps_fresh_ones() {
    let queue = BatchQueue::new(&[10, 20]);
    // Captured after `new` so the fresh items' `ready_at` is already due.
    let now = Instant::now();
    let leased = match queue.lease(now) {
        Lease::Work(item) => item,
        _ => panic!("expected work"),
    };
    assert_eq!(leased.id, 10);
    queue.requeue(leased.id, leased.pass + 1, now + Duration::from_secs(60));

    assert_eq!(queue.drain_deferred(), vec![10]);
    // The fresh pass-0 item must survive the drain and stay leasable.
    let survivor = match queue.lease(now) {
        Lease::Work(item) => item,
        _ => panic!("expected the fresh item to survive"),
    };
    assert_eq!(survivor.id, 20);
    assert_eq!(survivor.pass, 0);
    queue.complete();
    assert!(matches!(queue.lease(now), Lease::Done));
}

#[tokio::test]
async fn drop_signal_drains_deferred_items_while_the_worker_streams() {
    // GH issue #2 shape: two maps deferred on a 60 s cooldown sit in the queue
    // while the only worker streams a healthy map. The hard drop must skip the
    // deferred maps at the instant of the press (before the stream finishes)
    // and must not touch the fresh pass-0 map still queued behind them.
    let (addr, server) = spawn_path_router(5, |path, stream| match path {
        "/bad/1" => stream
            .write_all(
                b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\nContent-Length: 0\r\n\r\n",
            )
            .unwrap(),
        "/ok/1" | "/ok/4" => stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
            .unwrap(),
        "/ok/2" => {
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=2.osz\r\nContent-Length: 8\r\n\r\ndat").unwrap();
            // Hold the body open so the worker is mid-stream when the drop fires.
            thread::sleep(Duration::from_millis(400));
            stream.write_all(b"aaaaa").unwrap();
        }
        "/ok/3" => stream.write_all(&ok_archive(3)).unwrap(),
        other => panic!("unexpected request path {other}"),
    });

    let bad = Mirror::custom(format!("http://{addr}/bad/{{id}}")).unwrap();
    let ok = Mirror::custom(format!("http://{addr}/ok/{{id}}")).unwrap();
    let pool = Arc::new(MirrorPool::new(vec![bad, ok]));
    let dir = tempfile::tempdir().unwrap();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let drop_signal = Arc::new(Notify::new());
    let drop_trigger = drop_signal.clone();

    let start = Instant::now();
    let batch = tokio::spawn({
        let dir = dir.path().to_path_buf();
        async move {
            download_batch(
                vec![1, 4, 2, 3],
                &dir,
                reqwest::Client::new(),
                pool,
                test_config(1, Duration::from_millis(100)),
                event_tx,
                cancel_rx,
                Arc::new(Notify::new()),
                drop_signal,
            )
            .await
        }
    });

    let mut events = Vec::new();
    let mut deferred_seen = 0;
    while let Some(event) = event_rx.recv().await {
        if matches!(event, Event::BeatmapsetDeferred { .. }) {
            deferred_seen += 1;
            // Both rate-limited maps are now queue-deferred; press the drop.
            if deferred_seen == 2 {
                drop_trigger.notify_waiters();
            }
        }
        let done = matches!(event, Event::SessionCompleted { .. });
        events.push(event);
        if done {
            break;
        }
    }
    let summary = batch.await.unwrap();

    let skipped: Vec<u32> = events
        .iter()
        .filter_map(|event| match event {
            Event::BeatmapsetSkipped {
                beatmapset_id,
                reason: Skip::RateLimitSkipped,
            } => Some(*beatmapset_id),
            _ => None,
        })
        .collect();
    assert_eq!(skipped, vec![1, 4], "each dropped map skips exactly once");

    // The drops must land while the worker is still streaming map 2, i.e.
    // strictly before its completion event.
    let last_skip = events
        .iter()
        .rposition(|event| matches!(event, Event::BeatmapsetSkipped { .. }))
        .expect("skip events recorded");
    let completed_2 = events
        .iter()
        .position(|event| {
            matches!(
                event,
                Event::BeatmapsetCompleted {
                    beatmapset_id: 2,
                    ..
                }
            )
        })
        .expect("map 2 completes");
    assert!(
        last_skip < completed_2,
        "deferred maps must drop instantly, not after the busy worker frees"
    );

    // The fresh pass-0 map (3) survives the drop and still downloads.
    assert_eq!(summary.downloaded, vec![2, 3]);
    assert_eq!(
        summary.skipped,
        vec![(1, Skip::RateLimitSkipped), (4, Skip::RateLimitSkipped)]
    );
    assert!(start.elapsed() < Duration::from_secs(10));
    server.join().unwrap();
}
