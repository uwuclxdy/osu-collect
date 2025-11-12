use crate::error::{AppError, Result};
use crate::utils::sanitize_filename;
use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;

const MAX_FILE_SIZE: u32 = 100 * 1024 * 1024;
const DOWNLOAD_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadResult {
    Success(Box<str>),
    Skipped(Box<str>),
    Failed(&'static str),
    FailedDynamic(Box<str>),
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileExistsAction {
    Skip,
    Overwrite,
    Abort,
}

/// Create HTTP client optimized for downloads
#[inline]
pub fn create_download_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(AppError::Network)
}

/// Download beatmap with streaming and async I/O
pub async fn download_beatmap(
    client: &reqwest::Client,
    beatmapset_id: u32,
    mirror_url_template: &str,
    output_dir: &Path,
    skip_existing: bool,
    auto_overwrite: bool,
    shutdown: Arc<AtomicBool>,
) -> Result<DownloadResult> {
    let mirror_url = mirror_url_template.replace("{id}", &beatmapset_id.to_string());

    let response = match client.get(&mirror_url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            return Ok(if e.is_timeout() {
                DownloadResult::Failed("Connection timeout")
            } else if e.is_connect() {
                DownloadResult::Failed("Connection failed")
            } else {
                return Err(AppError::from(e));
            });
        }
    };

    let status = response.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(DownloadResult::Failed("Not found (404)"));
    }

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Ok(DownloadResult::Failed("Rate limited (429)"));
    }

    if !status.is_success() {
        return Ok(DownloadResult::FailedDynamic(
            format!("HTTP {}", status).into_boxed_str()
        ));
    }

    let content_length = response.content_length();
    if let Some(len) = content_length {
        if len > MAX_FILE_SIZE as u64 {
            return Ok(DownloadResult::FailedDynamic(
                format!("File too large ({} MB, max 100 MB)", len / 1024 / 1024).into_boxed_str()
            ));
        }
    }

    let filename = extract_filename_from_response(&response, beatmapset_id)?;
    let sanitized_filename = sanitize_filename(&filename);
    let output_path = output_dir.join(&sanitized_filename);

    if output_path.exists() {
        // Check if shutdown was triggered by another download
        if shutdown.load(Ordering::Acquire) {
            return Ok(DownloadResult::Aborted);
        }

        let action = determine_file_exists_action(skip_existing, auto_overwrite, &sanitized_filename, shutdown.clone())?;

        match action {
            FileExistsAction::Skip => {
                return Ok(DownloadResult::Skipped(sanitized_filename.into_boxed_str()));
            }
            FileExistsAction::Abort => {
                return Ok(DownloadResult::Aborted);
            }
            FileExistsAction::Overwrite => {}
        }
    }

    download_with_streaming(response, &output_path).await
        .map(|_| DownloadResult::Success(sanitized_filename.into_boxed_str()))
}

/// Stream download to file with chunked writing
async fn download_with_streaming(
    response: reqwest::Response,
    output_path: &Path,
) -> Result<()> {
    let mut file = fs::File::create(output_path).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(AppError::Network)?;

        downloaded += chunk.len() as u64;

        if downloaded > MAX_FILE_SIZE as u64 {
            file.shutdown().await?;
            let _ = fs::remove_file(output_path).await;
            return Err(AppError::other_dynamic(
                format!("File too large ({} MB, max 100 MB)", downloaded / 1024 / 1024).into_boxed_str()
            ));
        }

        file.write_all(&chunk).await?;
    }

    file.flush().await?;
    file.shutdown().await?;

    Ok(())
}

/// Extract filename from HTTP response headers
fn extract_filename_from_response(
    response: &reqwest::Response,
    beatmapset_id: u32,
) -> Result<String> {
    if let Some(content_disposition) = response.headers().get(reqwest::header::CONTENT_DISPOSITION) {
        if let Ok(value) = content_disposition.to_str() {
            if let Some(filename) = parse_content_disposition(value) {
                return Ok(filename);
            }
        }
    }

    Ok(format!("{}.osz", beatmapset_id))
}

/// Parse Content-Disposition header
fn parse_content_disposition(value: &str) -> Option<String> {
    for part in value.split(';') {
        let part = part.trim();

        if let Some(filename) = part.strip_prefix("filename*=UTF-8''") {
            return Some(filename.trim_matches('"').to_string());
        }

        if let Some(filename) = part.strip_prefix("filename=") {
            return Some(filename.trim_matches('"').to_string());
        }
    }

    None
}

/// Determine action when file exists
fn determine_file_exists_action(
    skip_existing: bool,
    auto_overwrite: bool,
    filename: &str,
    shutdown: Arc<AtomicBool>,
) -> Result<FileExistsAction> {
    if skip_existing {
        return Ok(FileExistsAction::Skip);
    }

    if auto_overwrite {
        return Ok(FileExistsAction::Overwrite);
    }

    eprintln!("\nFile already exists: {}", filename);
    eprintln!("Options:");
    eprintln!("  [s] Skip this file");
    eprintln!("  [o] Overwrite this file");
    eprintln!("  [a] Abort (stop all downloads)");
    eprint!("Choose action (s/o/a): ");
    std::io::Write::flush(&mut std::io::stderr())?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    match input.trim().to_lowercase().as_str() {
        "s" => Ok(FileExistsAction::Skip),
        "o" => Ok(FileExistsAction::Overwrite),
        "a" => {
            shutdown.store(true, Ordering::Release);
            Ok(FileExistsAction::Abort)
        }
        _ => {
            eprintln!("Invalid choice, skipping file.");
            Ok(FileExistsAction::Skip)
        }
    }
}

/// Validate and prepare download directory
pub async fn validate_and_prepare_directory(directory: &str) -> Result<PathBuf> {
    let expanded_path = if directory.starts_with("~/") {
        if let Some(home_dir) = dirs::home_dir() {
            home_dir.join(&directory[2..])
        } else {
            PathBuf::from(directory)
        }
    } else {
        PathBuf::from(directory)
    };

    if !expanded_path.exists() {
        fs::create_dir_all(&expanded_path).await.map_err(|e| {
            AppError::FileSystem(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create directory '{}': {}", expanded_path.display(), e),
            ))
        })?;
    }

    let metadata = fs::metadata(&expanded_path).await?;
    if !metadata.is_dir() {
        return Err(AppError::FileSystem(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            format!("Path '{}' is not a directory", expanded_path.display()),
        )));
    }

    let test_file = expanded_path.join(".write_test");
    match fs::File::create(&test_file).await {
        Ok(_) => {
            let _ = fs::remove_file(&test_file).await;
            Ok(expanded_path)
        }
        Err(e) => Err(AppError::FileSystem(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("Directory '{}' is not writable: {}", expanded_path.display(), e),
        ))),
    }
}
