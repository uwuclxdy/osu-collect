use crate::{
    config::constants::{GB, KB, LOW_SPACE_THRESHOLD_BYTES, MB},
    utils::error::{AppError, Result},
};
use fs2::available_space;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};
use tokio::fs;

/// Strip the Windows extended-length ("verbatim") path prefix for display.
///
/// `std::fs::canonicalize` returns paths in `\\?\` form on Windows, which is
/// noise in any user-facing surface. Maps `\\?\C:\x` → `C:\x` and
/// `\\?\UNC\server\share` → `\\server\share`; returns the input untouched when
/// no such prefix is present (so it is a no-op for every non-Windows path).
fn strip_verbatim_prefix(s: &str) -> Cow<'_, str> {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        Cow::Owned(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        Cow::Borrowed(rest)
    } else {
        Cow::Borrowed(s)
    }
}

/// Format `path` for display: drop the Windows verbatim prefix, collapse a
/// leading home directory to `~`, and on Windows use `/` separators
/// (display-only, the filesystem never reads this string).
///
/// Returns `"~"` (a `&'static str` borrow) when `path` is exactly the home
/// directory, `"~/…"` (owned) when it is a subdirectory, and the cleaned path
/// (owned) for all other paths. The verbatim strip runs first so the home
/// collapse still matches a canonicalized Windows path.
pub fn pretty_path(path: impl AsRef<Path>) -> Cow<'static, str> {
    let raw = path.as_ref().to_string_lossy();
    let cleaned = strip_verbatim_prefix(&raw);
    let path = Path::new(cleaned.as_ref());
    if let Some(home) = dirs::home_dir() {
        if path == home {
            return Cow::Borrowed("~");
        }
        if let Ok(stripped) = path.strip_prefix(&home) {
            let collapsed = Path::new("~").join(stripped);
            #[cfg(windows)]
            let s = collapsed.to_string_lossy().into_owned().replace('\\', "/");
            #[cfg(not(windows))]
            let s = collapsed.to_string_lossy().into_owned();
            return Cow::Owned(s);
        }
    }
    #[cfg(windows)]
    let s = path.to_string_lossy().into_owned().replace('\\', "/");
    #[cfg(not(windows))]
    let s = path.to_string_lossy().into_owned();
    Cow::Owned(s)
}

/// Expand a leading `~` or `~/` (also `~\` on Windows) to the user's home
/// directory.
///
/// Only a bare `~` or `~` followed by a path separator is substituted, joined
/// with the platform separator. Absolute paths, relative paths, and `~` followed
/// by a non-separator character are returned unchanged.
pub fn expand_tilde(path: &str) -> String {
    if path == "~" {
        return dirs::home_dir()
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
    }
    // On Unix a backslash is a valid filename byte, so only `~/` is home-relative;
    // Windows also writes `~\`.
    let rest = path.strip_prefix("~/");
    #[cfg(windows)]
    let rest = rest.or_else(|| path.strip_prefix(r"~\"));
    if let Some(rest) = rest
        && let Some(home) = dirs::home_dir()
    {
        let mut s = home.to_string_lossy().into_owned();
        s.push(std::path::MAIN_SEPARATOR);
        s.push_str(rest);
        return s;
    }
    path.to_string()
}

pub fn is_low_disk_space(path: &Path) -> bool {
    available_space(path).is_ok_and(|space| space < LOW_SPACE_THRESHOLD_BYTES)
}

/// Format `bytes` with the given unit suffix.
///
/// Sizes use IEC binary units (`"B"` → `"3.45 GiB"`, 1024-based); rates use SI
/// decimal units (`"B/s"` → `"3.45 GB/s"`, 1000-based); storage is IEC,
/// throughput is SI.
pub fn format_bytes(bytes: u64, unit: &str) -> String {
    let bytes_f = bytes as f64;
    if unit.ends_with("/s") {
        // Throughput → SI decimal (KB/s, MB/s, GB/s).
        const SI_G: f64 = 1_000_000_000.0;
        const SI_M: f64 = 1_000_000.0;
        const SI_K: f64 = 1_000.0;
        if bytes_f >= SI_G {
            format!("{:.2} G{unit}", bytes_f / SI_G)
        } else if bytes_f >= SI_M {
            format!("{:.1} M{unit}", bytes_f / SI_M)
        } else if bytes_f >= SI_K {
            format!("{:.0} K{unit}", bytes_f / SI_K)
        } else {
            format!("{bytes} {unit}")
        }
    } else if bytes_f >= GB {
        format!("{:.2} Gi{unit}", bytes_f / GB)
    } else if bytes_f >= MB {
        format!("{:.1} Mi{unit}", bytes_f / MB)
    } else if bytes_f >= KB {
        format!("{:.0} Ki{unit}", bytes_f / KB)
    } else {
        format!("{bytes} {unit}")
    }
}

pub async fn prepare_directory(directory: &str) -> Result<PathBuf> {
    let expanded_path = PathBuf::from(expand_tilde(directory));

    if !expanded_path.exists() {
        fs::create_dir_all(&expanded_path).await.map_err(|err| {
            let message = format!(
                "Failed to create directory '{}': {}",
                expanded_path.display(),
                err
            );
            AppError::filesystem_source(err, message.into_boxed_str())
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
            Err(AppError::filesystem_source(err, message.into_boxed_str()))
        }
    }
}

/// Result of a directory tab-completion attempt.
#[derive(Debug, PartialEq)]
pub enum CompletionResult {
    /// Single unambiguous match — replace partial with full name + `/`.
    Single(String),
    /// Multiple matches — completed to the longest common prefix.
    /// The `candidates` list is for display (formatted as a comma-separated string).
    Ambiguous {
        completed: String,
        candidates: Vec<String>,
    },
    /// No matching directories found.
    NoMatch,
}

/// Attempt directory tab-completion on `value`.
///
/// Expands `~`, resolves the parent directory from the value, then lists
/// subdirectories whose names start with the partial last component.
///
/// Hidden entries (names starting with `.`) are only included when the partial
/// also starts with `.`. Symlinks resolving to directories are included.
/// Permission errors are silently skipped.
///
/// The returned `completed` / `Single` string is expressed in the same
/// tilde-style as the original `value` (i.e. the expanded parent prefix is
/// re-collapsed via [`pretty_path`] when the original started with `~`).
pub fn complete_dir(value: &str) -> CompletionResult {
    // Determine which base directory to list and what partial prefix to match.
    let (search_dir, partial, prefix) = resolve_completion_context(value);

    let mut matches = list_matching_dirs(&search_dir, &partial);
    matches.sort_unstable();

    match matches.as_slice() {
        [] => CompletionResult::NoMatch,
        [single] => {
            let completed = format!("{prefix}{single}/");
            CompletionResult::Single(completed)
        }
        _ => {
            let lcp = longest_common_prefix(&matches);
            let completed = format!("{prefix}{lcp}");
            CompletionResult::Ambiguous {
                completed,
                candidates: matches,
            }
        }
    }
}

/// Split `value` into `(search_dir, partial, display_prefix)`.
///
/// `display_prefix` is the part of the path before the partial component,
/// in the same tilde style as the input, used to reconstruct the completed
/// value without changing the user's notation.
fn resolve_completion_context(value: &str) -> (PathBuf, String, String) {
    let (parent_display, partial): (&str, &str) = if value.is_empty() {
        // Empty input: list cwd with no filter.
        ("", "")
    } else if value == "~" {
        // Bare `~` — treat as `~/`: list home with no filter, preserve `~/` prefix.
        ("~/", "")
    } else if let Some(slash_pos) = value.rfind('/') {
        // Split at the last slash: everything up-to-and-including the slash is
        // the display prefix; everything after is the partial to complete.
        (&value[..=slash_pos], &value[slash_pos + 1..])
    } else {
        // No slash — the whole value is a partial against cwd.
        ("", value)
    };

    let expanded_parent = if parent_display.is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        PathBuf::from(expand_tilde(parent_display.trim_end_matches('/')))
    };

    (
        expanded_parent,
        partial.to_string(),
        parent_display.to_string(),
    )
}

/// List subdirectory names inside `dir` whose names start with `partial`.
///
/// Hidden names are only returned when `partial` starts with `.`.
/// Non-directory entries (including broken symlinks) are skipped.
/// Read/permission errors are silently ignored.
fn list_matching_dirs(dir: &Path, partial: &str) -> Vec<String> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let include_hidden = partial.starts_with('.');

    read_dir
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !include_hidden && name.starts_with('.') {
                return None;
            }
            if !name.starts_with(partial) {
                return None;
            }
            // Follow symlinks: only keep entries that resolve to a directory.
            let file_type = entry.file_type().ok()?;
            if file_type.is_dir() {
                return Some(name);
            }
            if file_type.is_symlink() {
                let resolved = entry.path().canonicalize().ok()?;
                if resolved.is_dir() {
                    return Some(name);
                }
            }
            None
        })
        .collect()
}

/// Compute the longest common prefix of a non-empty slice of strings.
fn longest_common_prefix(names: &[String]) -> &str {
    let first = &names[0];
    // Count matching chars across all names, then convert to a byte offset so
    // we never slice mid-codepoint.
    let char_count = names[1..]
        .iter()
        .map(|name| {
            first
                .chars()
                .zip(name.chars())
                .take_while(|(a, b)| a == b)
                .count()
        })
        .min()
        .unwrap_or(first.chars().count());
    first
        .char_indices()
        .nth(char_count)
        .map_or(first, |(i, _)| &first[..i])
}

#[cfg(test)]
#[path = "../../tests/unit/utils_fs.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/home_completion.rs"]
mod completion_tests;
