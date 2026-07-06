//! Render for a [`SetBrowse`]: the reusable flat checkbox browse shared by the
//! search-results surface and collection browse&pick. Builds a
//! [`MasterDetail`](super::master_detail::MasterDetail) from the owned state —
//! a checkbox list of beatmapsets on the left, a read-only detail of the
//! highlighted row on the right, and a status line above. It is a pure
//! selector; the download button lives on the source form, not the browse.
//! Rows with [`BeatmapSetMeta`] render rich (title / artist / mapper); id-only
//! rows (collection browse&pick) render as `#id`.

use crate::app::search_source::{BrowseRow, SetBrowse};
use osu_downloader::search::BeatmapSetMeta;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};

use super::master_detail::{self, MasterDetail, Pane};
use super::widgets;
use super::{accent, focused_label, text, text_dim, text_faint, warning};

const PREVIEW_TITLE: &str = " PREVIEW ";
/// Widest key on the preview card, for column-aligning the static kv rows.
const KV_WIDTH: usize = "favourites".len();

/// Render a set browse over the whole body area. `list_title` names the left
/// pane, `list_meta` renders in the list pane's header border. Pure selector —
/// no download button.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    browse: &SetBrowse,
    list_title: &'static str,
    list_meta: Line<'static>,
) {
    // Caret + label promotion render only while the list pane owns focus (the
    // contract's per-pane cursor rule); a descended preview drops both.
    let list_focused = !browse.preview_focused();
    let cursor = browse.list_cursor();
    let list_items: Vec<ListItem<'static>> = browse
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            list_row(
                row,
                browse.is_selected(row.id),
                list_focused && cursor == Some(i),
            )
        })
        .collect();

    let preview_items = browse
        .highlighted_row()
        .map(preview_rows)
        .unwrap_or_default();

    let view = MasterDetail {
        status: None,
        list_title: list_title.into(),
        list_meta: Some(list_meta),
        list_items,
        list_selected: cursor,
        list_offset: &browse.list_offset,
        preview_title: PREVIEW_TITLE.into(),
        preview_meta: None,
        preview_items,
        // The preview is a read-only detail of the highlighted row, not a
        // selectable list, so no row is marked selected.
        preview_selected: None,
        preview_offset: &browse.preview_offset,
        focused: if browse.preview_focused() {
            Pane::Preview
        } else {
            Pane::List
        },
    };
    master_detail::render(frame, area, &view);
}

/// One list row: checkbox plus the set's compact label. Rich when metadata is
/// present (`artist - title`), else the bare id.
fn list_row(row: &BrowseRow, selected: bool, is_cursor: bool) -> ListItem<'static> {
    // caret → checkbox → label (the contract's checkbox-row order); only the
    // cursor row's label promotes to TEXT + bold.
    let mut spans = vec![widgets::focus_span(is_cursor)];
    spans.extend(widgets::checkbox_spans(selected));
    let label = match &row.meta {
        Some(meta) => format!(" {} - {}", meta.artist, meta.title),
        None => format!(" #{}", row.id),
    };
    spans.push(Span::styled(label, focused_label(is_cursor)));
    ListItem::new(Line::from(spans))
}

/// The read-only detail of the highlighted set. Rich metadata when present, else
/// an id-only line with a "no preview" note (collection browse&pick has no
/// per-set metadata to show).
fn preview_rows(row: &BrowseRow) -> Vec<ListItem<'static>> {
    match &row.meta {
        Some(meta) => meta_preview(meta),
        None => vec![
            ListItem::new(Line::from(Span::styled(
                format!("#{}", row.id),
                Style::default().fg(text()),
            ))),
            ListItem::new(Line::from(Span::styled(
                "no preview · osu!collector exposes no per-set metadata",
                Style::default().fg(text_faint()),
            ))),
        ],
    }
}

fn meta_preview(meta: &BeatmapSetMeta) -> Vec<ListItem<'static>> {
    let mut rows = vec![
        ListItem::new(Line::from(Span::styled(
            meta.title.clone(),
            Style::default().fg(accent()).bold(),
        ))),
        ListItem::new(Line::from(Span::styled(
            meta.artist.clone(),
            Style::default().fg(text()),
        ))),
        kv_row("mapper", meta.creator.clone()),
        kv_row("status", meta.status.clone()),
        kv_row("favourites", group_thousands(meta.favourite_count as u64)),
        kv_row("plays", group_thousands(meta.play_count as u64)),
    ];
    if meta.video || meta.nsfw {
        let mut flags: Vec<Span<'static>> = Vec::new();
        if meta.video {
            flags.push(Span::styled("video", Style::default().fg(text_dim())));
        }
        if meta.nsfw {
            if !flags.is_empty() {
                flags.push(Span::styled("  ·  ", Style::default().fg(text_faint())));
            }
            flags.push(Span::styled("nsfw", Style::default().fg(warning())));
        }
        rows.push(ListItem::new(Line::from(flags)));
    }
    rows
}

/// A static key→value preview row: key in `TEXT_DIM + bold` (the cloudy static-kv
/// treatment), column-aligned to [`KV_WIDTH`], value in `TEXT`.
fn kv_row(key: &str, value: String) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled(
            format!("{key:<width$}  ", width = KV_WIDTH),
            Style::default().fg(text_dim()).bold(),
        ),
        Span::styled(value, Style::default().fg(text())),
    ]))
}

/// `1240` → `"1,240"`. Preview counts are detail-panel figures, so they render at
/// full precision with thousands separators (cloudy numeric formatting).
fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}
