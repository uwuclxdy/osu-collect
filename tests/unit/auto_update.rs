use super::{
    AutoUpdateError, AvailableUpdate, DownloadedAsset, apply_update_to, check_for_update_with,
    check_release, is_cargo_target_path, target_asset_name, verify_checksum,
};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn unique_temp_dir(suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "osu_collect_update_test_{}_{}_{}",
        std::process::id(),
        suffix,
        nanos
    ))
}

#[tokio::test]
async fn verify_checksum_removes_temp_on_mismatch() {
    let dir = unique_temp_dir("checksum");
    fs::create_dir_all(&dir).await.unwrap();
    let temp_path = dir.join("download.tmp");
    fs::write(&temp_path, b"data").await.unwrap();

    let asset = DownloadedAsset {
        path: temp_path.clone(),
        checksum: "aaaa".into(),
    };

    let result = verify_checksum(&asset, "bbbb").await;
    assert!(matches!(
        result,
        Err(AutoUpdateError::ChecksumMismatch { .. })
    ));
    assert!(!temp_path.exists());

    let _ = fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn apply_update_to_replaces_binary_and_cleans_rollback() {
    let dir = unique_temp_dir("apply");
    fs::create_dir_all(&dir).await.unwrap();

    let exe_path = dir.join("osu-collect-test");
    fs::write(&exe_path, b"old-binary").await.unwrap();

    let download_path = dir.join("downloaded");
    fs::write(&download_path, b"new-binary").await.unwrap();

    let asset = DownloadedAsset {
        path: download_path.clone(),
        checksum: "ignored".into(),
    };

    apply_update_to(&asset, &exe_path).await.unwrap();

    let updated = fs::read(&exe_path).await.unwrap();
    assert_eq!(updated, b"new-binary");
    assert!(!download_path.exists());
    assert!(!exe_path.with_extension("rollback").exists());

    let _ = fs::remove_dir_all(&dir).await;
}

async fn start_mock_release_server(
    release_body: String,
    asset_body: Vec<u8>,
    checksum_body: String,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{}", addr);
    let release_body = release_body.replace("http://placeholder", &base);

    let handle = tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };

            let mut buf = [0u8; 4096];
            let read = match socket.read(&mut buf).await {
                Ok(size) => size,
                Err(_) => break,
            };
            if read == 0 {
                continue;
            }

            let request = String::from_utf8_lossy(&buf[..read]);
            let response_bytes = if request.starts_with("GET /release") {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    release_body.len(),
                    release_body
                )
                .into_bytes()
            } else if request.starts_with("GET /asset.sha256") {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                    checksum_body.len(),
                    checksum_body
                )
                .into_bytes()
            } else if request.starts_with("GET /asset") {
                let mut resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                    asset_body.len()
                )
                .into_bytes();
                resp.extend_from_slice(&asset_body);
                resp
            } else {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
            };

            let _ = socket.write_all(&response_bytes).await;
        }
    });

    (base, handle)
}

fn target_asset_or_skip() -> Option<&'static str> {
    target_asset_name()
}

#[tokio::test]
async fn check_and_apply_with_client_runs_hooks_on_update() {
    let Some(asset_name) = target_asset_or_skip() else {
        return;
    };

    let asset_bytes = b"new-binary".to_vec();
    let checksum: String = Sha256::digest(&asset_bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let release_body = serde_json::json!({
        "name": "v9.9.9",
        "tag_name": "v9.9.9",
        "assets": [
            {"name": asset_name, "browser_download_url": "http://placeholder/asset"},
            {"name": format!("{asset_name}.sha256"), "browser_download_url": "http://placeholder/asset.sha256"}
        ]
    })
    .to_string();

    let (base, handle) =
        start_mock_release_server(release_body, asset_bytes.clone(), checksum.clone()).await;

    let release_url = format!("{}/release", base);
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let callback_ran = Arc::new(AtomicBool::new(false));
    let applier_ran = Arc::new(AtomicBool::new(false));
    let callback_flag = callback_ran.clone();
    let applier_flag = applier_ran.clone();

    let result = check_release(
        &client,
        &release_url,
        move || callback_flag.store(true, Ordering::SeqCst),
        move |asset| {
            let applier_flag = applier_flag.clone();
            async move {
                applier_flag.store(true, Ordering::SeqCst);
                let _ = fs::remove_file(&asset.path).await;
                Ok(())
            }
        },
    )
    .await
    .unwrap();

    assert!(result.is_some());
    assert!(callback_ran.load(Ordering::SeqCst));
    assert!(applier_ran.load(Ordering::SeqCst));

    handle.abort();
}

#[tokio::test]
async fn check_and_apply_with_client_skips_when_current() {
    let Some(asset_name) = target_asset_or_skip() else {
        return;
    };

    let release_body = serde_json::json!({
        "name": env!("CARGO_PKG_VERSION"),
        "tag_name": env!("CARGO_PKG_VERSION"),
        "assets": [
            {"name": asset_name, "browser_download_url": "http://placeholder/asset"}
        ]
    })
    .to_string();

    let (base, handle) =
        start_mock_release_server(release_body, b"noop".to_vec(), "deadbeef".to_string()).await;

    let release_url = format!("{}/release", base);
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let callback_ran = Arc::new(AtomicBool::new(false));
    let applier_ran = Arc::new(AtomicBool::new(false));
    let callback_flag = callback_ran.clone();
    let applier_flag = applier_ran.clone();

    let result = check_release(
        &client,
        &release_url,
        move || callback_flag.store(true, Ordering::SeqCst),
        move |_asset| {
            let applier_flag = applier_flag.clone();
            async move {
                applier_flag.store(true, Ordering::SeqCst);
                Ok(())
            }
        },
    )
    .await
    .unwrap();

    assert!(result.is_none());
    assert!(!callback_ran.load(Ordering::SeqCst));
    assert!(!applier_ran.load(Ordering::SeqCst));

    handle.abort();
}

#[tokio::test]
async fn check_for_update_returns_metadata_when_newer() {
    let Some(asset_name) = target_asset_or_skip() else {
        return;
    };

    let release_body = serde_json::json!({
        "name": "v9.9.9",
        "tag_name": "v9.9.9",
        "body": "## changes\n- did a thing\n- fixed a bug",
        "assets": [
            {"name": asset_name, "browser_download_url": "http://placeholder/asset"}
        ]
    })
    .to_string();

    let (base, handle) =
        start_mock_release_server(release_body, b"noop".to_vec(), "deadbeef".to_string()).await;
    let release_url = format!("{}/release", base);
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let info: Option<AvailableUpdate> = check_for_update_with(&client, &release_url).await.unwrap();
    let info = info.expect("newer release should be reported");
    assert_eq!(info.version, "9.9.9");
    assert_eq!(info.name, "v9.9.9");
    assert!(info.changelog.contains("did a thing"));

    handle.abort();
}

#[tokio::test]
async fn check_for_update_none_when_current() {
    let Some(asset_name) = target_asset_or_skip() else {
        return;
    };

    let release_body = serde_json::json!({
        "name": env!("CARGO_PKG_VERSION"),
        "tag_name": env!("CARGO_PKG_VERSION"),
        "body": "nothing new",
        "assets": [
            {"name": asset_name, "browser_download_url": "http://placeholder/asset"}
        ]
    })
    .to_string();

    let (base, handle) =
        start_mock_release_server(release_body, b"noop".to_vec(), "deadbeef".to_string()).await;
    let release_url = format!("{}/release", base);
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let info = check_for_update_with(&client, &release_url).await.unwrap();
    assert!(info.is_none());

    handle.abort();
}

#[test]
fn cargo_target_paths_are_dev_builds() {
    assert!(is_cargo_target_path(std::path::Path::new(
        "/home/dev/proj/target/debug/osu-collect"
    )));
    assert!(is_cargo_target_path(std::path::Path::new(
        "/home/dev/proj/target/release/osu-collect"
    )));
    // Shared/custom target dir (workspace target one level up) still matches.
    assert!(is_cargo_target_path(std::path::Path::new(
        "/home/dev/repos/rs/target/debug/osu-collect"
    )));
}

#[test]
fn installed_paths_are_not_dev_builds() {
    // cargo install lands in ~/.cargo/bin — no profile parent.
    assert!(!is_cargo_target_path(std::path::Path::new(
        "/home/dev/.cargo/bin/osu-collect"
    )));
    // A downloaded release on PATH.
    assert!(!is_cargo_target_path(std::path::Path::new(
        "/usr/local/bin/osu-collect"
    )));
    // A `debug`/`release` profile dir NOT under `target` doesn't count.
    assert!(!is_cargo_target_path(std::path::Path::new(
        "/opt/release/osu-collect"
    )));
}
