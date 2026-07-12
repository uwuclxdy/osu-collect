use crate::app::{
    App, ConfigField, ConfigTab, FindBackend, GetMapsSource, HomeField, HomeTab, LoginField, Tab,
    messages::AppMessage,
};
use crate::download::DownloadStage;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::{accent, bg_raised, spinner_str, text_dim, text_faint, warning, widgets};

const HINT_SEPARATOR: &str = "  ·  ";
/// Rendered gap between hint groups: 3 spaces, no glyph.
const HINT_GROUP_GAP: &str = "   ";

/// Footer-alert prefix glyph (` ! `) for the quit prompt, in semantic color.
const ALERT_WARN: &str = " ! ";

/// Right-edge indicator shown whenever the vim keymap is enabled.
const VIM_CHIP: &str = " vim ";

const QUIT_PROMPT_TEXT: &str = "press [q] again to quit";
const QUIT_PROMPT_TEXT_DOWNLOADS: &str = "press [q] again to quit · active downloads will stop";

/// Downloads preview, run still in flight: `q` cancels it (esc/← only ascend).
/// Must stay advertised — `q` is destructive here.
const HINT_CANCEL: &str = "q cancel";
/// `esc`/`q` close key for the login split. `x` stays toast-only (a
/// notification key, not a page action) so it isn't a back key.
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
const HINT_SOURCE: &str = "↵ switch source";
/// Get Maps: jump straight to a source by its strip digit.
const HINT_SOURCE_JUMP: &str = "1-3 source";
/// Find form chip (preset / special / mode / status / sort): `space` cycles it.
const HINT_CYCLE: &str = "space cycle";
/// Find form's CTA: run the resolved query (osu search or nzbasic filter).
const HINT_FIND: &str = "↵ find";
/// Find browse (list pane): load the next page of results.
const HINT_MORE: &str = "m more";
const HINT_ENTER_TOGGLE: &str = "↵ toggle";
const HINT_ENTER_OPEN: &str = "↵ open";
const HINT_ENTER_CONFIRM: &str = "↵ confirm";
const HINT_ENTER_DOWNLOAD: &str = "↵ download";
/// Update source's scan CTA (form focus): run the scan / descend into the browse.
const HINT_SCAN: &str = "↵ scan";
/// Update browse: focus the preview pane / return to the collections list.
const HINT_FOCUS_PREVIEW: &str = "→ preview";
const HINT_FOCUS_LIST: &str = "← list";
/// Text-input row, selected-not-editing: enter descends into edit mode.
const HINT_EDIT: &str = "↵ edit";
/// While editing a text field: esc (or enter) exits back to selected.
const HINT_EDIT_DONE: &str = "esc done";
const HINT_PLUS_MINUS: &str = "+/- adjust";
/// Update browse (list pane): select every collection / none.
const HINT_SELECT_ALL_NONE: &str = "a all / A none";
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
        // Warn about stopping downloads only when one is actually in flight —
        // settled pages are retained on the Downloads tab and quit clean.
        frame.render_widget(quit_prompt_paragraph(app.is_downloading()), area);
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
        // The update source carries the scan-progress loading line; the other
        // sources use the shared home message.
        Tab::Home if app.home.source == GetMapsSource::Update => app.home.update.message.as_ref(),
        Tab::Home => app.home.message.as_ref(),
        // The login split lives on Config and surfaces its in-progress status
        // via `config.message`, so the Config arm covers it too.
        Tab::Config => app.config.message.as_ref(),
        Tab::Downloads => None,
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
    // The login split traps focus while open; its keys own the bar (esc/q close).
    if app.login_open() {
        return (login_hints(app), Some(HINT_CLOSE));
    }
    match app.active_tab() {
        Tab::Home => home_tab_hints(&app.home),
        Tab::Config => (config_hints(&app.config), Some(HINT_QUIT)),
        Tab::Downloads => downloads_hints(app),
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

/// Downloads-tab middle hints + trailing back key.
///
/// List focused: select/open keys, `q quit` trails. Preview focused, the
/// download-control keys scoped there: `s defer` acts only on a row parked on
/// an inline cooldown now (`defer_rate_limited` wakes inline waiters); `S
/// drop` also drains deferred-pending queue items, so it shows under the
/// broader parked-or-deferred gate — matching `handle_download_tab_key` so
/// neither hint advertises a dead key. `r retry failed` shows only when the
/// page has a retryable failure (404s never are). While the previewed run is
/// in flight the trailing key is `q cancel` (destructive, must be advertised);
/// a settled run / history record ascends on esc, unadvertised like every other
/// browse.
fn downloads_hints(app: &App) -> (Vec<&'static str>, Option<&'static str>) {
    if !app.downloads_tab.preview_focused {
        let segments = if app.downloads_rows().is_empty() {
            Vec::new()
        } else {
            vec![HINT_MOVE, HINT_ENTER_OPEN]
        };
        return (segments, Some(HINT_QUIT));
    }

    let page = app.selected_download_page();
    let running = page.is_some_and(|page| !page.is_settled());
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
    segments.push(HINT_FOCUS_LIST);
    let back = running.then_some(HINT_CANCEL);
    (segments, back)
}

fn join(segments: &[&str]) -> String {
    segments.join(HINT_SEPARATOR)
}

/// Home-tab middle hints + trailing back key. The update source has its own
/// form / browse hint sets; the other sources use the standard form hints and
/// trail `q quit`.
fn home_tab_hints(form: &HomeTab) -> (Vec<&'static str>, Option<&'static str>) {
    if form.source == GetMapsSource::Update {
        return update_source_hints(form);
    }
    if let Some(browse) = active_set_browse(form) {
        return set_browse_hints(form, browse);
    }
    (home_form_hints(form), Some(HINT_QUIT))
}

/// The active source's flat browse when it is descended, else `None`.
fn active_set_browse(form: &HomeTab) -> Option<&crate::app::SetBrowse> {
    match form.source {
        GetMapsSource::Find if form.find.browse.is_browsing() => Some(&form.find.browse),
        GetMapsSource::Collection if form.collection_browse.is_browsing() => {
            Some(&form.collection_browse)
        }
        _ => None,
    }
}

fn set_browse_hints(
    form: &HomeTab,
    browse: &crate::app::SetBrowse,
) -> (Vec<&'static str>, Option<&'static str>) {
    // The browse ascends on esc rather than quitting; that back step is left
    // unadvertised (esc-to-go-back is universal), so no trailing key.
    if browse.preview_focused() {
        return (vec![HINT_SCROLL, HINT_FOCUS_LIST], None);
    }
    let mut segments = vec![
        HINT_SCROLL,
        HINT_ENTER_TOGGLE,
        HINT_SELECT_ALL_NONE,
        HINT_FOCUS_PREVIEW,
    ];
    // `m` loads more: the next osu results page, or the next osu-batch enrichment
    // page for an id-only browse (nzbasic find results / collection browse&pick)
    // — matching the `m` key handler.
    let more = match form.source {
        GetMapsSource::Find => match form.find.results_backend() {
            Some(FindBackend::Osu) => form.find.next_cursor.is_some(),
            Some(FindBackend::Nzbasic) => form.find.browse.has_more_enrichment(),
            None => false,
        },
        GetMapsSource::Collection => form.collection_browse.has_more_enrichment(),
        GetMapsSource::Update => false,
    };
    if more {
        segments.push(HINT_MORE);
    }
    (segments, None)
}

fn home_form_hints(form: &HomeTab) -> Vec<&'static str> {
    let mut segments = vec![HINT_MOVE];
    match form.focus {
        HomeField::Source => segments.push(HINT_SOURCE),
        HomeField::Download => segments.push(HINT_ENTER_DOWNLOAD),
        HomeField::CollectionBrowse | HomeField::FindBrowse => segments.push(HINT_ENTER_OPEN),
        HomeField::Mirrors => segments.push(HINT_ENTER_OPEN),
        HomeField::FindRun => segments.push(HINT_FIND),
        f if f.is_find_chip() => segments.push(HINT_CYCLE),
        f if f.is_stepper() => segments.push(HINT_PLUS_MINUS),
        f if f.is_toggle() => segments.push(HINT_ENTER_TOGGLE),
        f if f.is_text_input() => segments.push(HINT_EDIT),
        _ => {}
    }
    segments.push(HINT_SOURCE_JUMP);
    segments
}

fn update_source_hints(form: &HomeTab) -> (Vec<&'static str>, Option<&'static str>) {
    let update = &form.update;
    // `r` rechecks known-bad maps from any non-editing focus.
    let can_recheck = update.can_recheck_failed_maps();

    if update.is_browsing() {
        let mut segments = if update.preview_focused() {
            vec![HINT_SCROLL, HINT_MARK_INSTALLED, HINT_FOCUS_LIST]
        } else {
            vec![
                HINT_SCROLL,
                HINT_ENTER_TOGGLE,
                HINT_SELECT_ALL_NONE,
                HINT_FOCUS_PREVIEW,
            ]
        };
        if can_recheck {
            segments.push(HINT_RECHECK);
        }
        // The browse ascends on esc rather than quitting; that back step is left
        // unadvertised (esc-to-go-back is universal), so no trailing key.
        return (segments, None);
    }

    let mut segments = vec![HINT_MOVE];
    match form.focus {
        HomeField::Source => segments.push(HINT_SOURCE),
        HomeField::UpdateScan => segments.push(HINT_SCAN),
        HomeField::UpdateBrowse => segments.push(HINT_ENTER_OPEN),
        HomeField::UpdateOsuPath => segments.push(HINT_EDIT),
        HomeField::Download => segments.push(HINT_ENTER_DOWNLOAD),
        _ => {}
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
    let mut spans = vec![Span::styled(ALERT_WARN, Style::default().fg(warning()))];
    spans.extend(widgets::keyed_spans(
        text,
        Style::default().fg(accent()).bold(),
        Style::default().fg(text_dim()),
    ));
    Paragraph::new(Line::from(spans))
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
