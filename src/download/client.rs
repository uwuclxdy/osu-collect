use crate::{
    check_shutdown,
    download::{http_client, status},
    mirrors::{MirrorEndpoint, MirrorKind},
    utils::{AppError, FileExistsAction, Result, determine_file_exists_action, sanitize_filename},
    worker::{
        DownloadContext, MirrorPool,
        io::{
            ArchiveValidationOptions, ArchiveValidationResult, download_with_streaming,
            ensure_valid_archive, validate_archive,
        },
    },
};
use std::path::Path;
use std::{borrow::Cow, io::ErrorKind, path::PathBuf, sync::Arc, time::Duration};
use tokio::{fs, time::sleep};

type StatusCallback = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone, Default)]
pub struct StatusReporter {
    callback: Option<StatusCallback>,
}

impl StatusReporter {
    pub fn new(callback: Option<StatusCallback>) -> Self {
        Self { callback }
    }

    pub fn emit_with<F>(&self, build: F)
    where
        F: FnOnce() -> String,
    {
        if let Some(callback) = &self.callback {
            let message = build();
            callback(&message);
        }
    }
}

impl From<Option<StatusCallback>> for StatusReporter {
    fn from(callback: Option<StatusCallback>) -> Self {
        StatusReporter::new(callback)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadResult {
    Success(CompletedDownload),
    Skipped(Box<str>),
    Failed(DownloadFailure),
    Aborted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadFailure {
    pub mirror: Option<MirrorKind>,
    pub reason: Cow<'static, str>,
}

impl DownloadResult {
    fn failed(mirror: Option<MirrorKind>, reason: impl Into<Cow<'static, str>>) -> Self {
        DownloadResult::Failed(DownloadFailure {
            mirror,
            reason: reason.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedDownload {
    pub filename: Box<str>,
    pub hash: Box<str>,
    pub mirror: MirrorKind,
}

#[inline]
pub fn create_download_client() -> Result<reqwest::Client> {
    http_client::download_client()
}

pub async fn download_beatmap(
    beatmapset_id: u32,
    mirrors: &[MirrorEndpoint],
    context: &DownloadContext,
    progress_callback: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
    status_reporter: StatusReporter,
    rate_limiter: Option<MirrorPool>,
) -> Result<DownloadResult> {
    check_shutdown!(context.shutdown);

    let mut last_error: Option<DownloadResult> = None;
    let mut pending: Vec<MirrorEndpoint> = mirrors.to_vec();

    while !pending.is_empty() {
        let mut deferred_rate_limited: Vec<MirrorEndpoint> = Vec::new();

        for mirror in pending.iter() {
            check_shutdown!(context.shutdown);

            status_reporter.emit_with(|| {
                format!(
                    "{} #{} from {}",
                    status::FETCHING,
                    beatmapset_id,
                    mirror.display_name()
                )
            });

            let url = mirror.url_for(beatmapset_id);
            let response = match context.client.get(&url).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    let err = if e.is_timeout() {
                        DownloadResult::failed(Some(mirror.kind), "Connection timeout")
                    } else if e.is_connect() {
                        DownloadResult::failed(Some(mirror.kind), "Connection failed")
                    } else {
                        DownloadResult::failed(
                            Some(mirror.kind),
                            format!("Request failed on {}: {e}", mirror.display_name()),
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
                deferred_rate_limited.push(mirror.clone());
                last_error = Some(DownloadResult::failed(
                    Some(mirror.kind),
                    status::RATE_LIMITED,
                ));
                continue;
            }

            if status == reqwest::StatusCode::NOT_FOUND {
                last_error = Some(DownloadResult::failed(Some(mirror.kind), "Not found (404)"));
                continue;
            }

            if !status.is_success() {
                last_error = Some(DownloadResult::failed(
                    Some(mirror.kind),
                    format!("HTTP {}", status),
                ));
                continue;
            }

            status_reporter.emit_with(|| {
                format!(
                    "{} #{} from {}",
                    status::DOWNLOADING,
                    beatmapset_id,
                    mirror.display_name()
                )
            });

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
                    last_error = Some(DownloadResult::failed(
                        Some(mirror.kind),
                        format!("{} via {}", err, mirror.display_name()),
                    ));
                    continue;
                }
            };

            match result {
                DownloadResult::Success(_) | DownloadResult::Skipped(_) => return Ok(result),
                DownloadResult::Aborted => return Ok(DownloadResult::Aborted),
                DownloadResult::Failed(_) => {
                    last_error = Some(result);
                }
            }
        }

        if deferred_rate_limited.is_empty() {
            break;
        }

        let Some(ref limiter) = rate_limiter else {
            break;
        };

        let wait_duration = deferred_rate_limited
            .iter()
            .filter_map(|mirror| limiter.penalty_remaining(mirror.kind))
            .min()
            .unwrap_or(Duration::from_secs(0));

        if !wait_duration.is_zero() {
            sleep(wait_duration).await;
        }

        pending = deferred_rate_limited;
    }

    Ok(last_error.unwrap_or_else(|| DownloadResult::failed(None, "All mirrors failed")))
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

    if let Some(ref ct) = content_type
        && !is_archive_content_type(ct)
    {
        return Ok(DownloadResult::failed(
            Some(mirror.kind),
            format!(
                "Unexpected content type '{}' from {}",
                ct,
                mirror.display_name()
            ),
        ));
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
        check_shutdown!(context.shutdown);

        if context.skip_existing {
            if context.tracker.is_verified(beatmapset_id) {
                return Ok(DownloadResult::Skipped(
                    sanitized_filename.clone().into_boxed_str(),
                ));
            }

            let validation_opts = ArchiveValidationOptions {
                verify_zip_eocd: context.verify_zip_eocd,
                remove_on_invalid: true,
            };
            match validate_archive(&output_path, validation_opts).await? {
                ArchiveValidationResult::Valid => {
                    return Ok(DownloadResult::Skipped(
                        sanitized_filename.clone().into_boxed_str(),
                    ));
                }
                ArchiveValidationResult::NotFound
                | ArchiveValidationResult::Invalid(_)
                | ArchiveValidationResult::Removed(_) => {
                    // Fall through to download
                }
            }
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

    write_and_verify_archive(
        mirror,
        response,
        context,
        output_path,
        sanitized_filename,
        content_length,
        progress_callback,
    )
    .await
}

async fn write_and_verify_archive(
    mirror: &MirrorEndpoint,
    response: reqwest::Response,
    context: &DownloadContext,
    output_path: PathBuf,
    sanitized_filename: String,
    content_length: Option<u64>,
    progress_callback: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
) -> Result<DownloadResult> {
    context.cleanup_tracker.track(&output_path);

    let stream = match download_with_streaming(
        response,
        &output_path,
        content_length,
        progress_callback,
        context.progress_watchdog,
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

    // Check for incomplete download - but still validate the file in case server sent incorrect Content-Length
    // If the file is valid despite being "incomplete", we accept it
    if let Some(expected) = content_length
        && stream.bytes_written < expected
    {
        // Try to validate anyway - server might have sent wrong Content-Length
        if ensure_valid_archive(&output_path, context.verify_zip_eocd)
            .await
            .is_err()
        {
            let _ = fs::remove_file(&output_path).await;
            context.cleanup_tracker.mark_removed(&output_path);
            return Ok(DownloadResult::failed(
                Some(mirror.kind),
                format!(
                    "Download incomplete from {} (received {} of {} bytes)",
                    mirror.display_name(),
                    stream.bytes_written,
                    expected
                ),
            ));
        }
    } else if let Err(err) = ensure_valid_archive(&output_path, context.verify_zip_eocd).await {
        let _ = fs::remove_file(&output_path).await;
        context.cleanup_tracker.mark_removed(&output_path);
        return Ok(DownloadResult::failed(
            Some(mirror.kind),
            format!(
                "{} returned an invalid archive: {}",
                mirror.display_name(),
                err
            ),
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
    let filename = if let Some(content_disposition) =
        response.headers().get(reqwest::header::CONTENT_DISPOSITION)
        && let Ok(value) = content_disposition.to_str()
        && let Some(name) = parse_content_disposition(value)
    {
        name
    } else {
        format!("{}.osz", beatmapset_id)
    };

    let has_osz_extension = Path::new(&filename)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("osz"));

    if has_osz_extension {
        Ok(filename)
    } else {
        Ok(format!("{}.osz", filename))
    }
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
