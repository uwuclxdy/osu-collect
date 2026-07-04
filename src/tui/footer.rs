use crate::app::{
    App, ConfigField, ConfigTab, HomeField, HomeTab, LoginField, UpdatesField, UpdatesTab,
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

/// Download-page back key while running: `q` aborts the in-flight download.
const HINT_ABORT: &str = "q abort";
/// `esc`/`q` close key — a settled download page or the dynamic login tab. `x`
/// stays toast-only (a notification key, not a page action) so it isn't a back key.
const HINT_CLOSE: &str = "esc/q close";
const HINT_RETRY: &str = "r retry failed";
const HINT_DEFER_DROP: &str = "s defer · S drop";
/// Drop-only variant: shown when maps are queue-deferred but none are parked
/// inline, so `s defer` cannot act but `S drop` still drains the queue.
const HINT_DROP: &str = "S drop";

const HINT_MOVE: &str = "↑↓ move";
const HINT_SCROLL: &str = "↑↓ scroll";
/// ⇧↑↓ reorders the focused built-in mirror row in the Config try-order.
const HINT_REORDER: &str = "⇧↑↓ reorder";
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
/// Global `c` binding: switch the osu! client (stable ↔ lazer) from any tab.
const HINT_SWITCH_CLIENT: &str = "c switch client";
/// `x` dismisses the top toast; advertised only while one is visible.
const HINT_DISMISS: &str = "x dismiss";

/// Footer hint shown while a modal is open — discoverability lives in the
/// context-aware footer hint bar, not a per-modal hint row.
const HINT_MODAL_CLOSE: &str = "esc close";
/// Footer hint for button-carrying confirm modals — the buttons show the
/// choices, so only the universal cancel key is surfaced.
const HINT_MODAL_CANCEL: &str = "esc cancel";
/// Footer hint for the scrollable update-changelog modal: its body scrolls and
/// `esc` closes (the [later]/[update] buttons carry the choices).
const HINT_MODAL_SCROLL_CLOSE: &str = "↑↓ scroll  ·  esc close";

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
    } else if app.update_modal.is_some() {
        Some(HINT_MODAL_SCROLL_CLOSE.to_string())
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
    // Editing collapses the bar to the single exit affordance — no globals,
    // no universal help/back tail (the field owns every other key).
    if app.editing {
        return HINT_EDIT_DONE.to_string();
    }
    let (mut segments, back) = tab_hints(app);
    // App-global hints sit in the middle, ahead of the universal tail: `c`
    // (any tab), `u` (an update is pending), `x` (a toast is visible).
    segments.push(HINT_SWITCH_CLIENT);
    if app.available_update.is_some() {
        segments.push(HINT_UPDATE);
    }
    if !app.toasts.is_empty() {
        segments.push(HINT_DISMISS);
    }
    // Universal trailing pair (cloudy-tui hint-bar order): `? help`, then the
    // context-aware back/quit key last.
    segments.push(HINT_HELP);
    if let Some(back) = back {
        segments.push(back);
    }
    join(&segments)
}

/// Per-tab middle hints plus the context-aware trailing back/quit key. `? help`
/// and the global hints are layered on by [`hint_for`]; this returns only the
/// tab-specific action segments and which back/quit key (if any) trails the bar.
fn tab_hints(app: &App) -> (Vec<&'static str>, Option<&'static str>) {
    match app.active_tab() {
        HOME_TAB_INDEX => (home_hints(&app.home), Some(HINT_QUIT)),
        UPDATES_TAB_INDEX => updates_hints(&app.updates),
        CONFIG_TAB_INDEX => (config_hints(&app.config), Some(HINT_QUIT)),
        tab if app.is_login_tab(tab) => (login_hints(app), Some(HINT_CLOSE)),
        _ => download_hints(app),
    }
}

fn login_hints(app: &App) -> Vec<&'static str> {
    let mut segments = vec![HINT_MOVE];
    if let Some(login) = app.login.as_ref() {
        match login.focus {
            LoginField::Submit | LoginField::Resend => segments.push(HINT_ENTER_CONFIRM),
            field if field.is_text_input() => segments.push(HINT_EDIT),
            _ => {}
        }
    }
    segments
}

/// Download-page middle hints + trailing back key. `s defer` acts only on a row
/// parked on an inline cooldown now (`defer_rate_limited` wakes inline waiters);
/// `S drop` also drains deferred-pending queue items, so it shows under the
/// broader parked-or-deferred gate — matching `handle_download_tab_key` so
/// neither hint advertises a dead key. `r retry failed` shows only when the page
/// has a retryable failure (404s never are). The back key is `q abort` while
/// running, `esc/q close` once settled (`Completed`/`Failed`).
fn download_hints(app: &App) -> (Vec<&'static str>, Option<&'static str>) {
    let page = app.download_for_tab(app.active_tab());
    let settled = page
        .is_some_and(|page| matches!(page.stage, DownloadStage::Completed | DownloadStage::Failed));
    let mut segments = vec![HINT_SCROLL];
    if let Some(page) = page.filter(|page| matches!(page.stage, DownloadStage::Downloading)) {
        if page.any_active_rate_limited() {
            segments.push(HINT_DEFER_DROP);
        } else if page.rate_limited_or_deferred() {
            segments.push(HINT_DROP);
        }
    }
    if page.is_some_and(|page| !page.retryable_ids(None).is_empty()) {
        segments.push(HINT_RETRY);
    }
    let back = if settled { HINT_CLOSE } else { HINT_ABORT };
    (segments, Some(back))
}

fn join(segments: &[&str]) -> String {
    segments.join(HINT_SEPARATOR)
}

fn home_hints(form: &HomeTab) -> Vec<&'static str> {
    let mut segments = vec![HINT_MOVE];
    match form.focus {
        HomeField::Download => segments.push(HINT_ENTER_DOWNLOAD),
        HomeField::Mirrors => segments.push(HINT_ENTER_OPEN),
        f if f.is_stepper() => segments.push(HINT_PLUS_MINUS),
        f if f.is_toggle() => segments.push(HINT_ENTER_TOGGLE),
        f if f.is_text_input() => segments.push(HINT_EDIT),
        _ => {}
    }
    segments
}

fn updates_hints(form: &UpdatesTab) -> (Vec<&'static str>, Option<&'static str>) {
    // `r` rechecks known-bad maps from any non-editing focus (list or settled).
    let can_recheck = form.can_recheck_failed_maps();
    if form.selection.in_collection_list || form.selection.in_beatmap_list {
        let mut segments = vec![HINT_SCROLL, HINT_ENTER_TOGGLE, HINT_ALL_NONE];
        if form.selection.in_beatmap_list {
            segments.push(HINT_MARK_INSTALLED);
        }
        if can_recheck {
            segments.push(HINT_RECHECK);
        }
        // An open list ascends on esc/q rather than quitting; that back step is
        // left unadvertised (esc-to-go-back is universal), so no trailing key.
        return (segments, None);
    }

    let mut segments = vec![HINT_MOVE];
    match form.selection.focus {
        UpdatesField::Collections | UpdatesField::BeatmapList => segments.push(HINT_ENTER_OPEN),
        UpdatesField::Download => segments.push(HINT_ENTER_DOWNLOAD),
        UpdatesField::OsuPath => segments.push(HINT_EDIT),
    }
    if can_recheck {
        segments.push(HINT_RECHECK);
    }
    (segments, Some(HINT_QUIT))
}

fn config_hints(config: &ConfigTab) -> Vec<&'static str> {
    let mut segments = vec![HINT_MOVE];
    match config.focus {
        ConfigField::AuthChip => segments.push(HINT_ENTER_CONFIRM),
        field if field.is_stepper() => segments.push(HINT_PLUS_MINUS),
        field if field.is_text_input() => segments.push(HINT_EDIT),
        _ => segments.push(HINT_ENTER_TOGGLE),
    }
    // ⇧↑↓ reorders the focused built-in mirror row in the try-order.
    if config.focus_is_builtin_mirror() {
        segments.push(HINT_REORDER);
    }
    segments
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
