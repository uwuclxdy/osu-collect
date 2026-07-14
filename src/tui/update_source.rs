//! Render for the Get Maps "update" source: a small form (osu! path + scan CTA)
//! that descends into a two-pane master-detail browse (collections list +
//! missing-set preview). The source strip is drawn by the Home view; the FORM
//! rows are pushed into the same Home panel, and the BROWSE takes over the whole
//! body via [`super::master_detail`].

use crate::app::{
    EnrichSink, HomeField, LibraryState, ScanCta, UpdateSource,
    update_source::{MissingBeatmapset, PreviewEntry, ScanStatus},
};
use crate::osu_db::OsuClient;
use crate::utils::pretty_path;
use osu_downloader::search::BeatmapSetMeta;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::ListItem,
};

use std::borrow::Cow;

use super::master_detail::{self, MasterDetail, Pane};
use super::widgets::{self, Metric};
use super::{accent, focused_label, spinner_str, success, text_dim, text_faint};

const LIST_TITLE: &str = " COLLECTIONS ";
const PREVIEW_TITLE: &str = " PREVIEW ";

const METRIC_KNOWN_BAD: &str = "known bad";
const TAG_PREVIOUSLY_DELETED: &str = "previously deleted";

/// Focus hint for the osu! path field: names the database file the *active*
/// client actually reads.
fn osu_path_help(client: OsuClient) -> &'static str {
    match client {
        OsuClient::Stable => "osu! install path (must contain osu!.db)",
        OsuClient::Lazer => "osu!lazer install path (must contain client.realm)",
    }
}

/// Push the update-source FORM rows into the Home panel's item list (called
/// after the source strip): the osu! path input, an optional "N new" summary,
/// then the scan CTA button.
pub fn push_form_rows(
    items: &mut widgets::FormItems<HomeField>,
    form: &UpdateSource,
    library: &LibraryState,
    focus: HomeField,
    editing: bool,
    tick: u64,
) {
    let path_focused = focus == HomeField::UpdateOsuPath;
    items.push_focusable(
        HomeField::UpdateOsuPath,
        osu_path_row(library, path_focused, editing),
    );
    if path_focused {
        items.push(widgets::help_item(osu_path_help(library.client_type)));
    }
    items.push(widgets::spacer());

    // Scan result block — the headline + any caveats — grouped together above
    // the actions and split from them by a spacer, so the figures don't blur
    // into the buttons.
    let mut have_result = false;
    if form.scan.scan_status == ScanStatus::Ready && form.total_new_count() > 0 {
        items.push(new_summary_row(
            form.total_new_count(),
            form.collections_with_new_count(),
        ));
        have_result = true;
    }
    for metric in summary_metrics(form) {
        items.push(widgets::summary_item(std::slice::from_ref(&metric)));
        have_result = true;
    }
    if have_result {
        items.push(widgets::spacer());
    }

    // Action block: the scan CTA, the `view N maps` browse button, then download.
    let busy = form.scan_cta() == ScanCta::Busy;
    let cta_label = if busy {
        format!("{} scanning", spinner_str(tick).trim())
    } else {
        form.scan_cta_label()
    };
    items.push_focusable(
        HomeField::UpdateScan,
        widgets::button_item(&cta_label, focus == HomeField::UpdateScan, !busy),
    );

    // `view N maps` opens the two-pane browse over the scan's missing sets;
    // enabled once a scan actually found something (bare `view maps` while
    // empty so it doesn't read as a zero count).
    let new_count = form.total_new_count();
    items.push_focusable(
        HomeField::UpdateBrowse,
        widgets::view_browse_button(
            new_count,
            focus == HomeField::UpdateBrowse,
            new_count > 0,
            form.is_enriching(),
            tick,
        ),
    );
    // The shared download button + run settings render AFTER this, in the Home
    // view's download section (one section borrowed across all three sources).
}

/// Render the two-pane browse over the whole body area.
pub fn render_browse(frame: &mut Frame, area: Rect, form: &UpdateSource, tick: u64) {
    // Caret + label promotion render only while the list pane owns focus.
    let list_focused = !form.preview_focused();
    // While a batch page is in flight the loading cue joins the preview title's
    // existing `N new · M removed` meta instead of decorating each row.
    let enriching = form.is_enriching();
    let list_selected = form.selection.collections_cursor;
    let list_items: Vec<ListItem<'static>> = form
        .selection
        .local_collections
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let new = entry
                .collection_id
                .map(|id| form.new_count_for(id))
                .unwrap_or(0);
            collection_row(
                entry.selected,
                &entry.name,
                new,
                list_focused && list_selected == Some(i),
            )
        })
        .collect();

    // Selected/total collections rides the COLLECTIONS panel's title-right meta.
    let total = form.selection.local_collections.len();
    let selected = form
        .selection
        .local_collections
        .iter()
        .filter(|c| c.selected)
        .count();
    let list_meta = (total > 0).then(|| widgets::ratio_line(selected, total));

    let preview_entries = form.preview_entries();
    let mut preview_items: Vec<ListItem<'static>> = Vec::with_capacity(preview_entries.len());
    for entry in &preview_entries {
        match entry {
            PreviewEntry::Marked(idx) => {
                if let Some(set) = form.selection.marked_installed.get(*idx) {
                    preview_items.push(preview_row(set, form.set_meta(set.id), true));
                }
            }
            PreviewEntry::Missing(idx) => {
                if let Some(set) = form.selection.cached_missing_sets.get(*idx) {
                    preview_items.push(preview_row(set, form.set_meta(set.id), false));
                }
            }
        }
    }
    // The logical cursor indexes `preview_entries` 1:1; there is no separator
    // row between the marked and missing groups, so no visual shift.
    let preview_selected = form
        .preview_focused()
        .then(|| {
            form.selection
                .preview_cursor
                .map(|cursor| cursor.min(preview_items.len().saturating_sub(1)))
        })
        .flatten();

    // Preview pane is named after the highlighted collection (proper-noun case
    // preserved) with its per-collection new/removed stats in the title-right
    // meta, so the list rows can stay name-only.
    let highlighted = form.highlighted_collection();
    let preview_title: Cow<'static, str> = match highlighted {
        Some(c) => Cow::Owned(format!(" {} ", c.name)),
        None => Cow::Borrowed(PREVIEW_TITLE),
    };
    let preview_meta = highlighted.map(|c| {
        let new = c
            .collection_id
            .map(|id| form.new_count_for(id))
            .unwrap_or(0);
        let base = collection_stats_meta(new, c.removed_count);
        widgets::meta_with_loading_cue(base, enriching, tick)
    });

    let view = MasterDetail {
        status: None,
        list_title: Cow::Borrowed(LIST_TITLE),
        list_meta,
        list_items,
        list_selected,
        list_offset: &form.list_offset,
        preview_title,
        preview_meta,
        preview_items,
        preview_selected,
        preview_offset: &form.preview_offset,
        focused: if form.preview_focused() {
            Pane::Preview
        } else {
            Pane::List
        },
    };
    master_detail::render(frame, area, &view);
}

/// One collection row: caret, checkbox, name. Per-collection stats moved to the
/// preview's title-right meta, so the row stays name-only (no truncation). A
/// no-update collection is **inert** (unselectable, sunk to the bottom): the
/// whole row goes faint with an inert `[ ]`, the caret still lands so the user
/// can inspect it. A collection with updates renders live — the cursor row's
/// name promotes to TEXT + bold, others sit in TEXT_DIM.
fn collection_row(selected: bool, name: &str, new: usize, is_cursor: bool) -> ListItem<'static> {
    let mut spans = vec![widgets::focus_span(is_cursor)];
    if new == 0 {
        let faint = Style::default().fg(text_faint());
        spans.push(Span::styled("[ ]", faint));
        spans.push(Span::styled(format!(" {name}"), faint));
    } else {
        spans.extend(widgets::checkbox_spans(selected));
        spans.push(Span::styled(format!(" {name}"), focused_label(is_cursor)));
    }
    ListItem::new(Line::from(spans))
}

/// The preview panel title-right meta for the highlighted collection: `N new`
/// (SUCCESS when any, else faint) and `· M removed` (faint) when any were
/// removed locally.
fn collection_stats_meta(new: usize, removed: usize) -> Line<'static> {
    let mut spans = vec![format!("{new} new").fg(if new > 0 { success() } else { text_faint() })];
    if removed > 0 {
        spans.push(format!("  ·  {removed} removed").fg(text_faint()));
    }
    Line::from(spans)
}

/// One read-only preview row: the missing set as `artist - title` once its
/// enrichment page lands, else the bare id — plus a marker for a set the user
/// previously deleted from the collection. `marked` tints the row as a
/// manually-installed (reversible) entry.
fn preview_row(
    set: &MissingBeatmapset,
    meta: Option<&BeatmapSetMeta>,
    marked: bool,
) -> ListItem<'static> {
    let label_style = if marked {
        Style::default().fg(success())
    } else {
        Style::default().fg(text_dim())
    };
    let mut spans = widgets::browse_row_label(set.id, meta, label_style);
    if marked {
        spans.push("  ✓ installed".fg(success()));
    } else if set.previously_deleted {
        spans.push(format!("  {TAG_PREVIOUSLY_DELETED}").fg(text_faint()));
    }
    ListItem::new(Line::from(spans))
}

/// The "N new across M collections" status line (browse header + form summary).
fn new_summary_line(new: usize, collections: usize) -> Line<'static> {
    Line::from(vec![
        new.to_string().fg(accent()).bold(),
        format!(
            " new across {collections} {}",
            if collections == 1 {
                "collection"
            } else {
                "collections"
            }
        )
        .fg(text_dim()),
    ])
}

fn new_summary_row(new: usize, collections: usize) -> ListItem<'static> {
    let mut spans = vec![Span::raw("  ")];
    spans.extend(new_summary_line(new, collections).spans);
    ListItem::new(Line::from(spans))
}

fn osu_path_row(library: &LibraryState, focused: bool, editing: bool) -> ListItem<'static> {
    let field = &library.osu_path;

    // Focused/typing: show the raw value so the user edits exactly what they see.
    // Blurred: collapse the home prefix to `~` for readability.
    let display_value = if focused || field.value.is_empty() {
        field.value.clone()
    } else {
        pretty_path(&field.value).into_owned()
    };

    let value = if field.value.is_empty() {
        pretty_path(&field.placeholder)
            .into_owned()
            .fg(text_faint())
    } else if library.is_path_auto_detected() {
        display_value.fg(text_faint())
    } else {
        display_value.fg(accent())
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

/// The form's summary metrics — only the `known bad` count, once a scan has
/// flagged maps no mirror can serve.
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
