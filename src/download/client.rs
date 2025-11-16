use crate::{
    mirrors::{MirrorEndpoint, MirrorKind},
    utils::{AppError, FileExistsAction, Result, determine_file_exists_action, sanitize_filename},
    worker::{
        DownloadContext, MirrorPool,
        io::{MAX_FILE_SIZE, download_with_streaming, ensure_valid_archive, verify_existing_file},
    },
};
use std::{
    io::ErrorKind,
    sync::{Arc, atomic::Ordering},
};
use tokio::fs;

const DOWNLOAD_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadResult {
    Success(CompletedDownload),
    Skipped(Box<str>),
    Failed(&'static str),
    FailedDynamic(Box<str>),
    Aborted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedDownload {
    pub filename: Box<str>,
    pub hash: Box<str>,
    pub mirror: MirrorKind,
}

#[inline]
pub fn create_download_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(AppError::from)
}

pub async fn download_beatmap(
    beatmapset_id: u32,
    mirrors: &[MirrorEndpoint],
    context: &DownloadContext,
    progress_callback: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
    status_callback: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    rate_limiter: Option<MirrorPool>,
) -> Result<DownloadResult> {
    if context.shutdown.load(Ordering::Acquire) {
        return Ok(DownloadResult::Aborted);
    }

    let mut last_error: Option<DownloadResult> = None;

    for (idx, mirror) in mirrors.iter().enumerate() {
        if context.shutdown.load(Ordering::Acquire) {
            return Ok(DownloadResult::Aborted);
        }

        if let Some(ref callback) = status_callback {
            callback(&format!("Contacting {}...", mirror.display_name()));
        }

        let url = mirror.url_for(beatmapset_id);
        let response = match context.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                let err = if e.is_timeout() {
                    DownloadResult::Failed("Connection timeout")
                } else if e.is_connect() {
                    DownloadResult::Failed("Connection failed")
                } else {
                    DownloadResult::FailedDynamic(
                        format!("Request failed on {}: {e}", mirror.display_name())
                            .into_boxed_str(),
                    )
                };
                last_error = Some(err);
                continue;
            }
        };

        let status = response.status();

        let catboy_rate_limited = matches!(mirror.kind, MirrorKind::Catboy(_))
            && status == reqwest::StatusCode::FORBIDDEN;

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || catboy_rate_limited {
            if let Some(ref limiter) = rate_limiter {
                limiter.mark_rate_limited(mirror.kind);
            }
            if let Some(ref callback) = status_callback {
                let mut message = format!("Rate limited on {}", mirror.display_name());
                if let Some(next) = mirrors.get(idx + 1) {
                    message.push_str(&format!(", switching to {}", next.display_name()));
                }
                callback(&message);
            }
            last_error = Some(DownloadResult::Failed("Rate limited"));
            continue;
        }

        if status == reqwest::StatusCode::NOT_FOUND {
            last_error = Some(DownloadResult::Failed("Not found (404)"));
            continue;
        }

        if !status.is_success() {
            last_error = Some(DownloadResult::FailedDynamic(
                format!("HTTP {}", status).into_boxed_str(),
            ));
            continue;
        }

        let result = match process_mirror_response(
            mirror,
            response,
            beatmapset_id,
            context,
            progress_callback.clone(),
        )
        .await
        {
            Ok(res) => res,
            Err(err) => {
                last_error = Some(DownloadResult::FailedDynamic(
                    format!("{} via {}", err, mirror.display_name()).into_boxed_str(),
                ));
                continue;
            }
        };

        match result {
            DownloadResult::Success(_) | DownloadResult::Skipped(_) => return Ok(result),
            DownloadResult::Aborted => return Ok(DownloadResult::Aborted),
            DownloadResult::Failed(_) | DownloadResult::FailedDynamic(_) => {
                last_error = Some(result);
            }
        }
    }

    Ok(last_error.unwrap_or_else(|| DownloadResult::FailedDynamic("All mirrors failed".into())))
}

async fn process_mirror_response(
    mirror: &MirrorEndpoint,
    response: reqwest::Response,
    beatmapset_id: u32,
    context: &DownloadContext,
    progress_callback: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
) -> Result<DownloadResult> {
    let content_length = response.content_length();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase());

    if let Some(ref ct) = content_type {
        if !is_archive_content_type(ct) {
            return Ok(DownloadResult::FailedDynamic(
                format!(
                    "Unexpected content type '{}' from {}",
                    ct,
                    mirror.display_name()
                )
                .into_boxed_str(),
            ));
        }
    }
    if let Some(len) = content_length {
        if len > MAX_FILE_SIZE as u64 {
            return Ok(DownloadResult::FailedDynamic(
                format!("File too large ({} MB, max 100 MB)", len / 1024 / 1024).into_boxed_str(),
            ));
        }
    }

    let filename = extract_filename_from_response(&response, beatmapset_id)?;
    let sanitized_filename = sanitize_filename(&filename);
    let output_path = context.output_dir.join(&sanitized_filename);

    let existing_metadata = match fs::metadata(&output_path).await {
        Ok(meta) => Some(meta),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(AppError::from(err)),
    };

    if existing_metadata.is_some() {
        if context.shutdown.load(Ordering::Acquire) {
            return Ok(DownloadResult::Aborted);
        }

        if context.skip_existing {
            if let Some(registry) = &context.verified_registry {
                if registry.contains(beatmapset_id) {
                    return Ok(DownloadResult::Skipped(
                        sanitized_filename.clone().into_boxed_str(),
                    ));
                }
            }

            if verify_existing_file(&output_path).await? {
                return Ok(DownloadResult::Skipped(
                    sanitized_filename.clone().into_boxed_str(),
                ));
            }
            // invalid archives are deleted inside verify_existing_file, so fall through
        } else {
            let action =
                determine_file_exists_action(context.skip_existing, context.auto_overwrite)?;
            if matches!(action, FileExistsAction::Skip) {
                return Ok(DownloadResult::Skipped(
                    sanitized_filename.clone().into_boxed_str(),
                ));
            }
        }
    }

    context.cleanup_tracker.track(&output_path);

    let stream = match download_with_streaming(
        response,
        &output_path,
        content_length,
        progress_callback,
        context.shutdown.clone(),
    )
    .await
    {
        Ok(stream) => stream,
        Err(err) => {
            context.cleanup_tracker.mark_removed(&output_path);
            return Err(err);
        }
    };

    if stream.aborted {
        context.cleanup_tracker.mark_removed(&output_path);
        return Ok(DownloadResult::Aborted);
    }

    if let Some(expected) = content_length {
        if stream.bytes_written < expected {
            let _ = fs::remove_file(&output_path).await;
            context.cleanup_tracker.mark_removed(&output_path);
            return Ok(DownloadResult::FailedDynamic(
                format!(
                    "Download incomplete from {} (received {} of {} bytes)",
                    mirror.display_name(),
                    stream.bytes_written,
                    expected
                )
                .into_boxed_str(),
            ));
        }
    }

    if let Err(err) = ensure_valid_archive(&output_path).await {
        let _ = fs::remove_file(&output_path).await;
        context.cleanup_tracker.mark_removed(&output_path);
        return Ok(DownloadResult::FailedDynamic(
            format!(
                "{} returned an invalid archive: {}",
                mirror.display_name(),
                err
            )
            .into_boxed_str(),
        ));
    }

    let hash = stream.hash.unwrap_or_else(|| "unknown".into());

    context.cleanup_tracker.mark_complete(&output_path);

    Ok(DownloadResult::Success(CompletedDownload {
        filename: sanitized_filename.into_boxed_str(),
        hash,
        mirror: mirror.kind,
    }))
}

fn is_archive_content_type(raw: &str) -> bool {
    let mime = raw.split(';').next().map(str::trim).unwrap_or("");

    matches!(
        mime,
        "application/x-osu-beatmap-archive"
            | "application/octet-stream"
            | "binary/octet-stream"
            | "application/zip"
            | "application/x-zip-compressed"
    )
}

fn extract_filename_from_response(
    response: &reqwest::Response,
    beatmapset_id: u32,
) -> Result<String> {
    if let Some(content_disposition) = response.headers().get(reqwest::header::CONTENT_DISPOSITION)
    {
        if let Ok(value) = content_disposition.to_str() {
            if let Some(filename) = parse_content_disposition(value) {
                return Ok(filename);
            }
        }
    }

    Ok(format!("{}.osz", beatmapset_id))
}

fn parse_content_disposition(value: &str) -> Option<String> {
    for part in value.split(';') {
        let part = part.trim();

        if let Some(filename) = part.strip_prefix("filename*=UTF-8''") {
            return Some(filename.trim_matches('\"').to_string());
        }

        if let Some(filename) = part.strip_prefix("filename=") {
            return Some(filename.trim_matches('\"').to_string());
        }
    }

    None
}
