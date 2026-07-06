//! Render for the Get Maps "update" source: a small form (osu! path + scan CTA)
//! that descends into a two-pane master-detail browse (collections list +
//! missing-set preview). The source strip is drawn by the Home view; the FORM
//! rows are pushed into the same Home panel, and the BROWSE takes over the whole
//! body via [`super::master_detail`].

use crate::app::{
    HomeField, LibraryState, ScanCta, UpdateSource,
    update_source::{MissingBeatmapset, ScanStatus},
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

    // A completed scan with pending updates gets a one-line summary above the CTA.
    if form.scan.scan_status == ScanStatus::Ready && form.total_new_count() > 0 {
        items.push(new_summary_row(
            form.total_new_count(),
            form.collections_with_new_count(),
        ));
    }

    // A running scan is indeterminate work, so the CTA animates: swap the static
    // label for an inline braille spinner and hold the button inert until done.
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

    // The download button dispatches every missing set of the checked
    // collections; disabled until a scan selects at least one. Grouped tight
    // under the scan CTA — the collection source stacks its two action buttons
    // the same way — so the actions read as one block instead of split by a
    // lone spacer.
    let selected = form.selected_new_count();
    let (download_label, download_enabled) = widgets::download_button_label(selected);
    items.push_focusable(
        HomeField::Download,
        widgets::button_item(
            &download_label,
            focus == HomeField::Download,
            download_enabled,
        ),
    );

    for metric in summary_metrics(form) {
        items.push(widgets::summary_item(std::slice::from_ref(&metric)));
    }
}

/// Render the two-pane browse over the whole body area.
pub fn render_browse(frame: &mut Frame, area: Rect, form: &UpdateSource) {
    let status = Some(new_summary_line(
        form.total_new_count(),
        form.collections_with_new_count(),
    ));

    // Caret + label promotion render only while the list pane owns focus.
    let list_focused = !form.preview_focused();
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
                entry.removed_count,
                list_focused && list_selected == Some(i),
            )
        })
        .collect();

    let preview_indices = form.preview_missing_indices();
    let preview_items: Vec<ListItem<'static>> = preview_indices
        .iter()
        .filter_map(|&idx| form.selection.cached_missing_sets.get(idx))
        .map(preview_row)
        .collect();
    let preview_selected = form
        .preview_focused()
        .then(|| {
            form.selection
                .preview_cursor
                .map(|cursor| cursor.min(preview_items.len().saturating_sub(1)))
        })
        .flatten();

    let view = MasterDetail {
        status,
        list_title: LIST_TITLE,
        list_items,
        list_selected,
        list_offset: &form.list_offset,
        preview_title: PREVIEW_TITLE,
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

/// One collection row: caret, checkbox, name, "N new", and "· -M removed" when
/// present. Only the cursor row's name promotes to TEXT + bold.
fn collection_row(
    selected: bool,
    name: &str,
    new: usize,
    removed: usize,
    is_cursor: bool,
) -> ListItem<'static> {
    let mut spans = vec![widgets::focus_span(is_cursor)];
    spans.extend(widgets::checkbox_spans(selected));
    spans.push(Span::styled(format!(" {name}"), focused_label(is_cursor)));
    spans.push(Span::styled(
        format!("  {new} new"),
        Style::default().fg(if new > 0 { success() } else { text_faint() }),
    ));
    if removed > 0 {
        spans.push(Span::styled(
            format!("  · -{removed} removed"),
            Style::default().fg(text_faint()),
        ));
    }
    ListItem::new(Line::from(spans))
}

/// One read-only preview row: the missing set id, plus a marker for a set the
/// user previously deleted from the collection.
fn preview_row(set: &MissingBeatmapset) -> ListItem<'static> {
    let mut spans = vec![Span::styled(
        format!("#{}", set.id),
        Style::default().fg(text_dim()),
    )];
    if set.previously_deleted {
        spans.push(Span::styled(
            format!("  {TAG_PREVIOUSLY_DELETED}"),
            Style::default().fg(text_faint()),
        ));
    }
    ListItem::new(Line::from(spans))
}

/// The "N new across M collections" status line (browse header + form summary).
fn new_summary_line(new: usize, collections: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(new.to_string(), Style::default().fg(accent()).bold()),
        Span::styled(
            format!(
                " new across {collections} {}",
                if collections == 1 {
                    "collection"
                } else {
                    "collections"
                }
            ),
            Style::default().fg(text_dim()),
        ),
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
