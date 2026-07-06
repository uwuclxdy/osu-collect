use std::{fs, io::Write, path::Path};

pub mod banner;
pub mod collection;
pub mod collection_state;
pub mod config;
pub mod custom_mirrors;
pub mod failed_maps;
pub mod home;
pub mod ignored_maps;
pub mod library;
pub mod library_cache;
pub mod login;
pub mod messages;
pub mod runtime;
pub mod search_source;
pub mod snapshots;
pub mod state;
pub mod tab;
pub mod toast;
pub mod update_source;

pub use banner::{Banner, BannerRecency, system_banners};
pub use collection::{ActiveDownloadLine, CollectionPage};
pub use config::{AuthLoginState, ConfigField, ConfigTab};
pub use custom_mirrors::CustomMirrorList;
pub use home::{GetMapsSource, HomeField, HomeTab, InputField, ResolveState};
pub use library::LibraryState;
pub use login::{LoginField, LoginPhase, LoginTab};
pub use runtime::run as run_app;
pub use search_source::{BrowseRow, SearchSource, SearchStatusMsg, SetBrowse};
pub use state::{App, AppCommand, UpdateIndicator};
pub use tab::Tab;
pub use toast::{Toast, ToastLevel, ToastTag, Toasts};
pub use update_source::{ScanCta, UpdateSource};

pub(crate) fn next_field<T: Copy + PartialEq>(fields: &[T], current: T) -> T {
    adjacent_field(fields, current, 1)
}

pub(crate) fn prev_field<T: Copy + PartialEq>(fields: &[T], current: T) -> T {
    adjacent_field(fields, current, fields.len().saturating_sub(1))
}

/// First focusable field (`gg` / Home). Falls back to `current` for the
/// degenerate empty slice so callers never index out of bounds.
pub(crate) fn first_field<T: Copy>(fields: &[T], current: T) -> T {
    fields.first().copied().unwrap_or(current)
}

/// Last focusable field (`G` / End).
pub(crate) fn last_field<T: Copy>(fields: &[T], current: T) -> T {
    fields.last().copied().unwrap_or(current)
}

fn adjacent_field<T: Copy + PartialEq>(fields: &[T], current: T, offset: usize) -> T {
    let idx = fields
        .iter()
        .position(|&field| field == current)
        .unwrap_or_default();
    fields[(idx + offset) % fields.len()]
}

fn write_atomic(path: &Path, tmp_extension: &str, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension(tmp_extension);
    let write_result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }

    write_result
}
