use reqwest::Client;
use semver::Version;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use tokio::{fs, io::AsyncWriteExt};
use tracing::{debug, info, warn};

use sha2::{Digest, Sha256};

use crate::config::constants::{AUTO_UPDATE_TIMEOUT, RELEASES_URL};

fn build_client() -> Result<Client, AutoUpdateError> {
    Ok(Client::builder()
        .user_agent(format!("osu-collect/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()?)
}

pub async fn check_and_apply<F>(on_update_found: F) -> Result<Option<String>, AutoUpdateError>
where
    F: FnOnce() + Send,
{
    let client = build_client()?;

    check_release(&client, RELEASES_URL, on_update_found, |asset| async move {
        apply_update(&asset).await
    })
    .await
}

/// Metadata for a newer release surfaced to the UI in notify-only mode.
#[derive(Debug, Clone)]
pub struct AvailableUpdate {
    /// Release semver (tag with any leading `v` stripped).
    pub version: String,
    /// Release display name.
    pub name: String,
    /// Release body / changelog (GitHub-flavoured markdown; may be empty).
    pub changelog: String,
}

/// Check for a newer release WITHOUT downloading or applying it. `Ok(None)` =
/// up to date (or an unsupported platform). Used when auto-update is disabled.
pub async fn check_for_update() -> Result<Option<AvailableUpdate>, AutoUpdateError> {
    let client = build_client()?;
    check_for_update_with(&client, RELEASES_URL).await
}

#[doc(hidden)]
pub async fn check_for_update_with(
    client: &Client,
    releases_url: &str,
) -> Result<Option<AvailableUpdate>, AutoUpdateError> {
    // An unsupported platform can't self-replace, so notifying is pointless.
    if target_asset_name().is_none() {
        debug!("update check skipped: unsupported platform");
        return Ok(None);
    }

    let Some((release, latest)) = fetch_newer_release(client, releases_url).await? else {
        return Ok(None);
    };

    Ok(Some(AvailableUpdate {
        version: latest.to_string(),
        name: release.name,
        changelog: release.body.unwrap_or_default(),
    }))
}

/// Fetch the latest release and return it with its parsed version only when it
/// is newer than the running binary. `Ok(None)` = up to date.
async fn fetch_newer_release(
    client: &Client,
    releases_url: &str,
) -> Result<Option<(ReleaseResponse, Version)>, AutoUpdateError> {
    let release: ReleaseResponse = client
        .get(releases_url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let current_version = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let latest_version = parse_release_version(&release)
        .ok_or_else(|| AutoUpdateError::UnparseableVersion(release.tag_name.clone()))?;

    if latest_version <= current_version {
        debug!(?latest_version, ?current_version, "no updates available");
        return Ok(None);
    }

    Ok(Some((release, latest_version)))
}

#[doc(hidden)]
pub async fn check_release<F, A, Fut>(
    client: &Client,
    releases_url: &str,
    on_update_found: F,
    applier: A,
) -> Result<Option<String>, AutoUpdateError>
where
    F: FnOnce() + Send,
    A: FnOnce(DownloadedAsset) -> Fut,
    Fut: Future<Output = Result<(), AutoUpdateError>> + Send,
{
    let Some(target_asset) = target_asset_name() else {
        debug!("auto-update skipped: unsupported platform");
        return Ok(None);
    };

    let Some((release, _latest_version)) = fetch_newer_release(client, releases_url).await? else {
        return Ok(None);
    };

    on_update_found();

    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == target_asset)
        .ok_or_else(|| AutoUpdateError::AssetMissing(target_asset.to_string()))?;

    info!(release = %release.name, "Downloading newer release");
    let downloaded = download_asset(client, asset, AUTO_UPDATE_TIMEOUT).await?;

    let expected_checksum = match fetch_asset_checksum(client, asset, &release.assets).await {
        Ok(checksum) => checksum,
        Err(err) => {
            let _ = fs::remove_file(&downloaded.path).await;
            return Err(err);
        }
    };

    verify_checksum(&downloaded, &expected_checksum).await?;

    applier(downloaded).await?;

    let message = format!("Application updated to {}, please restart", release.name);
    Ok(Some(message))
}

pub fn spawn_background_update() {
    let handle = spawn_update_task(|| check_and_apply(print_update_banner));
    drop(handle);
}

pub fn spawn_update_task<Fut>(
    update_fn: impl FnOnce() -> Fut + Send + 'static,
) -> tokio::task::JoinHandle<()>
where
    Fut: Future<Output = Result<Option<String>, AutoUpdateError>> + Send + 'static,
{
    tokio::spawn(async move {
        match update_fn().await {
            Ok(Some(message)) => {
                info!(%message, "Auto-update applied");
            }
            Ok(None) => {}
            Err(err) => {
                warn!(error = %err, "Auto-update failed; new version may be available");
            }
        }
    })
}

fn print_update_banner() {
    println!("{}", update_banner());
}

#[doc(hidden)]
pub fn update_banner() -> &'static str {
    "\u{1b}[32mDownloading update...\u{1b}[0m"
}

#[doc(hidden)]
pub struct DownloadedAsset {
    pub path: PathBuf,
    pub checksum: String,
}

async fn download_asset(
    client: &Client,
    asset: &ReleaseAsset,
    timeout: Duration,
) -> Result<DownloadedAsset, AutoUpdateError> {
    let mut response = client
        .get(&asset.browser_download_url)
        .timeout(timeout)
        .send()
        .await?
        .error_for_status()?;

    let exe_path = std::env::current_exe()?;
    let temp_path = exe_path
        .parent()
        .ok_or(AutoUpdateError::ExecutablePath)?
        .join(".osu-collect-update.tmp");

    let mut hasher = Sha256::new();
    let mut file = fs::File::create(&temp_path).await?;
    while let Some(chunk) = response.chunk().await? {
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    set_executable_permissions(&temp_path).await?;

    let checksum: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    Ok(DownloadedAsset {
        path: temp_path,
        checksum,
    })
}

async fn fetch_asset_checksum(
    client: &Client,
    asset: &ReleaseAsset,
    assets: &[ReleaseAsset],
) -> Result<String, AutoUpdateError> {
    let checksum_asset = assets
        .iter()
        .find(|candidate| candidate.name == format!("{}.sha256", asset.name))
        .or_else(|| {
            assets.iter().find(|candidate| {
                candidate.name.ends_with(".sha256") && candidate.name.contains(&asset.name)
            })
        })
        .ok_or_else(|| AutoUpdateError::ChecksumMissing(asset.name.clone()))?;

    let body = client
        .get(&checksum_asset.browser_download_url)
        .timeout(AUTO_UPDATE_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    parse_checksum(&body, &asset.name)
}

fn parse_checksum(body: &str, asset_name: &str) -> Result<String, AutoUpdateError> {
    let checksum = body
        .split_whitespace()
        .find(|part| !part.is_empty())
        .ok_or_else(|| AutoUpdateError::ChecksumFormat(asset_name.to_string()))?
        .to_ascii_lowercase();

    if checksum.len() != 64 || !checksum.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AutoUpdateError::ChecksumFormat(asset_name.to_string()));
    }

    Ok(checksum)
}

#[doc(hidden)]
pub async fn verify_checksum(
    asset: &DownloadedAsset,
    expected: &str,
) -> Result<(), AutoUpdateError> {
    let actual = asset.checksum.to_ascii_lowercase();
    if actual == expected {
        return Ok(());
    }

    let _ = fs::remove_file(&asset.path).await;
    Err(AutoUpdateError::ChecksumMismatch {
        expected: expected.to_string(),
        actual,
    })
}

async fn apply_update(asset: &DownloadedAsset) -> Result<(), AutoUpdateError> {
    let exe_path = std::env::current_exe()?;
    apply_update_to(asset, &exe_path).await
}

#[doc(hidden)]
pub async fn apply_update_to(
    asset: &DownloadedAsset,
    exe_path: &Path,
) -> Result<(), AutoUpdateError> {
    let rollback_path = exe_path.with_extension("rollback");

    // Drop any leftover rollback from a previous failed attempt so the rename below
    // doesn't trip on an existing destination.
    let _ = fs::remove_file(&rollback_path).await;

    // Move the running binary aside instead of overwriting it. Windows refuses to
    // replace a running .exe but permits renaming it within the same directory.
    if let Err(err) = fs::rename(exe_path, &rollback_path).await {
        let _ = fs::remove_file(&asset.path).await;
        return Err(AutoUpdateError::Io(err));
    }

    match fs::rename(&asset.path, exe_path).await {
        Ok(()) => {
            // Best-effort cleanup. On Windows the old image is still mapped by the
            // running process and can only be removed after restart;
            // cleanup_stale_artifacts() handles that on the next launch.
            let _ = fs::remove_file(&rollback_path).await;
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&asset.path).await;
            if let Err(restore_err) = fs::rename(&rollback_path, exe_path).await {
                Err(AutoUpdateError::RollbackFailed(restore_err))
            } else {
                Err(AutoUpdateError::Io(error))
            }
        }
    }
}

/// Remove a leftover `.rollback` next to the current executable. Windows can't
/// delete the running image during the update, so it's cleaned on the next
/// startup instead.
pub fn cleanup_stale_artifacts() {
    let Ok(exe_path) = std::env::current_exe() else {
        return;
    };
    let rollback_path = exe_path.with_extension("rollback");
    let _ = std::fs::remove_file(&rollback_path);
}

/// Whether the running binary lives under a Cargo `target/` output dir
/// (`cargo run` / `cargo run --release`), i.e. a local dev build. Self-replacing
/// such a binary is pointless and clobbers the freshly compiled artifact, so the
/// update flow surfaces availability rather than auto-applying it.
pub fn is_cargo_build() -> bool {
    std::env::current_exe()
        .map(|path| is_cargo_target_path(&path))
        .unwrap_or(false)
}

/// A path is a Cargo build artifact when its immediate parent is a `debug` or
/// `release` profile dir nested somewhere under a `target` dir — the layout
/// `cargo run` produces. `cargo install` (→ `~/.cargo/bin`) and downloaded
/// releases don't match, so auto-update stays enabled for them.
fn is_cargo_target_path(exe_path: &Path) -> bool {
    let parent_is_profile = exe_path
        .parent()
        .and_then(|parent| parent.file_name())
        .is_some_and(|name| name == "debug" || name == "release");
    parent_is_profile && exe_path.components().any(|c| c.as_os_str() == "target")
}

async fn set_executable_permissions(path: &Path) -> Result<(), AutoUpdateError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        fs::set_permissions(path, perms).await?;
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}

fn parse_release_version(release: &ReleaseResponse) -> Option<Version> {
    parse_version(&release.tag_name).or_else(|| parse_version(&release.name))
}

fn parse_version(input: &str) -> Option<Version> {
    let trimmed = input.trim_start_matches('v');
    Version::parse(trimmed).ok()
}

#[doc(hidden)]
pub fn target_asset_name() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("osu-collect-linux-x64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("osu-collect-windows-x64.exe")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("osu-collect-macos-arm64")
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseResponse {
    pub name: String,
    pub tag_name: String,
    #[serde(default)]
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Error)]
pub enum AutoUpdateError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to parse version: {0}")]
    Version(#[from] semver::Error),
    #[error("unable to locate current executable")]
    ExecutablePath,
    #[error("missing asset for platform: {0}")]
    AssetMissing(String),
    #[error("missing checksum for asset: {0}")]
    ChecksumMissing(String),
    #[error("checksum file malformed for asset: {0}")]
    ChecksumFormat(String),
    #[error("checksum mismatch: expected {expected}, actual {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("failed during IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("unable to determine release version from tag: {0}")]
    UnparseableVersion(String),
    #[error("failed to restore original binary after update failure: {0}")]
    RollbackFailed(std::io::Error),
}

#[cfg(test)]
#[path = "../tests/unit/auto_update.rs"]
mod tests;
