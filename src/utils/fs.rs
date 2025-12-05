use crate::{
    config::constants::{GB, KB, LOW_SPACE_THRESHOLD_BYTES, MB},
    utils::error::{AppError, Result},
};
use fs2::available_space;
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileExistsAction {
    Skip,
    Overwrite,
}

pub fn check_available_space(path: &Path) -> Option<u64> {
    available_space(path).ok()
}

pub fn is_low_disk_space(path: &Path) -> bool {
    check_available_space(path).is_some_and(|space| space < LOW_SPACE_THRESHOLD_BYTES)
}

pub fn format_bytes(bytes: u64) -> String {
    let bytes_f = bytes as f64;
    if bytes_f >= GB {
        format!("{:.2} GB", bytes_f / GB)
    } else if bytes_f >= MB {
        format!("{:.1} MB", bytes_f / MB)
    } else if bytes_f >= KB {
        format!("{:.0} KB", bytes_f / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn determine_file_exists_action(
    skip_existing: bool,
    auto_overwrite: bool,
) -> Result<FileExistsAction> {
    if skip_existing {
        return Ok(FileExistsAction::Skip);
    }

    if auto_overwrite {
        return Ok(FileExistsAction::Overwrite);
    }

    Ok(FileExistsAction::Skip)
}

pub async fn validate_and_prepare_directory(directory: &str) -> Result<PathBuf> {
    let expanded_path = if let Some(stripped) = directory.strip_prefix("~/") {
        if let Some(home_dir) = dirs::home_dir() {
            home_dir.join(stripped)
        } else {
            PathBuf::from(directory)
        }
    } else {
        PathBuf::from(directory)
    };

    if !expanded_path.exists() {
        fs::create_dir_all(&expanded_path).await.map_err(|err| {
            let message = format!(
                "Failed to create directory '{}': {}",
                expanded_path.display(),
                err
            );
            AppError::filesystem_context_with_source(err, message.into_boxed_str())
        })?;
    }

    let metadata = fs::metadata(&expanded_path).await?;
    if !metadata.is_dir() {
        return Err(AppError::filesystem_context(
            format!("Path '{}' is not a directory", expanded_path.display()).into_boxed_str(),
        ));
    }

    let test_file = expanded_path.join(".write_test");
    match fs::File::create(&test_file).await {
        Ok(_) => {
            let _ = fs::remove_file(&test_file).await;
            let canonical_path = fs::canonicalize(&expanded_path).await?;
            Ok(canonical_path)
        }
        Err(err) => {
            let message = format!(
                "Directory '{}' is not writable: {}",
                expanded_path.display(),
                err
            );
            Err(AppError::filesystem_context_with_source(
                err,
                message.into_boxed_str(),
            ))
        }
    }
}
