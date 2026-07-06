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
use super::{accent, success, text, text_dim, text_faint, warning};

const PREVIEW_TITLE: &str = "preview";

/// Render a set browse over the whole body area. `list_title` names the left
/// pane, `status` is the line above it. Pure selector — no download button.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    browse: &SetBrowse,
    list_title: &'static str,
    status: Line<'static>,
) {
    let list_items: Vec<ListItem<'static>> = browse
        .rows
        .iter()
        .map(|row| list_row(row, browse.is_selected(row.id)))
        .collect();

    let preview_items = browse
        .highlighted_row()
        .map(preview_rows)
        .unwrap_or_default();

    let view = MasterDetail {
        status: Some(status),
        list_title,
        list_items,
        list_selected: browse.list_cursor(),
        list_offset: &browse.list_offset,
        preview_title: PREVIEW_TITLE,
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
fn list_row(row: &BrowseRow, selected: bool) -> ListItem<'static> {
    let mut spans = widgets::checkbox_spans(selected);
    match &row.meta {
        Some(meta) => {
            spans.push(Span::styled(
                format!(" {} - {}", meta.artist, meta.title),
                Style::default().fg(text()),
            ));
        }
        None => {
            spans.push(Span::styled(
                format!(" #{}", row.id),
                Style::default().fg(text_dim()),
            ));
        }
    }
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
                "no preview — osu!collector exposes no per-set metadata",
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
        ListItem::new(Line::from(vec![
            Span::styled("mapper  ", Style::default().fg(text_faint())),
            Span::styled(meta.creator.clone(), Style::default().fg(text())),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("status  ", Style::default().fg(text_faint())),
            Span::styled(meta.status.clone(), Style::default().fg(text_dim())),
            Span::styled("   ♥ ", Style::default().fg(text_faint())),
            Span::styled(
                meta.favourite_count.to_string(),
                Style::default().fg(success()),
            ),
            Span::styled("   ▶ ", Style::default().fg(text_faint())),
            Span::styled(meta.play_count.to_string(), Style::default().fg(text_dim())),
        ])),
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
