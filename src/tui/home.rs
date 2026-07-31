use crate::app::{
    Covers, EnrichSink, GetMapsSource, HomeField, HomeTab, LibraryState, ResolveState,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets;
use super::{
    accent, danger, find_source, focused_label, line, set_browse, success, text_dim, text_faint,
    update_source, warning,
};
use crate::utils::pretty_path;
use std::path::Path;

const PANEL_TITLE: &str = " GET MAPS ";

const SECTION_COLLECTION: &str = "collection";
/// The shared run-settings section: mirrors summary + directory / threads /
/// overwrite / video. Rendered on every source (find / collection / update) —
/// the values live on `HomeTab`, so switching source never changes them.
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
const HELP_MIRRORS_SUMMARY: &str = "[↵] configure";

/// Positions the terminal caret (via [`ratatui::Frame::set_cursor_position`])
/// when a text field is focused in edit mode; otherwise leaves it hidden.
///
/// System-wide banners are rendered by [`super::draw`] above the body area, so
/// this receives the already-reduced content area.
#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    area: Rect,
    form: &HomeTab,
    library: &LibraryState,
    covers: &Covers,
    editing: bool,
    tick: u64,
    supporter: bool,
) {
    // A browse claims the whole body regardless of density.
    if form.source == GetMapsSource::Update && form.update.is_browsing() {
        update_source::render_browse(frame, area, &form.update, tick);
        return;
    }
    if maybe_render_set_browse(frame, area, form, covers, tick) {
        return;
    }
    // Compact (< COMPACT_HEIGHT) drops the section headers, spacers, and per-row
    // help tooltips to reclaim vertical space; navigation is identical.
    let chrome = area.height >= super::COMPACT_HEIGHT;
    render_form(frame, area, form, library, editing, tick, chrome, supporter);
}

/// Caret column for whichever text input is focused in edit mode: the update
/// source's osu! path lives on `library` (width 0); the find query is a boxed
/// search input with its own fixed value column; the other find inputs align to
/// [`find_source::LABEL_WIDTH`]; every other input (collection, directory) is
/// its own width. `None` when the focused row isn't an editing text field.
fn home_cursor_col(form: &HomeTab, library: &LibraryState, editing: bool) -> Option<u16> {
    if !editing {
        return None;
    }
    if form.focus == HomeField::UpdateOsuPath {
        return Some(widgets::input_cursor_col(&library.osu_path, 0));
    }
    if form.focus == HomeField::FindQuery {
        return Some(widgets::search_box_cursor_col(&form.find.query));
    }
    let label_width = if form.focus.is_find_input() {
        find_source::LABEL_WIDTH
    } else {
        0
    };
    form.focused_input()
        .map(|input| widgets::input_cursor_col(input, label_width))
}

/// If the active source is in a flat set browse (search results / collection
/// browse&pick), render it over the whole body and return `true`. The update
/// source's two-level browse is handled separately by its own render.
fn maybe_render_set_browse(
    frame: &mut Frame,
    area: Rect,
    form: &HomeTab,
    covers: &Covers,
    tick: u64,
) -> bool {
    match form.source {
        // One union browse: both backends' results land in `find.browse`, so the
        // list title is the shared ` RESULTS ` regardless of which form ran.
        GetMapsSource::Find if form.find.browse.is_browsing() => {
            let status = widgets::ratio_line(
                form.find.browse.selected_count(),
                form.find.browse.rows.len(),
            );
            set_browse::render(
                frame,
                area,
                &form.find.browse,
                find_source::BROWSE_LIST_TITLE,
                status,
                tick,
                Some(covers),
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
                tick,
                Some(covers),
            );
            true
        }
        _ => false,
    }
}

/// The collection source's download-button label + enabled state. Reads
/// `download all` (the whole resolved collection) until a proper nonempty subset
/// is checked in browse&pick, then flips to `download (N)` (dispatched via
/// the selective path in `dispatch_form_download`). Enabled state comes from
/// [`HomeTab::button_enabled`] so it can't drift from the `s`-jump target.
fn collection_download_button(form: &HomeTab) -> (String, bool) {
    let enabled = form.button_enabled(HomeField::Download);
    if form.collection_subset_picked() {
        (
            format!("download ({})", form.collection_browse.selected_count()),
            enabled,
        )
    } else {
        // `download all` (vs a source's bare `download`) names that this
        // dispatches the whole resolved collection, not a picked subset.
        (LABEL_DOWNLOAD_ALL.to_string(), enabled)
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

/// Renders the Get Maps form: the source strip, the active source's own rows,
/// then the shared download section + the source's download button. `chrome`
/// (off below `COMPACT_HEIGHT`) drops the section headers, spacers, and per-row
/// help tooltips; navigation is identical either way. The browse-claims-body and
/// density checks are handled by [`render`] before this is called.
#[allow(clippy::too_many_arguments)]
fn render_form(
    frame: &mut Frame,
    area: Rect,
    form: &HomeTab,
    library: &LibraryState,
    editing: bool,
    tick: u64,
    chrome: bool,
    supporter: bool,
) {
    let focus = form.focus;
    let primary = form.primary_action_field(supporter);
    let active_section = home_section(focus);
    let content_width = widgets::panel_content_width(area);
    let mut items = widgets::FormItems::new(focus);

    // Source strip is the first focusable row on every source.
    items.push_focusable(
        HomeField::Source,
        source_row_item(form.source, focus == HomeField::Source, content_width),
    );
    if chrome {
        items.push(widgets::spacer());
    }

    // Source-specific rows (find/update carry their own CTAs; collection is just
    // the URL field). The find/update rows already include their `view` buttons.
    match form.source {
        GetMapsSource::Collection => {
            if chrome {
                items.push(widgets::section_header(
                    SECTION_COLLECTION,
                    active_section == SECTION_COLLECTION,
                ));
            }
            items.push_focusable(
                HomeField::Collection,
                widgets::input_item(&form.collection, focus == HomeField::Collection, editing, 0),
            );
            if let Some((state, text)) = &form.collection_resolve {
                items.push(resolve_row(*state, text));
            }
            // `view N maps` sits with the collection field (mirroring find/update,
            // where the browse button follows their run/scan CTA), above the shared
            // download section, set off by a spacer (dropped in compact chrome).
            if chrome {
                items.push(widgets::spacer());
            }
            let resolved_count = form
                .resolved_collection
                .as_ref()
                .map(|(_, ids)| ids.len())
                .unwrap_or(0);
            items.push_focusable(
                HomeField::CollectionBrowse,
                widgets::view_browse_button(
                    resolved_count,
                    focus == HomeField::CollectionBrowse,
                    form.button_enabled(HomeField::CollectionBrowse),
                    form.collection_browse.is_enriching(),
                    tick,
                    widgets::ButtonProminence::primary_if(HomeField::CollectionBrowse == primary),
                ),
            );
        }
        GetMapsSource::Update => update_source::push_form_rows(
            &mut items,
            &form.update,
            library,
            focus,
            editing,
            tick,
            primary,
        ),
        GetMapsSource::Find => find_source::push_form_rows(
            &mut items,
            &form.find,
            focus,
            editing,
            tick,
            primary,
            content_width,
            chrome,
            supporter,
        ),
    }

    if chrome {
        items.push(widgets::spacer());
    }
    push_download_section(&mut items, form, focus, editing, chrome, active_section);
    if chrome {
        items.push(widgets::spacer());
    }
    push_action_buttons(&mut items, form, focus, primary);

    let cursor_col = home_cursor_col(form, library, editing);
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

/// The shared `download` run-settings section (mirrors summary + directory /
/// threads / overwrite / video), rendered on every source. The values live on
/// `HomeTab`, so switching source keeps them; `chrome` gates the header + help
/// tooltips.
fn push_download_section(
    items: &mut widgets::FormItems<HomeField>,
    form: &HomeTab,
    focus: HomeField,
    editing: bool,
    chrome: bool,
    active_section: &str,
) {
    if chrome {
        items.push(widgets::section_header(
            SECTION_DOWNLOAD,
            active_section == SECTION_DOWNLOAD,
        ));
    }
    let mirrors_focused = focus == HomeField::Mirrors;
    items.push_focusable(
        HomeField::Mirrors,
        mirror_summary_item(
            form.mirror_count(),
            form.mirror_latency_range(),
            mirrors_focused,
        ),
    );
    if chrome && mirrors_focused {
        items.push(widgets::help_item_keyed(HELP_MIRRORS_SUMMARY));
    }
    items.push_focusable(
        HomeField::Directory,
        widgets::input_item(&form.directory, focus == HomeField::Directory, editing, 0),
    );
    // Tooltip: the resolved path maps will be downloaded to (default dir when the
    // field is blank), so the user sees the target before starting.
    if chrome && focus == HomeField::Directory {
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
    push_toggle_rows(items, form, focus);
}

/// The shared download button that tails every source's form. The collection
/// source's `view N maps` renders earlier, grouped with its URL field. Every
/// source ends on the shared [`HomeField::Download`] button, labelled per source.
fn push_action_buttons(
    items: &mut widgets::FormItems<HomeField>,
    form: &HomeTab,
    focus: HomeField,
    primary: HomeField,
) {
    let (download_label, download_enabled) = match form.source {
        GetMapsSource::Collection => collection_download_button(form),
        GetMapsSource::Update => widgets::download_button_label(form.update.selected_new_count()),
        // osu-routed results carry a nekoha size backfill, so the button reads
        // `download (N) · ~X`; the nzbasic route (and un-probed sets) sums to 0,
        // which drops the suffix and leaves the plain `download (N)`.
        GetMapsSource::Find => widgets::download_button_label_with_size(
            form.find.browse.selected_count(),
            form.find.checked_known_bytes(),
        ),
    };
    items.push_focusable(
        HomeField::Download,
        widgets::button_item(
            &download_label,
            focus == HomeField::Download,
            download_enabled,
            widgets::ButtonProminence::primary_if(HomeField::Download == primary),
        ),
    );
}

/// Pushes the two boolean override toggles (`overwrite existing`, `video`) that
/// tail the shared download section.
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
        value.fg(accent()),
    ];
    // Min–max ping over the enabled built-ins that answered with a number; a
    // single value collapses to one readout, none omits the suffix entirely.
    // The fastest end reads SUCCESS, the slowest WARNING, so the range doubles
    // as a relative fast/slow cue; separator and unit stay dim.
    if let Some((min, max)) = latency_range {
        let dim = Style::default().fg(text_dim());
        spans.push(Span::styled("  ·  ", dim));
        spans.push(min.to_string().fg(success()));
        if min != max {
            spans.push(Span::styled("–", dim));
            spans.push(max.to_string().fg(warning()));
        }
        spans.push(Span::styled("ms", dim));
    }
    ListItem::new(Line::from(spans))
}

/// The section a focused field belongs to, driving the active-section header cue.
///
/// The download button sits below all sections, so it maps to no header
/// (`SECTION_NONE`): focusing it leaves every section title un-underlined. The
/// mirrors summary now lives inside the shared `download` section, so it lights
/// that header (not a standalone `mirrors` one). The find source pushes its own
/// eyebrows and resolves their active state itself (`find_source::find_section`),
/// so every find field maps to no header here.
fn home_section(field: HomeField) -> &'static str {
    use HomeField::*;
    match field {
        // The source strip sits above every section header, so focusing it
        // lights none of them.
        Source => SECTION_NONE,
        Collection => SECTION_COLLECTION,
        Mirrors | Threads | AutoOverwrite | Video | Directory => SECTION_DOWNLOAD,
        // The download buttons and the source-specific CTA rows render outside
        // the collection/download sections, so they light no header.
        Download | CollectionBrowse | UpdateOsuPath | UpdateScan | UpdateBrowse => SECTION_NONE,
        FindQuery | FindPreset | FindSpecial | FindMode | FindStatus | FindSort | FindAdvanced
        | FindStars | FindAr | FindCs | FindOd | FindHp | FindBpm | FindLength | FindKeys
        | FindFavourites | FindRanked | FindArtist | FindCreator | FindTitle | FindLimit
        | FindRun | FindBrowse | FindExplicit | FindGenre | FindLanguage | FindExtra | FindRank
        | FindPlayed => SECTION_NONE,
    }
}

/// The source strip: `‹active›  other  other`, the active source bracketed in
/// accent, the rest dim. The first focusable row on the Get Maps tab; `space`/
/// `enter` cycle it, a strip digit jumps straight to a source, arrows switch tabs.
fn source_row_item(active: GetMapsSource, focused: bool, width: u16) -> ListItem<'static> {
    let options: Vec<&str> = GetMapsSource::ALL.iter().map(|s| s.label()).collect();
    widgets::cycle_item(LABEL_SOURCE, &options, active.label(), focused, 0, width)
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
        RESOLVE_PREFIX.fg(leader_color),
        RESOLVE_ARROW.fg(arrow_color),
        text.to_string().fg(text_color),
    ]))
}
