use std::borrow::Cow;

use crate::{
    app::{AuthLoginState, ConfigField, ConfigTab},
    config::{LogFormat, LogLevel, RetryFailedOnDownload, ThemeMode},
    download::ArchiveValidation,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets;
use super::{
    HELP_CUSTOM_MIRROR, HELP_OSU_OFFICIAL_LOCKED, bg_hover, bg_raised, mirror_label, success, text,
    text_dim, warning,
};
use osu_downloader::MirrorKind;

const PANEL_TITLE: &str = " CONFIG ";

const SECTION_DISPLAY: &str = "display";
const SECTION_DOWNLOAD: &str = "download";
const SECTION_MIRRORS: &str = "mirrors";
const SECTION_LOGGING: &str = "logging";
const SECTION_UPDATE: &str = "updates";

const LABEL_THEME: &str = "theme";
const LABEL_VIM_KEYS: &str = "vim keys";
const HELP_VIM_KEYS: &str = "hjkl move · gg/G top/bottom · ctrl+d/u page · i/a edit";

const LABEL_VIDEO: &str = "video";
const LABEL_VERIFY_INTEGRITY: &str = "verify .osz integrity";
const LABEL_RETRY_FAILED: &str = "retry failed on download";
const LABEL_AUTO_DEFER_RATE_LIMITED: &str = "auto-defer rate limited";
const LABEL_SKIP_IMPORTED: &str = "skip already imported";
const LABEL_LOGGING_ENABLED: &str = "enable logging";
const LABEL_LOGGING_LEVEL: &str = "log level";
const LABEL_LOGGING_FORMAT: &str = "log format";
const LABEL_AUTO_UPDATE: &str = "auto-update";
const LABEL_PRERELEASES: &str = "prerelease channel";
const HELP_PRERELEASES: &str = "off: stable only · on: also offer prerelease builds";

const CHIP_LOGGED_OUT: &str = " signed out";
const CHIP_LOGGED_IN: &str = " signed in";
const CHIP_ACTION_LOGIN: &str = "log in";
const CHIP_ACTION_MANAGE: &str = "manage";
const CHIP_ACTION_VIEW: &str = "view";
const CHIP_LOGGING_IN: &str = " logging in… ";
const CHIP_LOGIN_HINT: &str = "opens the login tab to enable the osu! official mirror";

const THEME_MODE_LABELS: &[&str] = &["full", "compatible"];

const LOG_LEVELS: &[&str] = &["error", "warn", "info", "debug", "trace"];
const LOG_FORMATS: &[&str] = &["compact", "pretty"];
const ARCHIVE_VALIDATION_LABELS: &[&str] = &["off", "basic", "strict"];
const RETRY_FAILED_LABELS: &[&str] = &["ask", "yes", "no"];

/// State-specific hint for the archive-validation cycle: each describes only
/// what the currently selected mode does.
fn archive_validation_help(mode: ArchiveValidation) -> &'static str {
    match mode {
        ArchiveValidation::Off => "checks only the file is not empty",
        ArchiveValidation::Magic => "verifies archive headers",
        ArchiveValidation::Eocd => {
            "also verifies eocd footer; turn off if many maps fail validation"
        }
    }
}

/// State-specific hint for the auto-update toggle: each describes what happens
/// on the next launch in the current state.
fn auto_update_help(auto: bool) -> &'static str {
    if auto {
        "download & install update on launch"
    } else {
        "notify only; press u for changelog & update"
    }
}

/// State-specific hint for the retry-failed cycle: each describes only what the
/// currently selected mode does.
fn retry_failed_help(mode: RetryFailedOnDownload) -> &'static str {
    match mode {
        RetryFailedOnDownload::Ask => "prompts before each download",
        RetryFailedOnDownload::Yes => "always retries failed maps",
        RetryFailedOnDownload::No => "never retries failed maps",
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    form: &ConfigTab,
    editing: bool,
    library_db_hint: &str,
) {
    let show_chrome = area.height >= super::COMPACT_HEIGHT;
    let items = build_config_items(form, show_chrome, editing, library_db_hint);

    let cursor_col = editing
        .then(|| {
            form.focused_input()
                .map(|f| widgets::input_cursor_col(f, 0))
        })
        .flatten();
    let (items, focused_index) = items.into_parts();
    widgets::render_scrollable_panel(
        frame,
        area,
        PANEL_TITLE,
        items,
        focused_index,
        form.focus != ConfigField::AuthChip,
        cursor_col,
        true,
        true,
        &form.list_offset,
    )
}

/// Builds the config form item list. `show_chrome` gates the decorative section
/// headers, spacers, and focus-conditional help lines; the focusable field rows
/// are identical in both modes so the field list lives here once.
///
/// Compact mode (`show_chrome == false`) strips only that chrome — every field
/// stays focusable and navigable.
fn build_config_items(
    form: &ConfigTab,
    show_chrome: bool,
    editing: bool,
    library_db_hint: &str,
) -> widgets::FormItems<ConfigField> {
    let focus = form.focus;
    let active_section = focus_section(focus);
    let mut items = widgets::FormItems::new(focus);

    items.push_focusable(ConfigField::AuthChip, auth_chip_item(form));
    if show_chrome && focus == ConfigField::AuthChip {
        items.push(widgets::help_item(CHIP_LOGIN_HINT));
    }
    if show_chrome {
        items.push(widgets::spacer());
        items.push(widgets::section_header(
            SECTION_DISPLAY,
            active_section == Some(SECTION_DISPLAY),
        ));
    }

    items.push_focusable(
        ConfigField::Theme,
        widgets::cycle_item(
            LABEL_THEME,
            THEME_MODE_LABELS,
            theme_mode_label(form.theme),
            focus == ConfigField::Theme,
            0,
        ),
    );
    items.push_focusable(
        ConfigField::VimKeys,
        widgets::row_item(
            LABEL_VIM_KEYS,
            None,
            form.vim_keys,
            focus == ConfigField::VimKeys,
            0,
        ),
    );
    if show_chrome && focus == ConfigField::VimKeys {
        items.push(widgets::help_item(HELP_VIM_KEYS));
    }
    if show_chrome {
        items.push(widgets::spacer());
        items.push(widgets::section_header(
            SECTION_MIRRORS,
            active_section == Some(SECTION_MIRRORS),
        ));
    }

    let logged_in = matches!(form.login_state, AuthLoginState::LoggedIn);
    // Rows follow the configured try-order (`ordered_mirror_rows`), matching the
    // nav order and the pipeline so what the user reorders is what gets tried.
    for (kind, field, on) in form.ordered_mirror_rows() {
        // Host is an informational hint, not a configurable value, so it is NOT
        // column-aligned (label_width 0) — it trails the mirror name.
        let item = if kind == MirrorKind::OsuApi && !logged_in {
            // osu! official needs a login: greyed + inert when logged out.
            widgets::disabled_toggle_row(
                mirror_label(kind),
                Some(kind.host()),
                on,
                focus == field,
                0,
            )
        } else {
            widgets::row_item(mirror_label(kind), Some(kind.host()), on, focus == field, 0)
        };
        items.push_focusable(field, item);
    }
    if show_chrome && focus == ConfigField::MirrorOsuOfficial && !logged_in {
        items.push(widgets::help_item(HELP_OSU_OFFICIAL_LOCKED));
    }
    for (idx, row) in form.custom_mirrors.rows().iter().enumerate() {
        let field = ConfigField::MirrorCustomUrl(idx);
        let focused = focus == field;
        items.push_focusable(field, widgets::input_item(row, focused, editing, 0));
        if show_chrome && focused {
            items.push(widgets::help_item(HELP_CUSTOM_MIRROR));
        }
    }
    if show_chrome {
        items.push(widgets::spacer());
        items.push(widgets::section_header(
            SECTION_DOWNLOAD,
            active_section == Some(SECTION_DOWNLOAD),
        ));
    }

    items.push_focusable(
        ConfigField::DownloadVideo,
        widgets::row_item(
            LABEL_VIDEO,
            None,
            form.video,
            focus == ConfigField::DownloadVideo,
            0,
        ),
    );
    items.push_focusable(
        ConfigField::DownloadThreads,
        widgets::stepper_item(
            form.threads.label,
            form.resolved_threads(),
            form.default_threads,
            focus == ConfigField::DownloadThreads,
            0,
        ),
    );
    items.push_focusable(
        ConfigField::DownloadArchiveValidation,
        widgets::cycle_item(
            LABEL_VERIFY_INTEGRITY,
            ARCHIVE_VALIDATION_LABELS,
            archive_validation_label(form.archive_validation),
            focus == ConfigField::DownloadArchiveValidation,
            0,
        ),
    );
    if show_chrome && focus == ConfigField::DownloadArchiveValidation {
        items.push(widgets::help_item(archive_validation_help(
            form.archive_validation,
        )));
    }
    items.push_focusable(
        ConfigField::RetryFailedOnDownload,
        widgets::cycle_item(
            LABEL_RETRY_FAILED,
            RETRY_FAILED_LABELS,
            retry_failed_label(form.retry_failed_on_download),
            focus == ConfigField::RetryFailedOnDownload,
            0,
        ),
    );
    if show_chrome && focus == ConfigField::RetryFailedOnDownload {
        items.push(widgets::help_item(retry_failed_help(
            form.retry_failed_on_download,
        )));
    }
    items.push_focusable(
        ConfigField::DownloadSkipAlreadyImported,
        widgets::row_item(
            LABEL_SKIP_IMPORTED,
            None,
            form.skip_already_imported,
            focus == ConfigField::DownloadSkipAlreadyImported,
            0,
        ),
    );
    if show_chrome && focus == ConfigField::DownloadSkipAlreadyImported {
        items.push(widgets::help_item(format!("checks {library_db_hint}")));
    }
    items.push_focusable(
        ConfigField::DownloadAutoSkipRateLimited,
        widgets::row_item(
            LABEL_AUTO_DEFER_RATE_LIMITED,
            None,
            form.auto_skip_rate_limited,
            focus == ConfigField::DownloadAutoSkipRateLimited,
            0,
        ),
    );
    items.push_focusable(
        ConfigField::DownloadRateLimitSkipSecs,
        widgets::input_item(
            &form.rate_limit_skip_secs,
            focus == ConfigField::DownloadRateLimitSkipSecs,
            editing,
            0,
        ),
    );
    if show_chrome {
        items.push(widgets::spacer());
        items.push(widgets::section_header(
            SECTION_LOGGING,
            active_section == Some(SECTION_LOGGING),
        ));
    }

    items.push_focusable(
        ConfigField::LoggingEnabled,
        widgets::row_item(
            LABEL_LOGGING_ENABLED,
            None,
            form.logging_enabled,
            focus == ConfigField::LoggingEnabled,
            0,
        ),
    );
    items.push_focusable(
        ConfigField::LoggingLevel,
        widgets::cycle_item(
            LABEL_LOGGING_LEVEL,
            LOG_LEVELS,
            log_level_label(form.logging_level),
            focus == ConfigField::LoggingLevel,
            0,
        ),
    );
    items.push_focusable(
        ConfigField::LoggingFormat,
        widgets::cycle_item(
            LABEL_LOGGING_FORMAT,
            LOG_FORMATS,
            log_format_label(form.logging_format),
            focus == ConfigField::LoggingFormat,
            0,
        ),
    );
    items.push_focusable(
        ConfigField::LoggingDirectory,
        widgets::input_item(
            &form.logging_dir,
            focus == ConfigField::LoggingDirectory,
            editing,
            0,
        ),
    );

    if show_chrome {
        items.push(widgets::spacer());
        items.push(widgets::section_header(
            SECTION_UPDATE,
            active_section == Some(SECTION_UPDATE),
        ));
    }
    items.push_focusable(
        ConfigField::AutoUpdate,
        widgets::row_item(
            LABEL_AUTO_UPDATE,
            None,
            form.auto_update,
            focus == ConfigField::AutoUpdate,
            0,
        ),
    );
    if show_chrome && focus == ConfigField::AutoUpdate {
        items.push(widgets::help_item(auto_update_help(form.auto_update)));
    }
    items.push_focusable(
        ConfigField::Prereleases,
        widgets::row_item(
            LABEL_PRERELEASES,
            None,
            form.prereleases,
            focus == ConfigField::Prereleases,
            0,
        ),
    );
    if show_chrome && focus == ConfigField::Prereleases {
        items.push(widgets::help_item(HELP_PRERELEASES));
    }

    items
}

/// The section a focused field belongs to, driving the active-section header
/// cue. `AuthChip` sits above every header, so it maps to no section.
fn focus_section(field: ConfigField) -> Option<&'static str> {
    use ConfigField::*;
    Some(match field {
        AuthChip => return None,
        Theme | VimKeys => SECTION_DISPLAY,
        MirrorOsuDirect | MirrorNerinyan | MirrorSayobot | MirrorNekoha | MirrorBeatconnect
        | MirrorOsudl | MirrorCatboy | MirrorHinamizawa | MirrorOsuOfficial
        | MirrorCustomUrl(_) => SECTION_MIRRORS,
        DownloadThreads
        | DownloadVideo
        | DownloadArchiveValidation
        | RetryFailedOnDownload
        | DownloadAutoSkipRateLimited
        | DownloadRateLimitSkipSecs
        | DownloadSkipAlreadyImported => SECTION_DOWNLOAD,
        LoggingEnabled | LoggingLevel | LoggingFormat | LoggingDirectory => SECTION_LOGGING,
        AutoUpdate | Prereleases => SECTION_UPDATE,
    })
}

/// Renders the auth state chip: a single styled row at the top of the config
/// tab. The action segment opens the dedicated login tab (which owns the actual
/// login / verify / logout flow); the state segment mirrors `login_state`.
///
/// - Signed in:   ` signed in   manage`
/// - Signed out:  ` signed out   log in`
/// - In progress: ` logging in…   view`
fn auth_chip_item(form: &ConfigTab) -> ListItem<'static> {
    let focused = form.focus == ConfigField::AuthChip;
    let chip_bg = Style::default().bg(bg_raised());

    // State segment (semantic when charged, TEXT_DIM neutral) then a 2-space
    // gap on the chip fill, then the action segment — no mid-dot separator.
    let (state, action_label) = match &form.login_state {
        AuthLoginState::LoggedOut => (
            Span::styled(CHIP_LOGGED_OUT, chip_bg.fg(text_dim())),
            CHIP_ACTION_LOGIN,
        ),
        AuthLoginState::InProgress(step) => {
            let label: Cow<'static, str> = if step.is_empty() {
                CHIP_LOGGING_IN.into()
            } else {
                format!(" {step} ").into()
            };
            // No italic — italic is reserved for panel/modal titles.
            (Span::styled(label, chip_bg.fg(warning())), CHIP_ACTION_VIEW)
        }
        AuthLoginState::LoggedIn => (
            Span::styled(CHIP_LOGGED_IN, chip_bg.fg(success())),
            CHIP_ACTION_MANAGE,
        ),
    };

    ListItem::new(Line::from(vec![
        state,
        Span::styled("  ", chip_bg),
        chip_action_span(action_label, focused, chip_bg),
    ]))
}

/// The chip's inline action segment.
///
/// Focused: highlighted like a selected row — `TEXT + bold` on `BG_HOVER` (the
/// row-selection tint), 1-space inset. The auth action reads as the focused row,
/// not an inverse accent block. Blurred: `TEXT_DIM` on the chip's `BG_RAISED`
/// fill with a trailing pad cell, no bold.
fn chip_action_span(label: &'static str, focused: bool, chip_bg: Style) -> Span<'static> {
    if focused {
        Span::styled(
            format!(" {label} "),
            Style::default().fg(text()).bg(bg_hover()).bold(),
        )
    } else {
        Span::styled(format!("{label} "), chip_bg.fg(text_dim()))
    }
}

fn log_level_label(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Trace => "trace",
    }
}

fn log_format_label(format: LogFormat) -> &'static str {
    match format {
        LogFormat::Compact => "compact",
        LogFormat::Pretty => "pretty",
    }
}

fn archive_validation_label(mode: ArchiveValidation) -> &'static str {
    match mode {
        ArchiveValidation::Off => "off",
        ArchiveValidation::Magic => "basic",
        ArchiveValidation::Eocd => "strict",
    }
}

fn retry_failed_label(mode: RetryFailedOnDownload) -> &'static str {
    match mode {
        RetryFailedOnDownload::Ask => "ask",
        RetryFailedOnDownload::Yes => "yes",
        RetryFailedOnDownload::No => "no",
    }
}

fn theme_mode_label(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::Full => "full",
        ThemeMode::Compatible => "compatible",
    }
}
