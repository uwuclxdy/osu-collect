use crate::app::{
    App, ConfigField, ConfigTab, EnrichSink, FindBackend, GetMapsSource, HomeField, HomeTab,
    LoginField, Tab, messages::AppMessage,
};
use crate::download::DownloadStage;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Style, Stylize},
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
const HINT_CLOSE: &str = "esc / q close";
const HINT_RETRY: &str = "r retry failed";
const HINT_DEFER_DROP: &str = "s defer · S drop";
/// Drop-only variant: shown when maps are queue-deferred but none are parked
/// inline, so `s defer` cannot act but `S drop` still drains the queue.
const HINT_DROP: &str = "S drop";

const HINT_MOVE: &str = "↑↓ move";
const HINT_SCROLL: &str = "↑↓ scroll";
/// ⇧↑↓ reorders the focused built-in mirror row in the Config try-order.
const HINT_REORDER: &str = "⇧↑↓ reorder";
/// Source strip focused: `↵` cycles and `1`-`3` jump, both switch source —
/// merged so the word "source" isn't repeated down the bar.
const HINT_SOURCE_SWITCH: &str = "↵ / 1-3 switch source";
/// Any other row: the strip digits still switch source.
const HINT_SOURCE_JUMP: &str = "1-3 switch source";
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
const HINT_PLUS_MINUS: &str = "+ / - adjust";
/// Update browse (list pane): select every collection / none.
const HINT_SELECT_ALL_NONE: &str = "a all / A none";
/// Update browse (either pane): `s` cycles the focused pane's sort.
const HINT_SORT: &str = "s sort";
const HINT_RECHECK: &str = "r recheck";
const HINT_MARK_INSTALLED: &str = "i install / I all";
const HINT_RESTORE: &str = "u restore / U all";
const HINT_QUIT: &str = "q quit";
const HINT_HELP: &str = "? help";
const HINT_UPDATE: &str = "u update";
/// Global `c` binding: switch the osu! client (stable ↔ lazer) from any tab.
const HINT_SWITCH_CLIENT: &str = "c switch client";
/// `x` dismisses the top toast; advertised only while one is visible.
const HINT_DISMISS: &str = "x dismiss";

// Modal discoverability lives in the context-aware footer hint bar, not a
// per-modal hint row.
/// Footer hint for button-carrying confirm modals — the buttons show the
/// choices, so only the universal cancel key is surfaced.
const HINT_MODAL_CANCEL: &str = "esc cancel";
/// Footer hint for a scrollable modal — both the help overlay and the update
/// changelog scroll the body and close on `esc` (the changelog's
/// [later]/[update] buttons carry its choices).
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

    let mut segments = hint_segments(app);
    trim_to_fit(&mut segments, area.width as usize);
    frame.render_widget(Paragraph::new(hint_line(&join_segments(&segments))), area);
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
        Paragraph::new(VIM_CHIP.fg(text_dim()).bg(bg_raised())),
        chip_area,
    );
    Rect {
        width: area.width - chip_w,
        ..area
    }
}

fn modal_hint(app: &App) -> Option<String> {
    if app.help_open {
        Some(HINT_MODAL_SCROLL_CLOSE.to_string())
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

/// The full, width-agnostic hint string for `app` — every key the current
/// state advertises, in display order. [`render`] narrows this to the terminal
/// width via [`hint_segments`] + [`trim_to_fit`]; this builder is the superset
/// the unit tests assert against.
#[cfg(test)]
pub(crate) fn hint_for(app: &App) -> String {
    join_segments(&hint_segments(app))
}

/// Every advertised hint for `app`, tagged with the [`HintSegment::drop_rank`]
/// that decides which hints vanish first when the bar would overflow. Rank 0
/// (context actions, the edit-affordance) is never dropped; the back/quit key
/// ([`RANK_BACK`]) outlasts the app-globals so the way out is the last hint
/// trimmed.
fn hint_segments(app: &App) -> Vec<HintSegment> {
    // Editing collapses the bar to the single exit affordance — no globals,
    // no universal help/back tail (the field owns every other key).
    if app.editing {
        return vec![HintSegment::context(HINT_EDIT_DONE)];
    }
    let (context, back) = tab_hints(app);
    let mut segments: Vec<HintSegment> = context.into_iter().map(HintSegment::context).collect();
    // App-global hints sit in the middle, ahead of the universal tail: `c`
    // (any tab), `u` (an update is pending), `x` (a toast is visible). `c` is
    // suppressed while the login split traps focus — it is gated
    // `login.is_none()` there, so advertising it would promise a dead key.
    if !app.login_open() {
        segments.push(HintSegment::global(HINT_SWITCH_CLIENT, RANK_SWITCH_CLIENT));
    }
    if app.available_update.is_some() && !app.home.update.is_browsing() {
        segments.push(HintSegment::global(HINT_UPDATE, RANK_UPDATE));
    }
    if !app.toasts.is_empty() {
        segments.push(HintSegment::global(HINT_DISMISS, RANK_DISMISS));
    }
    // Universal trailing pair (cloudy-tui hint-bar order): `? help`, then the
    // context-aware back/quit key last.
    segments.push(HintSegment::global(HINT_HELP, RANK_HELP));
    if let Some(back) = back {
        segments.push(HintSegment::global(back, RANK_BACK));
    }
    segments
}

/// Drop priorities for the app-global hints when the bar must shrink. Higher
/// ranks vanish first; the back/quit key outlasts every global so the exit is
/// the last thing trimmed. Context actions are rank 0 and never drop.
const RANK_BACK: u8 = 1;
const RANK_HELP: u8 = 2;
const RANK_SWITCH_CLIENT: u8 = 3;
const RANK_DISMISS: u8 = 4;
const RANK_UPDATE: u8 = 5;

/// One footer hint plus the rank that orders it for width-trimming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HintSegment {
    text: &'static str,
    drop_rank: u8,
}

impl HintSegment {
    /// A context action tied to the focused surface — kept until the bar can
    /// no longer fit even after every global and the back key are gone.
    const fn context(text: &'static str) -> Self {
        Self { text, drop_rank: 0 }
    }

    /// A global/universal hint, droppable at `rank`.
    const fn global(text: &'static str, rank: u8) -> Self {
        Self {
            text,
            drop_rank: rank,
        }
    }
}

fn join_segments(segments: &[HintSegment]) -> String {
    segments
        .iter()
        .map(|s| s.text)
        .collect::<Vec<_>>()
        .join(HINT_SEPARATOR)
}

/// Columns the rendered hint line would occupy for `segments`, mirroring
/// [`hint_line`]'s layout: a leading space, then `·`-split groups joined by
/// [`HINT_GROUP_GAP`] (3 cols). Group width is `chars().count()` — the hint
/// glyph set (arrows, `·`, ASCII) is all single-column, so this tracks display
/// columns without a unicode-width dependency.
fn rendered_width(segments: &[HintSegment]) -> usize {
    let groups: Vec<&str> = segments
        .iter()
        .flat_map(|s| s.text.split('·'))
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .collect();
    if groups.is_empty() {
        return 0;
    }
    let body: usize = groups.iter().map(|g| g.chars().count()).sum();
    let gaps = (groups.len() - 1) * HINT_GROUP_GAP.chars().count();
    1 + body + gaps
}

/// Drop the lowest-priority hints until the line fits `budget` columns. Only
/// rank > 0 (global/back) hints are eligible; once only context actions remain
/// the bar is left to truncate at the terminal edge rather than hide the keys
/// that matter for the focused surface.
fn trim_to_fit(segments: &mut Vec<HintSegment>, budget: usize) {
    while rendered_width(segments) > budget {
        // `max_by_key` resolves rank ties to the last occurrence, but every
        // global/back rank is unique, so this is the single highest-rank hint.
        let drop = segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.drop_rank > 0)
            .max_by_key(|(_, s)| s.drop_rank)
            .map(|(i, _)| i);
        match drop {
            Some(i) => segments.remove(i),
            None => break,
        };
    }
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
    if form.focus == HomeField::Source {
        // Source-row focus merges the cycle key and the strip-digit jump into
        // one hint — both switch source, so advertising them apart repeats the
        // word "source" down the bar.
        segments.push(HINT_SOURCE_SWITCH);
        return segments;
    }
    match form.focus {
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
        // `s` cycles the focused pane's sort (collection list / missing-set
        // preview) — the only way to find it otherwise is reading the source.
        let mut segments = if update.preview_focused() {
            // Dynamic hints based on focused row state: show only the relevant
            // group (mark OR restore, never both).
            let is_marked = update.preview_focused_is_marked();
            let mut s = vec![HINT_SCROLL];
            if is_marked {
                s.push(HINT_RESTORE);
            } else {
                s.push(HINT_MARK_INSTALLED);
            }
            s.push(HINT_SORT);
            s.push(HINT_FOCUS_LIST);
            s
        } else {
            vec![
                HINT_SCROLL,
                HINT_ENTER_TOGGLE,
                HINT_SELECT_ALL_NONE,
                HINT_SORT,
                HINT_FOCUS_PREVIEW,
            ]
        };
        if can_recheck {
            segments.push(HINT_RECHECK);
        }
        // `m` backfills the next osu-batch page of missing-set titles (mirrors the
        // flat browse's `m more`), advertised only while pages remain.
        if update.has_more_enrichment() {
            segments.push(HINT_MORE);
        }
        // The browse ascends on esc rather than quitting; that back step is left
        // unadvertised (esc-to-go-back is universal), so no trailing key.
        return (segments, None);
    }

    let mut segments = vec![HINT_MOVE];
    if form.focus == HomeField::Source {
        // Same merged cycle+jump hint as the other two source forms.
        segments.push(HINT_SOURCE_SWITCH);
    } else {
        match form.focus {
            HomeField::UpdateScan => segments.push(HINT_SCAN),
            HomeField::UpdateBrowse => segments.push(HINT_ENTER_OPEN),
            HomeField::UpdateOsuPath => segments.push(HINT_EDIT),
            HomeField::Download => segments.push(HINT_ENTER_DOWNLOAD),
            _ => {}
        }
        // The strip digits switch source from the update form too — omitted
        // before, which read as if they only worked on the other two forms.
        segments.push(HINT_SOURCE_JUMP);
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
    let mut spans = vec![ALERT_WARN.fg(warning())];
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
        spinner_str(tick).fg(accent()).bold(),
        msg.text.trim_start().to_string().fg(text_dim()),
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
        if let Some((left, right)) = trimmed.split_once(" / ") {
            let push_key_label = |s: &str, spans: &mut Vec<Span<'static>>| {
                let mut sp = s.splitn(2, ' ');
                let k = sp.next().unwrap_or("");
                let l = sp.next().unwrap_or("");
                spans.push(Span::styled(k.to_string(), key_style));
                if !l.is_empty() {
                    spans.push(Span::styled(format!(" {l}"), label_style));
                }
            };
            push_key_label(left, &mut spans);
            spans.push(Span::styled(" / ", label_style));
            push_key_label(right, &mut spans);
        } else {
            let mut simple = trimmed.splitn(2, ' ');
            let key = simple.next().unwrap_or("");
            let label = simple.next().unwrap_or("");
            spans.push(Span::styled(key.to_string(), key_style));
            if !label.is_empty() {
                spans.push(Span::styled(format!(" {label}"), label_style));
            }
        }
    }

    Line::from(spans)
}

#[cfg(test)]
#[path = "../../tests/unit/tui_footer.rs"]
mod tests;
