use crate::app::{GetMapsSource, HomeField, HomeTab, LibraryState, ResolveState};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets;
use super::{
    accent, danger, focused_label, line, search_source, set_browse, success, text_dim, text_faint,
    update_source, warning,
};
use crate::utils::pretty_path;
use std::path::Path;

const PANEL_TITLE: &str = " HOME ";

const SECTION_COLLECTION: &str = "collection";
const SECTION_MIRRORS: &str = "mirrors";
const SECTION_DOWNLOAD: &str = "download";
/// Sentinel for a field that belongs to no section (the download button);
/// never equals a rendered header label, so no title lights up.
const SECTION_NONE: &str = "";

const LABEL_SOURCE: &str = "source";
const LABEL_OVERWRITE: &str = "overwrite existing";
const LABEL_VIDEO: &str = "video";

const LABEL_DOWNLOAD_ALL: &str = "download all";

/// Left title of the collection browse&pick browse (its left pane).
const COLLECTION_BROWSE_TITLE: &str = " COLLECTION ";

/// Focus hint under the mirrors summary: it is read-only here, so `↵` hands
/// off to the Config tab, which owns all mirror editing.
const HELP_MIRRORS_SUMMARY: &str = "↵ to edit mirrors in the config tab";

/// Positions the terminal caret (via [`ratatui::Frame::set_cursor_position`])
/// when a text field is focused in edit mode; otherwise leaves it hidden.
///
/// System-wide banners are rendered by [`super::draw`] above the body area, so
/// this receives the already-reduced content area.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    form: &HomeTab,
    library: &LibraryState,
    editing: bool,
    tick: u64,
) {
    if area.height < super::COMPACT_HEIGHT {
        render_compact(frame, area, form, library, editing, tick);
        return;
    }
    render_content(frame, area, form, library, editing, tick);
}

/// Caret column for the update source's osu! path input (its value lives on
/// `library`), or `None` when it isn't the focused, editing row.
fn update_cursor_col(form: &HomeTab, library: &LibraryState, editing: bool) -> Option<u16> {
    (editing && form.focus == HomeField::UpdateOsuPath)
        .then(|| widgets::input_cursor_col(&library.osu_path, 0))
}

/// If the active source is in a flat set browse (search results / collection
/// browse&pick), render it over the whole body and return `true`. The update
/// source's two-level browse is handled separately by its own render.
fn maybe_render_set_browse(frame: &mut Frame, area: Rect, form: &HomeTab) -> bool {
    match form.source {
        GetMapsSource::Search if form.search.browse.is_browsing() => {
            let status = widgets::ratio_line(
                form.search.browse.selected_count(),
                form.search.browse.rows.len(),
            );
            set_browse::render(
                frame,
                area,
                &form.search.browse,
                search_source::BROWSE_LIST_TITLE,
                status,
            );
            true
        }
        GetMapsSource::Collection if form.collection_browse.is_browsing() => {
            let status = widgets::ratio_line(
                form.collection_browse.selected_count(),
                form.collection_browse.rows.len(),
            );
            set_browse::render(
                frame,
                area,
                &form.collection_browse,
                COLLECTION_BROWSE_TITLE,
                status,
            );
            true
        }
        _ => false,
    }
}

/// Caret column for the search query input, or `None` when it isn't the focused,
/// editing row.
fn search_cursor_col(form: &HomeTab, editing: bool) -> Option<u16> {
    (editing && form.focus == HomeField::SearchQuery)
        .then(|| widgets::input_cursor_col(&form.search.query, 0))
}

/// Compact render: all focusable fields without section headers, spacers, or help lines.
///
/// Navigation is identical to normal mode — the full `HOME_FIELDS` cycle still applies.
/// Only decorative chrome is stripped to reclaim vertical space.
fn render_compact(
    frame: &mut Frame,
    area: Rect,
    form: &HomeTab,
    library: &LibraryState,
    editing: bool,
    tick: u64,
) {
    let focus = form.focus;

    // A browse claims the whole body regardless of density.
    if form.source == GetMapsSource::Update && form.update.is_browsing() {
        update_source::render_browse(frame, area, &form.update);
        return;
    }
    if maybe_render_set_browse(frame, area, form) {
        return;
    }

    let mut items = widgets::FormItems::new(focus);

    items.push_focusable(
        HomeField::Source,
        source_row_item(form.source, focus == HomeField::Source),
    );

    match form.source {
        GetMapsSource::Collection => {}
        GetMapsSource::Update => {
            update_source::push_form_rows(&mut items, &form.update, library, focus, editing, tick);
            let cursor_col = update_cursor_col(form, library, editing);
            let (items, focused_index) = items.into_parts();
            widgets::render_scrollable_panel(
                frame,
                area,
                PANEL_TITLE,
                None,
                items,
                focused_index,
                !focus.is_button(),
                cursor_col,
                true,
                true,
                &form.list_offset,
            );
            return;
        }
        GetMapsSource::Search => {
            search_source::push_form_rows(&mut items, &form.search, focus, editing, tick);
            let cursor_col = search_cursor_col(form, editing);
            let (items, focused_index) = items.into_parts();
            widgets::render_scrollable_panel(
                frame,
                area,
                PANEL_TITLE,
                None,
                items,
                focused_index,
                !focus.is_button(),
                cursor_col,
                true,
                true,
                &form.list_offset,
            );
            return;
        }
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

    let (download_label, download_enabled) = collection_download_button(form);
    items.push_focusable(
        HomeField::Download,
        widgets::button_item(
            &download_label,
            focus == HomeField::Download,
            download_enabled,
        ),
    );
    // `view N maps` opens the resolved collection in the checkbox browse.
    let (browse_label, browse_enabled) = collection_browse_button(form);
    items.push_focusable(
        HomeField::CollectionBrowse,
        widgets::button_item(
            &browse_label,
            focus == HomeField::CollectionBrowse,
            browse_enabled,
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
        None,
        items,
        focused_index,
        !focus.is_button(),
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

/// The collection source's download-button label + enabled state. Reads
/// `download all` (the whole resolved collection) until a proper nonempty subset
/// is checked in browse&pick, then flips to `download (N)` (dispatched via
/// the selective path in `dispatch_form_download`).
fn collection_download_button(form: &HomeTab) -> (String, bool) {
    if form.collection_subset_picked() {
        (
            format!("download ({})", form.collection_browse.selected_count()),
            true,
        )
    } else {
        // `download all` (vs a source's bare `download`) names that this
        // dispatches the whole resolved collection, not a picked subset.
        (LABEL_DOWNLOAD_ALL.to_string(), can_download(form))
    }
}

/// The collection source's `view N maps` button — opens the resolved collection
/// in the checkbox browse. Labelled with the set count once resolved (and
/// non-empty); disabled otherwise.
fn collection_browse_button(form: &HomeTab) -> (String, bool) {
    match form.resolved_collection.as_ref() {
        Some((_, ids)) if !ids.is_empty() => (widgets::view_maps_label(ids.len()), true),
        _ => ("view maps".to_string(), false),
    }
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

fn render_content(
    frame: &mut Frame,
    area: Rect,
    form: &HomeTab,
    library: &LibraryState,
    editing: bool,
    tick: u64,
) {
    let focus = form.focus;

    // A browse claims the whole body.
    if form.source == GetMapsSource::Update && form.update.is_browsing() {
        update_source::render_browse(frame, area, &form.update);
        return;
    }
    if maybe_render_set_browse(frame, area, form) {
        return;
    }

    let mut items = widgets::FormItems::new(focus);

    // Source strip is the first focusable row on every source.
    items.push_focusable(
        HomeField::Source,
        source_row_item(form.source, focus == HomeField::Source),
    );
    items.push(widgets::spacer());

    match form.source {
        GetMapsSource::Collection => {}
        GetMapsSource::Update => {
            update_source::push_form_rows(&mut items, &form.update, library, focus, editing, tick);
            let cursor_col = update_cursor_col(form, library, editing);
            let (items, focused_index) = items.into_parts();
            widgets::render_scrollable_panel(
                frame,
                area,
                PANEL_TITLE,
                None,
                items,
                focused_index,
                !focus.is_button(),
                cursor_col,
                true,
                true,
                &form.list_offset,
            );
            return;
        }
        GetMapsSource::Search => {
            search_source::push_form_rows(&mut items, &form.search, focus, editing, tick);
            let cursor_col = search_cursor_col(form, editing);
            let (items, focused_index) = items.into_parts();
            widgets::render_scrollable_panel(
                frame,
                area,
                PANEL_TITLE,
                None,
                items,
                focused_index,
                !focus.is_button(),
                cursor_col,
                true,
                true,
                &form.list_offset,
            );
            return;
        }
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

    let (download_label, download_enabled) = collection_download_button(form);
    items.push_focusable(
        HomeField::Download,
        widgets::button_item(
            &download_label,
            focus == HomeField::Download,
            download_enabled,
        ),
    );
    // `view N maps` opens the resolved collection in the checkbox browse.
    let (browse_label, browse_enabled) = collection_browse_button(form);
    items.push_focusable(
        HomeField::CollectionBrowse,
        widgets::button_item(
            &browse_label,
            focus == HomeField::CollectionBrowse,
            browse_enabled,
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
        None,
        items,
        focused_index,
        !focus.is_button(),
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
        // The download buttons and the update / search source fields render in
        // their own bodies, not the collection sections, so they light no header.
        Download | CollectionBrowse | UpdateOsuPath | UpdateScan | UpdateBrowse => SECTION_NONE,
        SearchQuery | SearchMode | SearchStatus | SearchSort | SearchRun | SearchBrowse => {
            SECTION_NONE
        }
    }
}

/// The source strip: `‹active›  other  other`, the active source bracketed in
/// accent, the rest dim. The first focusable row on the Get Maps tab; `space`/
/// `enter` cycle it (the config-cycle convention); arrows switch tabs.
fn source_row_item(active: GetMapsSource, focused: bool) -> ListItem<'static> {
    let options: Vec<&str> = GetMapsSource::ALL.iter().map(|s| s.label()).collect();
    widgets::cycle_item(LABEL_SOURCE, &options, active.label(), focused, 0)
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
