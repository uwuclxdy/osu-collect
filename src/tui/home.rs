use crate::app::{GetMapsSource, HomeField, HomeTab, ResolveState};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets;
use super::{accent, danger, focused_label, line, success, text_dim, text_faint, warning};
use crate::utils::pretty_path;
use std::path::Path;

const PANEL_TITLE: &str = " HOME ";

const SECTION_COLLECTION: &str = "collection";
const SECTION_MIRRORS: &str = "mirrors";
const SECTION_DOWNLOAD: &str = "download";
/// Sentinel for a field that belongs to no section (the download button);
/// never equals a rendered header label, so no title lights up.
const SECTION_NONE: &str = "";

const LABEL_OVERWRITE: &str = "overwrite existing";
const LABEL_VIDEO: &str = "video";

const LABEL_START_DOWNLOAD: &str = "start download";

/// Focus hint under the mirrors summary: it is read-only here, so `enter` hands
/// off to the Config tab, which owns all mirror editing.
const HELP_MIRRORS_SUMMARY: &str = "enter to edit mirrors in the config tab";

/// Positions the terminal caret (via [`ratatui::Frame::set_cursor_position`])
/// when a text field is focused in edit mode; otherwise leaves it hidden.
///
/// System-wide banners are rendered by [`super::draw`] above the body area, so
/// this receives the already-reduced content area.
pub fn render(frame: &mut Frame, area: Rect, form: &HomeTab, editing: bool) {
    if area.height < super::COMPACT_HEIGHT {
        render_compact(frame, area, form, editing);
        return;
    }
    render_content(frame, area, form, editing);
}

/// Compact render: all focusable fields without section headers, spacers, or help lines.
///
/// Navigation is identical to normal mode — the full `HOME_FIELDS` cycle still applies.
/// Only decorative chrome is stripped to reclaim vertical space.
fn render_compact(frame: &mut Frame, area: Rect, form: &HomeTab, editing: bool) {
    let focus = form.focus;
    let mut items = widgets::FormItems::new(focus);

    items.push_focusable(
        HomeField::Source,
        source_strip_item(form.source, focus == HomeField::Source),
    );

    if form.source != GetMapsSource::Collection {
        items.push(placeholder_body(form.source));
        let (items, focused_index) = items.into_parts();
        widgets::render_scrollable_panel(
            frame,
            area,
            PANEL_TITLE,
            items,
            focused_index,
            true,
            None,
            true,
            true,
            &form.list_offset,
        );
        return;
    }

    items.push_focusable(
        HomeField::Collection,
        widgets::input_item(&form.collection, focus == HomeField::Collection, editing, 0),
    );
    if let Some((state, text)) = &form.collection_resolve {
        items.push(resolve_row(*state, text));
    }

    items.push_focusable(
        HomeField::Mirrors,
        mirror_summary_item(
            form.mirror_count(),
            form.mirror_latency_range(),
            focus == HomeField::Mirrors,
        ),
    );

    items.push_focusable(
        HomeField::Directory,
        widgets::input_item(&form.directory, focus == HomeField::Directory, editing, 0),
    );
    items.push_focusable(
        HomeField::Threads,
        widgets::stepper_item(
            form.threads.label,
            form.resolved_threads(),
            form.default_threads,
            focus == HomeField::Threads,
            0,
        ),
    );
    push_toggle_rows(&mut items, form, focus);

    items.push_focusable(
        HomeField::Download,
        widgets::button_item(
            LABEL_START_DOWNLOAD,
            focus == HomeField::Download,
            can_download(form),
        ),
    );

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
        focus != HomeField::Download,
        cursor_col,
        true,
        true,
        &form.list_offset,
    )
}

/// Whether the form has the minimum inputs a download needs: a collection
/// reference and at least one enabled mirror. Drives the button's enabled state;
/// final validation still happens in `HomeTab::build_request` on activation.
fn can_download(form: &HomeTab) -> bool {
    !form.collection.value.trim().is_empty() && form.mirror_count() > 0
}

/// Tooltip text for the focused download-directory field: the per-collection
/// folder maps will be written to (`<base>/<collection folder>`), home collapsed
/// to `~`. Until a collection resolves the folder name is unknown, so a
/// `<collection>` placeholder stands in for it. A blank field resolves to the
/// default directory.
fn directory_hint(form: &HomeTab) -> String {
    let base = form.resolved_directory();
    // Unknown until the collection resolves; a placeholder stands in for the
    // per-collection folder. Joining via `Path` keeps the separator
    // platform-correct (`\` on Windows) rather than a hardcoded `/`.
    let folder = form
        .resolved_folder_name
        .as_deref()
        .unwrap_or("<collection>");
    format!(
        "downloads to {}",
        pretty_path(Path::new(&base).join(folder))
    )
}

fn render_content(frame: &mut Frame, area: Rect, form: &HomeTab, editing: bool) {
    let focus = form.focus;
    let mut items = widgets::FormItems::new(focus);

    // Source strip is the first focusable row on every source.
    items.push_focusable(
        HomeField::Source,
        source_strip_item(form.source, focus == HomeField::Source),
    );
    items.push(widgets::spacer());

    // Search / update render a placeholder until their real forms land; only the
    // collection source has a functional body today.
    if form.source != GetMapsSource::Collection {
        items.push(placeholder_body(form.source));
        let (items, focused_index) = items.into_parts();
        widgets::render_scrollable_panel(
            frame,
            area,
            PANEL_TITLE,
            items,
            focused_index,
            true,
            None,
            true,
            true,
            &form.list_offset,
        );
        return;
    }

    let active_section = home_section(focus);
    items.push(widgets::section_header(
        SECTION_COLLECTION,
        active_section == SECTION_COLLECTION,
    ));
    items.push_focusable(
        HomeField::Collection,
        widgets::input_item(&form.collection, focus == HomeField::Collection, editing, 0),
    );
    if let Some((state, text)) = &form.collection_resolve {
        items.push(resolve_row(*state, text));
    }
    items.push(widgets::spacer());

    items.push(widgets::section_header(
        SECTION_MIRRORS,
        active_section == SECTION_MIRRORS,
    ));
    let mirrors_focused = focus == HomeField::Mirrors;
    items.push_focusable(
        HomeField::Mirrors,
        mirror_summary_item(
            form.mirror_count(),
            form.mirror_latency_range(),
            mirrors_focused,
        ),
    );
    if mirrors_focused {
        items.push(widgets::help_item(HELP_MIRRORS_SUMMARY));
    }
    items.push(widgets::spacer());

    items.push(widgets::section_header(
        SECTION_DOWNLOAD,
        active_section == SECTION_DOWNLOAD,
    ));
    items.push_focusable(
        HomeField::Directory,
        widgets::input_item(&form.directory, focus == HomeField::Directory, editing, 0),
    );
    // Tooltip: the resolved path maps will be downloaded to (default dir when the
    // field is blank), so the user sees the target before starting.
    if focus == HomeField::Directory {
        items.push(widgets::help_item(directory_hint(form)));
    }
    items.push_focusable(
        HomeField::Threads,
        widgets::stepper_item(
            form.threads.label,
            form.resolved_threads(),
            form.default_threads,
            focus == HomeField::Threads,
            0,
        ),
    );
    push_toggle_rows(&mut items, form, focus);
    items.push(widgets::spacer());

    items.push_focusable(
        HomeField::Download,
        widgets::button_item(
            LABEL_START_DOWNLOAD,
            focus == HomeField::Download,
            can_download(form),
        ),
    );

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
        focus != HomeField::Download,
        cursor_col,
        true,
        true,
        &form.list_offset,
    )
}

/// Pushes the two boolean override toggles (`overwrite existing`, `video`),
/// shared by `render_compact` and `render_content`.
///
/// The slide-toggle glyph already encodes each row's state, so neither row
/// repeats it as text and neither carries a default hint.
fn push_toggle_rows(items: &mut widgets::FormItems<HomeField>, form: &HomeTab, focus: HomeField) {
    items.push_focusable(
        HomeField::AutoOverwrite,
        widgets::row_item(
            LABEL_OVERWRITE,
            None,
            form.auto_overwrite,
            focus == HomeField::AutoOverwrite,
            0,
        ),
    );
    items.push_focusable(
        HomeField::Video,
        widgets::row_item(LABEL_VIDEO, None, form.video, focus == HomeField::Video, 0),
    );
}

/// The collapsed mirrors row on the Get Maps tab: `mirrors  N enabled`. Mirror
/// editing lives on the Config tab, so this row is read-only; its `enter` hands
/// off there (focus hint + `App::open_config_mirrors`). `count` is
/// [`HomeTab::mirror_count`] — enabled built-ins plus valid custom mirrors.
fn mirror_summary_item(
    count: usize,
    latency_range: Option<(u32, u32)>,
    focused: bool,
) -> ListItem<'static> {
    let value = format!("{count} enabled");
    let mut spans = vec![
        widgets::focus_span(focused),
        Span::styled(widgets::label_cell("mirrors", 0), focused_label(focused)),
        Span::styled(value, Style::default().fg(accent())),
    ];
    // Min–max ping over the enabled built-ins that answered with a number; a
    // single value collapses to one readout, none omits the suffix entirely.
    // The fastest end reads SUCCESS, the slowest WARNING, so the range doubles
    // as a relative fast/slow cue; separator and unit stay dim.
    if let Some((min, max)) = latency_range {
        let dim = Style::default().fg(text_dim());
        spans.push(Span::styled("  ·  ", dim));
        spans.push(Span::styled(
            min.to_string(),
            Style::default().fg(success()),
        ));
        if min != max {
            spans.push(Span::styled("–", dim));
            spans.push(Span::styled(
                max.to_string(),
                Style::default().fg(warning()),
            ));
        }
        spans.push(Span::styled("ms", dim));
    }
    ListItem::new(Line::from(spans))
}

/// The section a focused field belongs to, driving the active-section header cue.
///
/// The download button sits below all sections, so it maps to no header
/// (`SECTION_NONE`): focusing it leaves every section title un-underlined.
fn home_section(field: HomeField) -> &'static str {
    use HomeField::*;
    match field {
        // The source strip sits above every section header, so focusing it
        // lights none of them.
        Source => SECTION_NONE,
        Collection => SECTION_COLLECTION,
        Mirrors => SECTION_MIRRORS,
        Threads | AutoOverwrite | Video | Directory => SECTION_DOWNLOAD,
        Download => SECTION_NONE,
    }
}

/// The source strip: `‹active›  other  other`, the active source bracketed in
/// accent, the rest dim. The first focusable row on the Get Maps tab; `←`/`→`
/// cycle it while focused.
fn source_strip_item(active: GetMapsSource, focused: bool) -> ListItem<'static> {
    let dim = Style::default().fg(text_dim());
    let mut spans = vec![widgets::focus_span(focused)];
    for (i, source) in GetMapsSource::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", dim));
        }
        if *source == active {
            spans.push(Span::styled(
                format!("‹{}›", source.label()),
                Style::default().fg(accent()).bold(),
            ));
        } else {
            spans.push(Span::styled(source.label(), dim));
        }
    }
    ListItem::new(Line::from(spans))
}

/// Placeholder body for a not-yet-wired source. Search lands in a later update;
/// the update source folds the existing Updates tab in, so it points there for
/// now.
fn placeholder_body(source: GetMapsSource) -> ListItem<'static> {
    let msg = match source {
        GetMapsSource::Search => "search lands in a later update",
        GetMapsSource::Update => "use the updates tab for now",
        // The collection source renders its real form, never this placeholder.
        GetMapsSource::Collection => "",
    };
    ListItem::new(Line::from(Span::styled(
        msg.to_string(),
        Style::default().fg(text_faint()),
    )))
}

const RESOLVE_PREFIX: &str = "  └ ";
const RESOLVE_ARROW: &str = "→ ";

fn resolve_row(state: ResolveState, text: &str) -> ListItem<'static> {
    let (arrow_color, text_color) = match state {
        ResolveState::Loading => (text_dim(), text_faint()),
        ResolveState::Success => (success(), text_faint()),
        ResolveState::Error => (danger(), danger()),
    };
    // Tooltip leader matches its line's color: DANGER on error, LINE otherwise.
    let leader_color = match state {
        ResolveState::Error => danger(),
        _ => line(),
    };
    ListItem::new(Line::from(vec![
        Span::styled(RESOLVE_PREFIX, Style::default().fg(leader_color)),
        Span::styled(RESOLVE_ARROW, Style::default().fg(arrow_color)),
        Span::styled(text.to_string(), Style::default().fg(text_color)),
    ]))
}
