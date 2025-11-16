use crate::utils::{AppError, Result};
use futures_util::StreamExt;
use md5::{Digest, Md5};
use std::{
    fs::File,
    io::ErrorKind,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{fs, io::AsyncWriteExt, task};
use zip::ZipArchive;

pub const MAX_FILE_SIZE: u32 = 100 * 1024 * 1024;

pub struct DownloadStreamResult {
    pub aborted: bool,
    pub hash: Option<Box<str>>,
    pub bytes_written: u64,
}

pub async fn download_with_streaming(
    response: reqwest::Response,
    output_path: &Path,
    content_length: Option<u64>,
    progress_callback: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
    shutdown: Arc<AtomicBool>,
) -> Result<DownloadStreamResult> {
    let mut file = fs::File::create(output_path).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let total = content_length.unwrap_or(0);
    let mut hasher = Md5::new();

    let mut last_progress_bytes = 0u64;
    let mut last_progress_emitted = Instant::now();
    const MIN_PROGRESS_DELTA: u64 = 256 * 1024;
    const MIN_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

    while let Some(chunk) = stream.next().await {
        if shutdown.load(Ordering::Acquire) {
            file.shutdown().await?;
            let _ = fs::remove_file(output_path).await;
            return Ok(DownloadStreamResult {
                aborted: true,
                hash: None,
                bytes_written: downloaded,
            });
        }

        let chunk = match chunk {
            Ok(bytes) => bytes,
            Err(err) => {
                file.shutdown().await.ok();
                let _ = fs::remove_file(output_path).await;
                return Err(AppError::from(err));
            }
        };
        downloaded += chunk.len() as u64;
        hasher.update(&chunk);

        if downloaded > MAX_FILE_SIZE as u64 {
            file.shutdown().await?;
            let _ = fs::remove_file(output_path).await;
            return Err(AppError::other_dynamic(
                format!(
                    "File too large ({} MB, max 100 MB)",
                    downloaded / 1024 / 1024
                )
                .into_boxed_str(),
            ));
        }

        if let Err(err) = file.write_all(&chunk).await {
            file.shutdown().await.ok();
            let _ = fs::remove_file(output_path).await;
            return Err(AppError::from(err));
        }

        if let Some(ref callback) = progress_callback {
            let delta = downloaded.saturating_sub(last_progress_bytes);
            if delta >= MIN_PROGRESS_DELTA
                || last_progress_emitted.elapsed() >= MIN_PROGRESS_INTERVAL
            {
                callback(downloaded, total);
                last_progress_bytes = downloaded;
                last_progress_emitted = Instant::now();
            }
        }
    }

    if shutdown.load(Ordering::Acquire) {
        file.shutdown().await.ok();
        let _ = fs::remove_file(output_path).await;
        return Ok(DownloadStreamResult {
            aborted: true,
            hash: None,
            bytes_written: downloaded,
        });
    }

    if let Err(err) = file.flush().await {
        file.shutdown().await.ok();
        let _ = fs::remove_file(output_path).await;
        return Err(AppError::from(err));
    }

    if let Err(err) = file.shutdown().await {
        let _ = fs::remove_file(output_path).await;
        return Err(AppError::from(err));
    }

    if let Some(ref callback) = progress_callback
        && downloaded != last_progress_bytes {
            callback(downloaded, total);
        }

    let digest = format!("{:032x}", hasher.finalize());
    Ok(DownloadStreamResult {
        aborted: false,
        hash: Some(digest.into_boxed_str()),
        bytes_written: downloaded,
    })
}

pub async fn ensure_valid_archive(path: &Path) -> Result<()> {
    let path_buf = path.to_path_buf();
    task::spawn_blocking(move || -> Result<()> {
        let file = File::open(&path_buf)?;
        let archive = ZipArchive::new(file)
            .map_err(|err| AppError::other_dynamic(format!("Invalid archive: {err}")))?;
        if archive.is_empty() {
            return Err(AppError::other("Archive did not contain any beatmap files"));
        }
        Ok(())
    })
    .await
    .map_err(|err| AppError::other_dynamic(format!("Archive validation failed: {err}")))?
}

pub async fn verify_existing_file(path: &Path) -> Result<bool> {
    let metadata = match fs::metadata(path).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(AppError::from(err)),
    };

    if !metadata.is_file() {
        remove_damaged_file(path).await?;
        return Ok(false);
    }

    let file_size = metadata.len();
    if file_size == 0 || file_size > MAX_FILE_SIZE as u64 {
        remove_damaged_file(path).await?;
        return Ok(false);
    }

    match ensure_valid_archive(path).await {
        Ok(_) => Ok(true),
        Err(_) => {
            remove_damaged_file(path).await?;
            Ok(false)
        }
    }
}

async fn remove_damaged_file(path: &Path) -> Result<()> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AppError::from(err)),
    }
}
