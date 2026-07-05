use crate::app::{
    LibraryState, UpdateField, UpdateSource,
    update_source::{
        BeatmapDisplayItem, BeatmapSort, CollectionEntry, CollectionSort, MissingBeatmapset,
        ScanStatus,
    },
};
use crate::osu_db::OsuClient;
use crate::utils::pretty_path;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets::{self, Metric};
use super::{accent, focused_label, text, text_dim, text_faint};

const PANEL_TITLE: &str = " UPDATES ";

const SECTION_SOURCE: &str = "source";
const SECTION_COLLECTIONS: &str = "collections";
const SECTION_MISSING: &str = "missing beatmaps";
/// Sentinel for a field that belongs to no section (the download button);
/// never equals a rendered header label, so no title lights up.
const SECTION_NONE: &str = "";

const LABEL_COLLECTIONS: &str = "collections";
const LABEL_AVAILABLE: &str = "missing";
const LABEL_DOWNLOAD_SELECTED: &str = "download selected";

const DETAIL_LOADED_SUFFIX: &str = "found";
const DETAIL_MISSING_SUFFIX: &str = "missing";
const DETAIL_LOADING: &str = "loading";
const DETAIL_LOCAL: &str = "local";

const METRIC_KNOWN_BAD: &str = "known bad";

const HELP_DOWNLOAD_SETTINGS: &str = "uses download settings from home tab";

/// Focus hint for the osu! path field: names the database file the *active*
/// client actually reads, so the requirement matches what's selected instead of
/// listing both.
fn osu_path_help(client: OsuClient) -> &'static str {
    match client {
        OsuClient::Stable => "osu! install path (must contain osu!.db)",
        OsuClient::Lazer => "osu!lazer install path (must contain client.realm)",
    }
}

const TAG_PREVIOUSLY_DELETED: &str = "previously deleted";

const COUNT_SUFFIX_MAPS: &str = "maps";
const SUFFIX_SELECTED: &str = "selected";

const DIFF_PREFIX_REMOVED: &str = "-";
const DIFF_SUFFIX_REMOVED: &str = "removed";

pub fn render(
    frame: &mut Frame,
    area: Rect,
    form: &UpdateSource,
    library: &LibraryState,
    editing: bool,
) {
    if area.height < super::COMPACT_HEIGHT {
        render_compact(frame, area, form);
        return;
    }

    let block = widgets::panel_block(PANEL_TITLE, true, true);
    let inner = block.inner(area);

    let (items, focused_index) = build_items(form, library, editing);
    let total = items.len();
    frame.render_widget(block, area);

    // The download button styles its own focus (KEEP per pending exception), so
    // it is excluded from the row highlight — but it must still scroll into view.
    let highlight = form.selection.focus != UpdateField::Download;
    let start = widgets::render_list(
        frame,
        inner,
        items,
        Some(focused_index),
        highlight,
        &form.list_offset,
    );
    let end = (start + inner.height as usize).min(total);

    // Caret only when the osu! path field is the focused, editable row AND in
    // edit mode (no caret on a selected-not-editing field).
    let cursor_col = (editing && form.osu_path_editable())
        .then(|| widgets::input_cursor_col(&library.osu_path, 0));
    widgets::set_panel_cursor(frame, inner, focused_index, start, end, cursor_col);
}

/// Compact render: collection list with `[selected] name (+N -M)`.
///
/// Inline beatmap list, sort label, and help text are hidden.
fn render_compact(frame: &mut Frame, area: Rect, form: &UpdateSource) {
    let block = widgets::panel_block(PANEL_TITLE, true, true);
    let inner = block.inner(area);

    let selected_idx = form.selection.collections_state.unwrap_or(0);
    let items: Vec<ListItem<'static>> = form
        .selection
        .local_collections
        .iter()
        .enumerate()
        .map(|(i, collection)| {
            let is_sel = i == selected_idx && form.selection.in_collection_list;
            let counts = collection
                .collection_id
                .map(|cid| count_selected(&form.selection.cached_missing_sets, cid));
            collection_item(collection, is_sel, counts)
        })
        .collect();

    let focused_index = if form.selection.in_collection_list {
        selected_idx
    } else {
        0
    };
    frame.render_widget(block, area);

    // Highlight the cursor row only while focus rests in the collection list;
    // scroll to it (and the highlight) only then, otherwise no cursor.
    let focused = form.selection.in_collection_list.then_some(focused_index);
    widgets::render_list(
        frame,
        inner,
        items,
        focused,
        form.selection.in_collection_list,
        &form.list_offset,
    );
}

fn build_items(
    form: &UpdateSource,
    library: &LibraryState,
    editing: bool,
) -> (Vec<ListItem<'static>>, usize) {
    let mut items: Vec<ListItem<'static>> = Vec::new();
    let mut focused_index = 0usize;
    let focus = form.selection.focus;
    let in_list = form.selection.in_collection_list || form.selection.in_beatmap_list;
    let active_section = updates_section(focus);

    items.push(widgets::section_header(
        SECTION_SOURCE,
        active_section == SECTION_SOURCE,
    ));
    if focus == UpdateField::OsuPath && !in_list {
        focused_index = items.len();
    }
    items.push(osu_path_item(form, library, editing));
    if focus == UpdateField::OsuPath && !in_list {
        items.push(widgets::help_item(osu_path_help(library.client_type)));
    }
    items.push(widgets::spacer());

    items.push(widgets::section_header(
        SECTION_COLLECTIONS,
        active_section == SECTION_COLLECTIONS,
    ));
    if focus == UpdateField::Collections && !in_list {
        focused_index = items.len();
    }
    items.push(collections_header(form));
    if form.selection.in_collection_list {
        let selected_idx = form.selection.collections_state.unwrap_or(0);
        for (i, collection) in form.selection.local_collections.iter().enumerate() {
            let is_sel = i == selected_idx;
            if is_sel && focus == UpdateField::Collections {
                focused_index = items.len();
            }
            let counts = collection
                .collection_id
                .map(|cid| count_selected(&form.selection.cached_missing_sets, cid));
            items.push(collection_item(collection, is_sel, counts));
        }
    }
    items.push(widgets::spacer());

    items.push(widgets::section_header(
        SECTION_MISSING,
        active_section == SECTION_MISSING,
    ));
    if focus == UpdateField::BeatmapList && !in_list {
        focused_index = items.len();
    }
    items.push(beatmaps_header(form));
    if form.selection.in_beatmap_list {
        let selected_idx = form.selection.beatmaps_state.unwrap_or(0);
        for (i, item) in form.selection.display_items.iter().enumerate() {
            let is_sel = i == selected_idx;
            if is_sel && focus == UpdateField::BeatmapList {
                focused_index = items.len();
            }
            items.push(display_item(item, is_sel, form));
        }
    }
    items.push(widgets::spacer());

    let selected = form.selected_beatmap_count();
    let download_label = if selected > 0 {
        format!("{LABEL_DOWNLOAD_SELECTED} ({selected})")
    } else {
        LABEL_DOWNLOAD_SELECTED.to_string()
    };
    let download_focused = focus == UpdateField::Download && !in_list;
    if download_focused {
        focused_index = items.len();
    }
    items.push(widgets::button_item(
        &download_label,
        download_focused,
        selected > 0,
    ));
    if download_focused && selected > 0 {
        items.push(widgets::help_item(HELP_DOWNLOAD_SETTINGS));
    }
    items.push(widgets::spacer());

    // Summary stats sit below the button, one metric per line.
    for metric in summary_metrics(form) {
        items.push(widgets::summary_item(std::slice::from_ref(&metric)));
    }

    (items, focused_index)
}

/// The section a focused field belongs to, driving the active-section header cue.
/// List rows keep their parent field's focus, so the same map covers both the
/// settled and in-list states.
///
/// The download button sits below all sections, so it maps to no header
/// (`SECTION_NONE`): focusing it leaves every section title un-underlined.
fn updates_section(field: UpdateField) -> &'static str {
    use UpdateField::*;
    match field {
        OsuPath => SECTION_SOURCE,
        Collections => SECTION_COLLECTIONS,
        BeatmapList => SECTION_MISSING,
        Download => SECTION_NONE,
    }
}

/// The summary metrics, each rendered on its own line by the caller. Only the
/// `known bad` count surfaces, and only once a scan has flagged maps no mirror
/// can serve.
fn summary_metrics(form: &UpdateSource) -> Vec<Metric<'static>> {
    let mut metrics = Vec::new();
    if form.scan.failed_beatmapset_count > 0 {
        metrics.push(Metric::muted(
            METRIC_KNOWN_BAD,
            form.scan.failed_beatmapset_count.to_string(),
        ));
    }
    metrics
}

fn display_item(
    item: &BeatmapDisplayItem,
    is_scroll_pos: bool,
    form: &UpdateSource,
) -> ListItem<'static> {
    match item {
        BeatmapDisplayItem::CollectionHeader { collection_id } => {
            let name = form
                .selection
                .cached_missing_sets
                .iter()
                .find(|beatmap| beatmap.collection_id == *collection_id)
                .map(|beatmap| beatmap.collection_name.clone())
                .unwrap_or_default();

            let visible_cache_indices: Vec<usize> = form
                .selection
                .visible_missing
                .iter()
                .copied()
                .filter(|&cache_idx| {
                    form.selection
                        .cached_missing_sets
                        .get(cache_idx)
                        .map(|beatmap| beatmap.collection_id == *collection_id)
                        .unwrap_or(false)
                })
                .collect();

            let all_selected = !visible_cache_indices.is_empty()
                && visible_cache_indices.iter().all(|&cache_idx| {
                    form.selection
                        .cached_missing_sets
                        .get(cache_idx)
                        .map(|beatmap| beatmap.selected)
                        .unwrap_or(false)
                });
            // Group header is hierarchy, not an active-item name marker —
            // neutral TEXT, no orange anchor (which needs the paired `●` dot).
            let mut spans = vec![widgets::focus_span(is_scroll_pos)];
            spans.extend(widgets::checkbox_spans(all_selected));
            spans.push(Span::styled(
                format!(" #{collection_id}"),
                Style::default().fg(text_faint()),
            ));
            spans.push(Span::styled(
                format!("  {name}"),
                Style::default().fg(text()),
            ));
            ListItem::new(Line::from(spans))
        }
        BeatmapDisplayItem::Beatmap { cache_index } => form
            .selection
            .cached_missing_sets
            .get(*cache_index)
            .map(|beatmap| beatmap_item(beatmap, is_scroll_pos))
            .unwrap_or_else(|| ListItem::new(Line::from(""))),
    }
}

fn osu_path_item(form: &UpdateSource, library: &LibraryState, editing: bool) -> ListItem<'static> {
    let focused = form.selection.focus == UpdateField::OsuPath
        && !form.selection.in_collection_list
        && !form.selection.in_beatmap_list;
    let field = &library.osu_path;

    // When focused and the user is actively typing, show the raw value so
    // they can see and edit exactly what they typed. When not focused,
    // collapse the home prefix to `~` for readability.
    let display_value = if focused || field.value.is_empty() {
        field.value.clone()
    } else {
        pretty_path(&field.value).into_owned()
    };

    let value = if field.value.is_empty() {
        Span::styled(
            pretty_path(&field.placeholder).into_owned(),
            Style::default().fg(text_faint()),
        )
    } else if library.is_path_auto_detected() {
        Span::styled(display_value, Style::default().fg(text_faint()))
    } else {
        Span::styled(display_value, Style::default().fg(accent()))
    };

    ListItem::new(Line::from(vec![
        widgets::input_focus_span(focused, editing),
        Span::styled(
            widgets::label_cell(&field.label.to_lowercase(), 0),
            focused_label(focused),
        ),
        value,
    ]))
}

fn collections_header(form: &UpdateSource) -> ListItem<'static> {
    let focused = form.selection.focus == UpdateField::Collections
        && !form.selection.in_collection_list
        && !form.selection.in_beatmap_list;
    let sort = form.selection.collection_sort;
    let detail = if sort == CollectionSort::Default {
        format!(
            "{} {DETAIL_LOADED_SUFFIX}",
            form.selection.local_collections.len()
        )
    } else {
        format!(
            "{} {DETAIL_LOADED_SUFFIX}  · {}",
            form.selection.local_collections.len(),
            sort.label(),
        )
    };
    widgets::disclosure_row(
        LABEL_COLLECTIONS,
        detail,
        form.selection.in_collection_list,
        focused,
        !form.selection.local_collections.is_empty(),
        0,
    )
}

fn collection_item(
    collection: &CollectionEntry,
    is_scroll_pos: bool,
    missing_counts: Option<(usize, usize)>,
) -> ListItem<'static> {
    let id = collection
        .collection_id
        .map(|id| format!("#{id}"))
        .unwrap_or_else(|| DETAIL_LOCAL.to_string());
    // Selected row: only the collection name promotes to TEXT + bold; the id,
    // map count, and diff badges keep their own faint metadata color.
    let name_style = focused_label(is_scroll_pos);

    let mut spans = vec![widgets::focus_span(is_scroll_pos)];
    spans.extend(widgets::checkbox_spans(collection.selected));
    spans.push(Span::styled(
        format!(" {id}"),
        Style::default().fg(text_faint()),
    ));
    spans.push(Span::styled(format!("  {}", collection.name), name_style));
    spans.push(Span::styled(
        format!("  {} {COUNT_SUFFIX_MAPS}", collection.beatmap_count),
        Style::default().fg(text_faint()),
    ));

    if let Some((n, total)) = missing_counts {
        spans.push(Span::styled(
            format!("  [{n}/{total} {SUFFIX_SELECTED}]"),
            Style::default().fg(text_faint()),
        ));
    }

    let removed_count = collection.removed_count;
    if removed_count > 0 {
        spans.push(Span::styled(
            format!("  {DIFF_PREFIX_REMOVED}{removed_count} {DIFF_SUFFIX_REMOVED}"),
            Style::default().fg(text_faint()),
        ));
    }

    ListItem::new(Line::from(spans))
}

/// Returns `(n_selected, total)` for `cached` entries belonging to `collection_id`.
pub(super) fn count_selected(cached: &[MissingBeatmapset], collection_id: u64) -> (usize, usize) {
    let mut total = 0usize;
    let mut selected = 0usize;
    for beatmap in cached {
        if beatmap.collection_id as u64 == collection_id {
            total += 1;
            if beatmap.selected {
                selected += 1;
            }
        }
    }
    (selected, total)
}

fn beatmaps_header(form: &UpdateSource) -> ListItem<'static> {
    let focused = form.selection.focus == UpdateField::BeatmapList
        && !form.selection.in_collection_list
        && !form.selection.in_beatmap_list;
    let sort = form.selection.beatmap_sort;
    let detail = if is_scanning(form) {
        DETAIL_LOADING.to_string()
    } else if sort == BeatmapSort::Default {
        format!("{} {DETAIL_MISSING_SUFFIX}", form.total_missing_count())
    } else {
        format!(
            "{} {DETAIL_MISSING_SUFFIX}  · {}",
            form.total_missing_count(),
            sort.label(),
        )
    };

    widgets::disclosure_row(
        LABEL_AVAILABLE,
        detail,
        form.selection.in_beatmap_list,
        focused,
        form.total_missing_count() > 0,
        0,
    )
}

fn beatmap_item(beatmap: &MissingBeatmapset, is_scroll_pos: bool) -> ListItem<'static> {
    let mut spans = vec![widgets::focus_span(is_scroll_pos)];
    spans.extend(widgets::checkbox_spans(beatmap.selected));
    spans.push(Span::styled(
        format!(" #{}", beatmap.id),
        Style::default().fg(text_dim()),
    ));

    if beatmap.previously_deleted {
        // Informational metadata tag — not a sanctioned orange anchor.
        spans.push(Span::styled(
            format!("  {TAG_PREVIOUSLY_DELETED}"),
            Style::default().fg(text_faint()),
        ));
    }

    ListItem::new(Line::from(spans))
}

fn is_scanning(form: &UpdateSource) -> bool {
    matches!(
        form.scan.scan_status,
        ScanStatus::ReadingDatabase
            | ScanStatus::FetchingCollection
            | ScanStatus::CheckingFailedMaps
    )
}

#[cfg(test)]
#[path = "../../tests/unit/tui_updates.rs"]
mod tests;
