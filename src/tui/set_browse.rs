//! Render for a [`SetBrowse`]: the reusable flat checkbox browse shared by the
//! search-results surface and collection browse&pick. Builds a
//! [`MasterDetail`](super::master_detail::MasterDetail) from the owned state —
//! a checkbox list of beatmapsets on the left, a read-only detail of the
//! highlighted row on the right, and a status line above. It is a pure
//! selector; the download button lives on the source form, not the browse.
//! Rows with [`BeatmapSetMeta`] render rich (title / artist / mapper); id-only
//! rows (collection browse&pick) render as `#id`.

use crate::app::EnrichSink;
use crate::app::covers::Covers;
use crate::app::find_source::{BrowseRow, SetBrowse};
use osu_downloader::filter::BeatmapDetails;
use osu_downloader::search::BeatmapSetMeta;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::ListItem,
};

use super::master_detail::{self, MasterDetail, Pane, PreviewCover, PreviewLead};
use super::theme::stars_color;
use super::widgets;
use super::{accent, focused_label, spinner_str, text, text_dim, text_faint, warning};

const PREVIEW_TITLE: &str = " PREVIEW ";
/// Widest key on the preview card, for column-aligning the static kv rows.
const KV_WIDTH: usize = "favourites".len();
/// Cell count of the AR/CS/OD/HP bar meters. Equal to the attributes' max
/// scale (10), so one cell == one attribute unit.
const BAR_WIDTH: usize = 10;

/// Render a set browse over the whole body area. `list_title` names the left
/// pane, `list_meta` renders in the list pane's header border. `covers` supplies
/// the highlighted row's cover (a right-hand image column, square or wide per
/// pane size) when it has loaded; `None` renders text-only. Pure selector — no download button.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    browse: &SetBrowse,
    list_title: &'static str,
    list_meta: Line<'static>,
    tick: u64,
    covers: Option<&Covers>,
) {
    // Caret + label promotion render only while the list pane owns focus (the
    // contract's per-pane cursor rule); a descended preview drops both.
    let list_focused = !browse.preview_focused();
    let cursor = browse.list_cursor();
    // A single browse-wide flag: while a batch page is in flight the loading
    // cue moves into the list pane's title (see `meta_with_loading_cue`) and the
    // preview's id-only detail; list rows themselves stay id-only, no per-row cue.
    let enriching = browse.is_enriching();
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

    // The title rides as the preview's lead (`master_detail` wraps it to two
    // lines beside the cover); the fields fill `preview_items`. An id-only or
    // still-loading row has no lead, its id/loading note filling the items.
    let (preview_lead, preview_items) = match browse.highlighted_row() {
        Some(row) => preview_lead_and_items(row, browse.details_for(row.id), enriching, tick),
        None => (None, Vec::new()),
    };

    // The highlighted row's two cover variants, once loaded: the square column
    // and the wide upgrade. Only pass a cover when at least one variant is ready,
    // so an id-only or unfetched row still reaches the text-only fallthrough.
    let preview_image = covers
        .zip(browse.highlighted_row())
        .and_then(|(covers, row)| {
            let cover = PreviewCover {
                square: covers.square_for(row.id),
                wide: covers.wide_for(row.id),
            };
            (cover.square.is_some() || cover.wide.is_some()).then_some(cover)
        });

    let view = MasterDetail {
        status: None,
        list_title: list_title.into(),
        list_meta: Some(widgets::meta_with_loading_cue(list_meta, enriching, tick)),
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
        preview_image,
        preview_lead,
        focused: if browse.preview_focused() {
            Pane::Preview
        } else {
            Pane::List
        },
    };
    master_detail::render(frame, area, &view);
}

/// One list row: checkbox plus the set's compact label. Rich when metadata is
/// present (`artist - title`), else the bare id. Only the cursor row's label
/// promotes to TEXT + bold.
fn list_row(row: &BrowseRow, selected: bool, is_cursor: bool) -> ListItem<'static> {
    // caret → checkbox → label (the contract's checkbox-row order).
    let mut spans = vec![widgets::focus_span(is_cursor)];
    spans.extend(widgets::checkbox_spans(selected));
    spans.push(Span::raw(" "));
    spans.extend(widgets::browse_row_label(
        row.id,
        row.meta.as_ref(),
        focused_label(is_cursor),
    ));
    ListItem::new(Line::from(spans))
}

/// The read-only preview of the highlighted set, split into `(lead, items)`: the
/// title lead (wrapped to two lines by `master_detail`) over the one-line field
/// rows. An id-only or still-enriching row has no lead — its id line plus a
/// `loading metadata…` spinner, or a genuine "no metadata" note once idle, fills
/// the items.
fn preview_lead_and_items(
    row: &BrowseRow,
    details: Option<&BeatmapDetails>,
    enriching: bool,
    tick: u64,
) -> (Option<PreviewLead>, Vec<ListItem<'static>>) {
    match &row.meta {
        Some(meta) => (
            Some(PreviewLead {
                text: meta.title.clone(),
                style: Style::new().fg(accent()).bold(),
            }),
            meta_field_rows(meta, details),
        ),
        None => {
            let id_line = ListItem::new(Line::from(format!("#{}", row.id).fg(text())));
            let note = if enriching {
                ListItem::new(Line::from(
                    format!("{} loading metadata…", spinner_str(tick).trim()).fg(text_dim()),
                ))
            } else {
                ListItem::new(Line::from("no metadata available".fg(text_faint())))
            };
            (None, vec![id_line, note])
        }
    }
}

/// The preview's field rows (no title — that rides as the lead): artist, the
/// static kv rows, the video/nsfw flags, then the set-level extras (nzbasic
/// route) and the per-difficulty section (both routes). Each is one line, so
/// the cover/text budget is a plain row count.
fn meta_field_rows(
    meta: &BeatmapSetMeta,
    details: Option<&BeatmapDetails>,
) -> Vec<ListItem<'static>> {
    let mut rows = vec![
        ListItem::new(Line::from(meta.artist.clone().fg(text()))),
        kv_row("mapper", meta.creator.clone()),
        kv_row("status", meta.status.clone()),
        kv_row("favourites", group_thousands(meta.favourite_count as u64)),
        kv_row("plays", group_thousands(meta.play_count as u64)),
    ];
    if meta.video || meta.nsfw {
        let mut flags: Vec<Span<'static>> = Vec::new();
        if meta.video {
            flags.push("video".fg(text_dim()));
        }
        if meta.nsfw {
            if !flags.is_empty() {
                flags.push("  ·  ".fg(text_faint()));
            }
            flags.push("nsfw".fg(warning()));
        }
        rows.push(ListItem::new(Line::from(flags)));
    }
    if let Some(details) = details {
        append_set_extras(&mut rows, details);
    }
    rows.extend(diff_section_rows(meta, details));
    rows
}

/// Append the nzbasic-only SET-level extra columns to the preview:
/// tags/source/genre/language (each skipped when the field is blank) then
/// ranked/updated dates on their separate millis/seconds formatters. Per-diff
/// figures live in [`diff_section_rows`], not here.
fn append_set_extras(rows: &mut Vec<ListItem<'static>>, d: &BeatmapDetails) {
    for (key, value) in [
        ("tags", d.tags.trim()),
        ("source", d.source.trim()),
        ("genre", d.genre.trim()),
        ("language", d.language.trim()),
    ] {
        if !value.is_empty() {
            rows.push(kv_row(key, value.to_string()));
        }
    }
    // `approved_date` is epoch millis (the never-ranked sentinel is negative);
    // `last_update` is epoch seconds — separate formatters, both skipping the
    // non-positive/sentinel case.
    if let Some(date) = format_epoch_date(d.approved_date / 1000) {
        rows.push(kv_row("ranked", date));
    }
    if let Some(date) = format_epoch_date(d.last_update) {
        rows.push(kv_row("updated", date));
    }
}

/// Build the per-difficulty section: a header line (difficulty name + a
/// tier-colored star rating, plus `+N more` when the set carries multiple
/// diffs), a BPM kv row, four AR/CS/OD/HP bar meters, and `length`/`drain`
/// rows. The representative diff is the HARDEST one (highest star rating),
/// first-seen winning ties — matching `record_details`'s strict-`>` fold so a
/// set's preview and its recorded details agree. osu-route rows source the
/// representative from the nested `beatmaps[]` array (count known, no
/// combo/pass/hash — those fields aren't on [`Beatmap`]); nzbasic rows source
/// it from the recorded [`BeatmapDetails`] (count unknown, combo / pass count
/// / short hash appended). Empty when neither source has data.
fn diff_section_rows(
    meta: &BeatmapSetMeta,
    details: Option<&BeatmapDetails>,
) -> Vec<ListItem<'static>> {
    // `Iterator::max_by` returns the LAST equal element; reverse so the FIRST
    // diff in the array wins ties, agreeing with `record_details`'s strict `>`.
    if let Some(hardest) = meta.beatmaps.iter().rev().max_by(|a, b| {
        a.difficulty_rating
            .partial_cmp(&b.difficulty_rating)
            .unwrap_or(std::cmp::Ordering::Equal)
    }) {
        diff_rows(
            &hardest.version,
            hardest.difficulty_rating,
            hardest.bpm,
            hardest.ar,
            hardest.cs,
            hardest.od,
            hardest.hp,
            hardest.total_length,
            hardest.hit_length,
            Some(meta.beatmaps.len()),
            None,
        )
    } else if let Some(d) = details {
        diff_rows(
            &d.version,
            d.stars,
            d.bpm,
            d.ar,
            d.cs,
            d.od,
            d.hp,
            d.total_length,
            d.hit_length,
            None,
            Some(d),
        )
    } else {
        Vec::new()
    }
}

/// Render the per-difficulty rows for a single representative diff. `diff_count`
/// carries the set's total diff count when known (osu route) — it drives the
/// `+N more` suffix. `details` carries the nzbasic-only combo/pass/hash extras
/// when present.
#[allow(clippy::too_many_arguments)]
fn diff_rows(
    version: &str,
    stars: f64,
    bpm: f64,
    ar: f64,
    cs: f64,
    od: f64,
    hp: f64,
    total_length: u32,
    hit_length: u32,
    diff_count: Option<usize>,
    details: Option<&BeatmapDetails>,
) -> Vec<ListItem<'static>> {
    let mut rows = Vec::new();

    // Header: difficulty name + tier-colored star rating; `+N more` when the
    // set carries multiple diffs and the count is known.
    let mut header: Vec<Span<'static>> = vec![
        version.to_string().fg(text_dim()).bold(),
        format!(" ★{stars:.2}").fg(stars_color(stars)),
    ];
    if let Some(count) = diff_count
        && count > 1
    {
        header.push(format!(" +{} more", count - 1).fg(text_faint()));
    }
    rows.push(ListItem::new(Line::from(header)));

    rows.push(kv_row("bpm", format!("{bpm:.0}")));
    rows.push(bar_row("ar", ar));
    rows.push(bar_row("cs", cs));
    rows.push(bar_row("od", od));
    rows.push(bar_row("hp", hp));
    if total_length > 0 {
        rows.push(kv_row("length", format_drain(total_length)));
    }
    if hit_length > 0 {
        rows.push(kv_row("drain", format_drain(hit_length)));
    }

    // nzbasic-only extras (combo / pass count / short hash) — `Beatmap` does
    // not carry these, so the osu route omits them.
    if let Some(d) = details {
        if d.max_combo > 0 {
            rows.push(kv_row("max combo", group_thousands(d.max_combo as u64)));
        }
        if d.pass_count > 0 {
            rows.push(kv_row("pass count", group_thousands(d.pass_count as u64)));
        }
        if !d.hash.is_empty() {
            rows.push(kv_row("hash", short_hash(&d.hash)));
        }
    }
    rows
}

/// A bar-meter row for one of AR/CS/OD/HP: `<label> [████░░░░░░] <value>` with
/// a [`BAR_WIDTH`]-cell bar, filled cells in `accent`, empty cells + brackets
/// in `text_faint`. Compact (no [`KV_WIDTH`] padding) so the row fits the text
/// floor beside a cover.
fn bar_row(label: &'static str, value: f64) -> ListItem<'static> {
    let filled = (value / 10.0 * BAR_WIDTH as f64)
        .round()
        .clamp(0.0, BAR_WIDTH as f64) as usize;
    let mut spans: Vec<Span<'static>> =
        vec![label.fg(text_dim()), Span::raw(" [").fg(text_faint())];
    if filled > 0 {
        spans.push("█".repeat(filled).fg(accent()));
    }
    if filled < BAR_WIDTH {
        spans.push("░".repeat(BAR_WIDTH - filled).fg(text_faint()));
    }
    spans.push("]".fg(text_faint()));
    spans.push(format!(" {value:.1}").fg(text()));
    ListItem::new(Line::from(spans))
}

/// A unix epoch (seconds) as `YYYY-MM-DD`, or `None` for a non-positive value —
/// the never-ranked sentinel and any missing date both land here. Mirrors the
/// Downloads-tab date formatter.
fn format_epoch_date(secs: i64) -> Option<String> {
    if secs <= 0 {
        return None;
    }
    time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .map(|dt| {
            format!(
                "{:04}-{:02}-{:02}",
                dt.year(),
                u8::from(dt.month()),
                dt.day()
            )
        })
}

/// Drain time in seconds as `m:ss`.
fn format_drain(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// The leading 10 chars of an MD5, ellipsised — a full 32-char hash is preview
/// clutter, but the prefix still identifies the diff at a glance.
fn short_hash(hash: &str) -> String {
    match hash.char_indices().nth(10) {
        Some((idx, _)) => format!("{}…", &hash[..idx]),
        None => hash.to_string(),
    }
}

/// A static key→value preview row: key in `TEXT_DIM + bold` (the cloudy static-kv
/// treatment), column-aligned to [`KV_WIDTH`], value in `TEXT`.
fn kv_row(key: &str, value: String) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        format!("{key:<width$}  ", width = KV_WIDTH)
            .fg(text_dim())
            .bold(),
        value.fg(text()),
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

#[cfg(test)]
#[path = "../../tests/unit/tui_set_browse.rs"]
mod tests;
