use crate::app::{
    App, ConfigField, HomeField, HomeTab, LoginField, UpdatesField, UpdatesTab,
    messages::AppMessage,
};
use crate::config::constants::{CONFIG_TAB_INDEX, HOME_TAB_INDEX, UPDATES_TAB_INDEX};
use crate::download::DownloadStage;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::{accent, bg_raised, spinner_str, text_dim, text_faint, warning};

const HINT_SEPARATOR: &str = "  ·  ";
/// Rendered gap between hint groups: 3 spaces, no glyph.
const HINT_GROUP_GAP: &str = "   ";

/// Footer-alert prefix glyph (` ! `) for the quit prompt, in semantic color.
const ALERT_WARN: &str = " ! ";

/// Right-edge indicator shown whenever the vim keymap is enabled.
const VIM_CHIP: &str = " vim ";

const QUIT_PROMPT_TEXT: &str = "press q again to quit";
const QUIT_PROMPT_TEXT_DOWNLOADS: &str = "press q again to quit · active downloads will stop";

const DOWNLOAD_TAB_HINT_RUNNING: &str = "↑↓ scroll  ·  q abort  ·  ? help";
// `esc`/`q` both close a settled page. `x` is toast-only (a notification key,
// not a download-page action) so it isn't advertised here.
const DOWNLOAD_TAB_HINT_SETTLED: &str = "↑↓ scroll  ·  esc/q close  ·  ? help";
const HINT_RETRY: &str = "r retry failed";
const HINT_DEFER_DROP: &str = "s defer · S drop";
/// Drop-only variant: shown when maps are queue-deferred but none are parked
/// inline, so `s defer` cannot act but `S drop` still drains the queue.
const HINT_DROP: &str = "S drop";

const HINT_MOVE: &str = "↑↓ move";
const HINT_SCROLL: &str = "↑↓ scroll";
const HINT_ENTER_TOGGLE: &str = "↵ toggle";
const HINT_ENTER_OPEN: &str = "↵ open";
const HINT_ENTER_CONFIRM: &str = "↵ confirm";
const HINT_ENTER_DOWNLOAD: &str = "↵ download";
/// Text-input row, selected-not-editing: enter descends into edit mode.
const HINT_EDIT: &str = "↵ edit";
/// While editing a text field: esc (or enter) exits back to selected.
const HINT_EDIT_DONE: &str = "esc done";
const HINT_PLUS_MINUS: &str = "+/- adjust";
const HINT_ALL_NONE: &str = "a all / d none";
const HINT_RECHECK: &str = "r recheck";
const HINT_MARK_INSTALLED: &str = "i installed / I all";
const HINT_QUIT: &str = "q quit";
const HINT_HELP: &str = "? help";
const HINT_UPDATE: &str = "u update";
/// Close hint for the dynamic, closeable login tab.
const HINT_LOGIN_CLOSE: &str = "esc/q close";

/// Footer hint shown while a modal is open — discoverability lives in the
/// context-aware footer hint bar, not a per-modal hint row.
const HINT_MODAL_CLOSE: &str = "esc close";
/// Footer hint for button-carrying confirm modals — the buttons show the
/// choices, so only the universal cancel key is surfaced.
const HINT_MODAL_CANCEL: &str = "esc cancel";

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    // The vim-keymap indicator claims the footer's right edge regardless of
    // which hint/alert branch renders the rest of the bar.
    let area = if app.config.vim_keys {
        render_vim_chip(frame, area)
    } else {
        area
    };

    // A modal owns the footer while open: show its context-aware keys.
    if let Some(hint) = modal_hint(app) {
        frame.render_widget(Paragraph::new(hint_line(&hint)), area);
        return;
    }

    if app.home.quit_prompt {
        frame.render_widget(quit_prompt_paragraph(!app.downloads.is_empty()), area);
        return;
    }

    if let Some(msg) = current_message(app) {
        frame.render_widget(Paragraph::new(message_line(msg, app.tick_count)), area);
        return;
    }

    frame.render_widget(Paragraph::new(hint_line(&hint_for(app))), area);
}

/// Context-aware footer keys for whichever modal is open, or `None` when no
/// modal is up. The confirm/retry modals now carry their choices as on-screen
/// buttons (←/→ + `enter`), so the footer only needs the universal `esc cancel`
/// rather than re-listing every key.
/// Draws the `vim` keymap badge flush to the footer's right edge and returns the
/// area left of it for the hint bar — a dim label on the raised chip fill,
/// matching the config auth-chip idiom. Skipped (returns the full area) when the
/// footer is too narrow to fit the badge.
fn render_vim_chip(frame: &mut Frame, area: Rect) -> Rect {
    let chip_w = VIM_CHIP.len() as u16;
    if area.width <= chip_w {
        return area;
    }
    let chip_area = Rect {
        x: area.right() - chip_w,
        y: area.y,
        width: chip_w,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            VIM_CHIP,
            Style::default().fg(text_dim()).bg(bg_raised()),
        )),
        chip_area,
    );
    Rect {
        width: area.width - chip_w,
        ..area
    }
}

fn modal_hint(app: &App) -> Option<String> {
    if app.help_open {
        Some(HINT_MODAL_CLOSE.to_string())
    } else if app.confirm_retry_on_start.is_some() || app.confirm_retry.is_some() {
        Some(HINT_MODAL_CANCEL.to_string())
    } else {
        None
    }
}

fn current_message(app: &App) -> Option<&AppMessage> {
    match app.active_tab() {
        HOME_TAB_INDEX => app.home.message.as_ref(),
        UPDATES_TAB_INDEX => app.updates.message.as_ref(),
        CONFIG_TAB_INDEX => app.config.message.as_ref(),
        // The login flow surfaces its in-progress status via `config.message`.
        tab if app.is_login_tab(tab) => app.config.message.as_ref(),
        _ => None,
    }
}

fn hint_for(app: &App) -> String {
    let mut hint = match app.active_tab() {
        HOME_TAB_INDEX => home_hint(&app.home, app.editing),
        UPDATES_TAB_INDEX => updates_hint(&app.updates, app.editing),
        CONFIG_TAB_INDEX => config_hint(app.config.focus, app.editing),
        tab if app.is_login_tab(tab) => login_hint(app, app.editing),
        _ => download_tab_hint(app),
    };
    // A pending notify-only update is actionable from any tab via `u`.
    if app.available_update.is_some() {
        if hint.is_empty() {
            hint = HINT_UPDATE.to_string();
        } else {
            hint = format!("{hint}{HINT_SEPARATOR}{HINT_UPDATE}");
        }
    }
    hint
}

fn login_hint(app: &App, editing: bool) -> String {
    if editing {
        return join(&[HINT_EDIT_DONE]);
    }
    let mut segments = vec![HINT_MOVE];
    if let Some(login) = app.login.as_ref() {
        match login.focus {
            LoginField::Submit | LoginField::Resend => segments.push(HINT_ENTER_CONFIRM),
            field if field.is_text_input() => segments.push(HINT_EDIT),
            _ => {}
        }
    }
    segments.push(HINT_LOGIN_CLOSE);
    segments.push(HINT_HELP);
    join(&segments)
}

/// `esc/q close` appears only when the active download page is settled
/// (`Completed` or `Failed`). In-progress pages keep the `q abort` hint.
/// A retry segment is appended whenever the active page has failed maps.
fn download_tab_hint(app: &App) -> String {
    let page = app.download_for_tab(app.active_tab());
    let settled = page
        .is_some_and(|page| matches!(page.stage, DownloadStage::Completed | DownloadStage::Failed));
    let base = if settled {
        DOWNLOAD_TAB_HINT_SETTLED
    } else {
        DOWNLOAD_TAB_HINT_RUNNING
    };
    let mut segments = vec![base];
    // `s defer` can act only on a row parked on an inline cooldown right now
    // (`defer_rate_limited` wakes inline waiters); `S drop` also drains
    // deferred-pending queue items, so it shows under the broader parked-or-
    // deferred gate. Matches the key gates in `handle_download_tab_key` so
    // neither hint advertises a dead key.
    if let Some(page) = page.filter(|page| matches!(page.stage, DownloadStage::Downloading)) {
        if page.any_active_rate_limited() {
            segments.push(HINT_DEFER_DROP);
        } else if page.rate_limited_or_deferred() {
            segments.push(HINT_DROP);
        }
    }
    // Advertise `r retry failed` only when something is actually retryable —
    // 404 (NotFound) failures are never retryable, so a page of pure 404s must
    // not show a hint whose key does nothing.
    let has_retryable = page.is_some_and(|page| !page.retryable_ids(None).is_empty());
    if has_retryable {
        segments.push(HINT_RETRY);
    }
    join(&segments)
}

fn join(segments: &[&str]) -> String {
    segments.join(HINT_SEPARATOR)
}

fn home_hint(form: &HomeTab, editing: bool) -> String {
    if editing {
        return join(&[HINT_EDIT_DONE]);
    }
    let mut segments = vec![HINT_MOVE];
    match form.focus {
        HomeField::Download => segments.push(HINT_ENTER_DOWNLOAD),
        f if f.is_stepper() => segments.push(HINT_PLUS_MINUS),
        f if f.is_toggle() => segments.push(HINT_ENTER_TOGGLE),
        f if f.is_text_input() => segments.push(HINT_EDIT),
        _ => {}
    }
    // Outside edit mode `q` quits (it is not captured by the field).
    segments.push(HINT_QUIT);
    segments.push(HINT_HELP);
    join(&segments)
}

fn updates_hint(form: &UpdatesTab, editing: bool) -> String {
    if editing {
        return join(&[HINT_EDIT_DONE]);
    }
    // `r` rechecks known-bad maps from any non-editing focus (list or settled).
    let can_recheck = form.can_recheck_failed_maps();
    let in_list = form.selection.in_collection_list || form.selection.in_beatmap_list;
    if in_list {
        let mut segments = vec![HINT_SCROLL, HINT_ENTER_TOGGLE, HINT_ALL_NONE];
        if form.selection.in_beatmap_list {
            segments.push(HINT_MARK_INSTALLED);
        }
        if can_recheck {
            segments.push(HINT_RECHECK);
        }
        segments.push(HINT_HELP);
        return join(&segments);
    }

    let mut segments = vec![HINT_MOVE];
    match form.selection.focus {
        UpdatesField::ClientType => segments.push(HINT_ENTER_TOGGLE),
        UpdatesField::Collections | UpdatesField::BeatmapList => segments.push(HINT_ENTER_OPEN),
        UpdatesField::Download => segments.push(HINT_ENTER_DOWNLOAD),
        UpdatesField::OsuPath => segments.push(HINT_EDIT),
    }
    if can_recheck {
        segments.push(HINT_RECHECK);
    }
    segments.push(HINT_QUIT);
    segments.push(HINT_HELP);
    join(&segments)
}

fn config_hint(focus: ConfigField, editing: bool) -> String {
    if editing {
        return join(&[HINT_EDIT_DONE]);
    }
    let mut segments = vec![HINT_MOVE];
    match focus {
        ConfigField::AuthChip => segments.push(HINT_ENTER_CONFIRM),
        field if field.is_stepper() => segments.push(HINT_PLUS_MINUS),
        field if field.is_text_input() => segments.push(HINT_EDIT),
        _ => segments.push(HINT_ENTER_TOGGLE),
    }
    // Config edits apply immediately (no save step); `q` quits outside edit mode.
    segments.push(HINT_QUIT);
    segments.push(HINT_HELP);
    join(&segments)
}

fn quit_prompt_paragraph(has_downloads: bool) -> Paragraph<'static> {
    let text = if has_downloads {
        QUIT_PROMPT_TEXT_DOWNLOADS
    } else {
        QUIT_PROMPT_TEXT
    };
    Paragraph::new(Line::from(vec![
        Span::styled(ALERT_WARN, Style::default().fg(warning())),
        Span::styled(text, Style::default().fg(text_dim())),
    ]))
}

/// Footer loading line: a spinner + the in-progress status in `TEXT_DIM`.
/// Results and errors no longer appear here — they surface as toasts.
fn message_line(msg: &AppMessage, tick: u64) -> Line<'static> {
    Line::from(vec![
        Span::styled(spinner_str(tick), Style::default().fg(accent()).bold()),
        Span::styled(
            msg.text.trim_start().to_string(),
            Style::default().fg(text_dim()),
        ),
    ])
}

fn hint_line(hint: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let label_style = Style::default().fg(text_faint());
    let key_style = Style::default().fg(accent()).bold();

    for (index, segment) in hint.split('·').enumerate() {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        if index > 0 {
            // Hint groups are 3-space separated, no glyph.
            spans.push(Span::raw(HINT_GROUP_GAP));
        } else {
            spans.push(Span::raw(" "));
        }
        let mut parts = trimmed.splitn(2, ' ');
        let key = parts.next().unwrap_or("");
        let label = parts.next().unwrap_or("");
        spans.push(Span::styled(key.to_string(), key_style));
        if !label.is_empty() {
            spans.push(Span::styled(format!(" {label}"), label_style));
        }
    }

    Line::from(spans)
}

#[cfg(test)]
#[path = "../../tests/unit/tui_footer.rs"]
mod tests;
