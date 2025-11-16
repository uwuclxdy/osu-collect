use reqwest::Client;
use semver::Version;
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};
use thiserror::Error;
use tokio::{fs, io::AsyncWriteExt};
use tracing::{debug, info};

const RELEASES_URL: &str = "https://api.github.com/repos/uwuclxdy/osu-collect/releases/latest";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn check_and_apply() -> Result<Option<String>, AutoUpdateError> {
    let Some(target_asset) = target_asset_name() else {
        debug!("auto-update skipped: unsupported platform");
        return Ok(None);
    };

    let client = Client::builder()
        .user_agent(format!("osu-collect/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()?;

    let release: ReleaseResponse = client
        .get(RELEASES_URL)
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

    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == target_asset)
        .ok_or_else(|| AutoUpdateError::AssetMissing(target_asset.to_string()))?;

    info!(release = %release.name, "Downloading newer release");
    let tmp_path = download_asset(&client, asset, DOWNLOAD_TIMEOUT).await?;
    apply_update(&tmp_path).await?;

    let message = format!("Application updated to {}, please restart", release.name);
    Ok(Some(message))
}

async fn download_asset(
    client: &Client,
    asset: &ReleaseAsset,
    timeout: Duration,
) -> Result<PathBuf, AutoUpdateError> {
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

    let mut file = fs::File::create(&temp_path).await?;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    set_executable_permissions(&temp_path).await?;

    Ok(temp_path)
}

async fn apply_update(temp_path: &PathBuf) -> Result<(), AutoUpdateError> {
    let exe_path = std::env::current_exe()?;
    match fs::rename(temp_path, exe_path).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(temp_path).await;
            Err(AutoUpdateError::Io(error))
        }
    }
}

async fn set_executable_permissions(path: &std::path::Path) -> Result<(), AutoUpdateError> {
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

fn target_asset_name() -> Option<&'static str> {
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
    #[error("failed during IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("unable to determine release version from tag: {0}")]
    UnparseableVersion(String),
}
