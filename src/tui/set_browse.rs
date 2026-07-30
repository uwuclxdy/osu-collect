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
use crate::app::find_source::{BrowseRow, DiffSort, SetBrowse};
use osu_downloader::filter::BeatmapDetails;
use osu_downloader::search::{Beatmap, BeatmapSetMeta};
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
use super::{accent, focused_label, spinner_str, success, text, text_dim, text_faint, warning};

const PREVIEW_TITLE: &str = " PREVIEW ";
/// Widest key on the preview card, for column-aligning the static kv rows.
const KV_WIDTH: usize = "favourites".len();
/// Cell count of the AR/CS/OD/HP bar meters. Equal to the attributes' max
/// scale (10), so one cell == one attribute unit.
const BAR_WIDTH: usize = 10;
/// Cell count of the spread star meters: half [`BAR_WIDTH`], so each cell
/// covers two stars (0–10 in 5 cells). The `★X.XX` number already carries the
/// exact rating; the bar only conveys spread shape, so the coarser resolution
/// is no loss and the shorter bar truncates less in a narrow preview.
const SPREAD_BAR_WIDTH: usize = 5;

/// Content width available for browse-row label text in the list pane,
/// computed from the body area. Accounts for the pane border, padding, caret,
/// checkbox, and leading space. Returns `None` when the list area is too narrow
/// to right-align the star suffix usefully.
fn list_label_width(body_area: Rect) -> Option<u16> {
    let list_width = if body_area.width >= 60 && body_area.height >= 14 {
        (body_area.width / 5 * 2).clamp(28, 52)
    } else {
        body_area.width
    };
    // inner = list_width - 4 (borders 2 + padding.left 1 + padding.right 1)
    // label = inner - 6 (caret 2 + checkbox 3 + space 1)
    let label = list_width.saturating_sub(10);
    // Stars take ~7 cells; need ≥8 more for a readable title prefix.
    (label >= 15).then_some(label)
}

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
    let label_width = list_label_width(area);
    let list_items: Vec<ListItem<'static>> = browse
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            list_row(
                row,
                browse.is_selected(row.id),
                list_focused && cursor == Some(i),
                label_width,
            )
        })
        .collect();

    // The title rides as the preview's lead (`master_detail` wraps it to two
    // lines beside the cover); the fields fill `preview_items`. An id-only or
    // still-loading row has no lead, its id/loading note filling the items.
    let (preview_lead, preview_items) = match browse.highlighted_row() {
        Some(row) => preview_lead_and_items(
            row,
            browse.details_for(row.id),
            browse.focused_diff_index(),
            browse.diff_sort,
            enriching,
            tick,
        ),
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
/// promotes to TEXT + bold. `label_width` right-aligns the star suffix within
/// the list pane when `Some` (computed from the body area in [`render`]).
fn list_row(
    row: &BrowseRow,
    selected: bool,
    is_cursor: bool,
    label_width: Option<u16>,
) -> ListItem<'static> {
    // caret → checkbox → label (the contract's checkbox-row order).
    let mut spans = vec![widgets::focus_span(is_cursor)];
    spans.extend(widgets::checkbox_spans(selected));
    spans.push(Span::raw(" "));
    spans.extend(widgets::browse_row_label(
        row.id,
        row.meta.as_ref(),
        focused_label(is_cursor),
        label_width,
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
    focused_idx: Option<usize>,
    diff_sort: DiffSort,
    enriching: bool,
    tick: u64,
) -> (Option<PreviewLead>, Vec<ListItem<'static>>) {
    match &row.meta {
        Some(meta) => (
            Some(PreviewLead {
                text: meta.title.clone(),
                style: Style::new().fg(accent()).bold(),
            }),
            meta_field_rows(meta, details, focused_idx, diff_sort),
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
/// static kv rows, the video/nsfw flags, then the per-difficulty section
/// (both routes) behind a blank-line separator, and finally the nzbasic-only
/// set-level extras. Each is one line, so the cover/text budget is a plain
/// row count.
fn meta_field_rows(
    meta: &BeatmapSetMeta,
    details: Option<&BeatmapDetails>,
    focused_idx: Option<usize>,
    diff_sort: DiffSort,
) -> Vec<ListItem<'static>> {
    let mut rows = Vec::new();
    // Unicode (original-script) title under the romanised lead, when it differs
    // — the web's two-line title. Omitted when empty or identical (roman-only).
    if !meta.title_unicode.is_empty() && meta.title_unicode != meta.title {
        rows.push(ListItem::new(Line::from(
            meta.title_unicode.clone().fg(text_dim()),
        )));
    }
    // Artist: romanised, with the unicode form appended when it differs.
    if !meta.artist_unicode.is_empty() && meta.artist_unicode != meta.artist {
        rows.push(ListItem::new(Line::from(vec![
            meta.artist.clone().fg(text()),
            " · ".fg(text_faint()),
            meta.artist_unicode.clone().fg(text_dim()),
        ])));
    } else {
        rows.push(ListItem::new(Line::from(meta.artist.clone().fg(text()))));
    }
    rows.push(kv_row("mapper", meta.creator.clone()));
    rows.push(kv_row("status", meta.status.clone()));
    rows.push(kv_row(
        "favourites",
        group_thousands(meta.favourite_count as u64),
    ));
    rows.push(kv_row("plays", group_thousands(meta.play_count as u64)));
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
    // The difficulty spread is the most scannable beatmap data, so it sits right
    // after the core metadata behind a separator. Secondary set-level extras
    // (tags/genre/dates, nzbasic only) follow below.
    let diff = diff_block_rows(meta, details, focused_idx, diff_sort);
    if !diff.is_empty() {
        rows.push(ListItem::new(Line::default()));
        rows.push(eyebrow_row("DIFFICULTIES"));
        rows.extend(diff);
    }
    if let Some(details) = details
        && has_set_extras(details)
    {
        rows.push(ListItem::new(Line::default()));
        rows.push(eyebrow_row("METADATA"));
        append_set_extras(&mut rows, details);
    }
    rows
}

/// Append the nzbasic-only SET-level extra columns to the preview:
/// tags/source/genre/language (each skipped when the field is blank) then
/// ranked/updated dates on their separate millis/seconds formatters. Per-diff
/// figures live in [`diff_block_rows`], not here.
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

/// Build the difficulty section. When the row carries a `beatmaps[]` spread
/// (every route once enriched), render a one-line-per-diff list with per-diff
/// star meters above the focused diff's full attribute block, sorted by
/// `diff_sort`. Otherwise fall back to the single recorded diff
/// ([`BeatmapDetails`], the nzbasic path while a set's spread has not
/// populated). Empty when neither source has data.
fn diff_block_rows(
    meta: &BeatmapSetMeta,
    details: Option<&BeatmapDetails>,
    focused_idx: Option<usize>,
    diff_sort: DiffSort,
) -> Vec<ListItem<'static>> {
    if let Some(focused) = focused_idx
        && !meta.beatmaps.is_empty()
    {
        spread_rows(&meta.beatmaps, focused, details, diff_sort)
    } else if let Some(d) = details {
        recorded_diff_rows(d)
    } else {
        Vec::new()
    }
}

/// The full difficulty spread: one line per diff (name + tier-coloured star
/// meter, the focused diff marked `▸`), a separator, then the focused diff's
/// attribute block. Diffs are sorted by `sort` so the easier end lists first by
/// default. The nzbasic-only combo/hash extras append only when the focused
/// diff is the recorded representative (the hardest) — they are recorded for
/// that diff alone, so rendering them under a different diff would mislabel.
fn spread_rows(
    beatmaps: &[Beatmap],
    focused: usize,
    details: Option<&BeatmapDetails>,
    sort: DiffSort,
) -> Vec<ListItem<'static>> {
    let focused = focused.min(beatmaps.len().saturating_sub(1));
    // Build sorted indices from the original array — tracking by original index
    // is id-independent, so tests with defaulted ids don't collapse.
    let mut indices: Vec<usize> = (0..beatmaps.len()).collect();
    match sort {
        DiffSort::StarsAsc => indices.sort_by(|&a, &b| {
            beatmaps[a]
                .difficulty_rating
                .partial_cmp(&beatmaps[b].difficulty_rating)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        DiffSort::StarsDesc => indices.sort_by(|&a, &b| {
            beatmaps[b]
                .difficulty_rating
                .partial_cmp(&beatmaps[a].difficulty_rating)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    }
    let focused = indices
        .iter()
        .position(|&i| i == focused)
        .unwrap_or(focused);
    // Pad every diff name to the spread's widest so the star ratings and meters
    // share a column across the list (display-width-aware for CJK names).
    let name_w = indices
        .iter()
        .map(|&i| display_width(&beatmaps[i].version))
        .max()
        .unwrap_or(0);
    let mut rows: Vec<ListItem<'static>> = Vec::new();
    for (pos, &i) in indices.iter().enumerate() {
        rows.push(ListItem::new(spread_line(
            &beatmaps[i],
            pos == focused,
            name_w,
        )));
    }
    let focused_diff = &beatmaps[indices[focused]];
    rows.push(ListItem::new(Line::default()));
    rows.extend(beat_attr_rows(focused_diff));
    // The hardest diff is the one with no strictly harder peer (used for the
    // nzbasic-only combo/hash extras below). Scan the original array — every
    // diff is present regardless of sort.
    let is_hardest = !beatmaps
        .iter()
        .any(|b| b.difficulty_rating > focused_diff.difficulty_rating);
    if is_hardest && let Some(d) = details {
        append_recorded_per_diff(&mut rows, d);
    }
    rows
}

/// The focused diff's attribute block (from a [`Beatmap`]): the core attribute
/// rows, the object-count breakdown, then the success-rate bar. No header — the
/// focused diff's `▸` spread line already names it and shows its star meter, so
/// a repeated header would only misalign with the kv/bar rows below.
fn beat_attr_rows(b: &Beatmap) -> Vec<ListItem<'static>> {
    let mut rows = core_attr_rows(b.bpm, b.ar, b.cs, b.od, b.hp, b.total_length, b.hit_length);
    let objects = b.count_circles + b.count_sliders + b.count_spinners;
    if objects > 0 {
        rows.push(objects_row(
            b.count_circles,
            b.count_sliders,
            b.count_spinners,
        ));
    }
    rows.push(ratio_bar_row("success", b.passcount, b.playcount));
    rows
}

/// The single recorded-diff block (the nzbasic fallback): name + star meter,
/// core attributes, a success-rate bar from the recorded pass/play counts, then
/// the combo/hash extras. Used only when a set has no `beatmaps[]` spread.
fn recorded_diff_rows(d: &BeatmapDetails) -> Vec<ListItem<'static>> {
    let mut header = vec![d.version.to_string().fg(text_dim()).bold()];
    header.extend(star_meter_spans(d.stars));
    let mut rows = vec![ListItem::new(Line::from(header))];
    rows.extend(core_attr_rows(
        d.bpm,
        d.ar,
        d.cs,
        d.od,
        d.hp,
        d.total_length,
        d.hit_length,
    ));
    rows.push(ratio_bar_row("success", d.pass_count, d.play_count));
    append_recorded_per_diff(&mut rows, d);
    rows
}

/// One spread line: a `▸` caret on the focused diff (a blank on the others),
/// the difficulty name padded to `name_w`, then the tier-coloured star rating
/// and meter. Padding the name puts every diff's star rating and meter in one
/// column. The meter truncates at the pane edge in a narrow preview; the
/// `★X.XX` rating always precedes it, so the essential figure stays visible.
fn spread_line(diff: &Beatmap, focused: bool, name_w: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> =
        vec![(if focused { "▸ " } else { "  " }).fg(if focused { accent() } else { text_faint() })];
    spans.push(pad_right(&diff.version, name_w).fg(if focused { text() } else { text_dim() }));
    spans.extend(star_meter_spans(diff.difficulty_rating));
    Line::from(spans)
}

/// Display width of `s` in terminal columns (unicode-aware), via ratatui's own
/// span measurement so wide (CJK) names pad by cells, not bytes.
fn display_width(s: &str) -> usize {
    Span::raw(s).width()
}

/// `s` right-padded with spaces to `width` display columns. Already-wide or
/// over-width strings are returned unchanged (a name longer than the spread's
/// max is not truncated — it just sets the column for the rest).
fn pad_right(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

/// Append the nzbasic-only combo/hash extras for the recorded diff.
fn append_recorded_per_diff(rows: &mut Vec<ListItem<'static>>, d: &BeatmapDetails) {
    if d.max_combo > 0 {
        rows.push(kv_row("max combo", group_thousands(d.max_combo as u64)));
    }
    if !d.hash.is_empty() {
        rows.push(kv_row("hash", short_hash(&d.hash)));
    }
}

/// The core attribute rows shared by both detail sources: bpm + the four
/// AR/CS/OD/HP bar meters + length/drain. Each is one line.
fn core_attr_rows(
    bpm: f64,
    ar: f64,
    cs: f64,
    od: f64,
    hp: f64,
    total_length: u32,
    hit_length: u32,
) -> Vec<ListItem<'static>> {
    let mut rows = vec![
        kv_row("bpm", format!("{bpm:.0}")),
        bar_row("ar", ar),
        bar_row("cs", cs),
        bar_row("od", od),
        bar_row("hp", hp),
    ];
    if total_length > 0 {
        rows.push(kv_row("length", format_drain(total_length)));
    }
    if hit_length > 0 {
        rows.push(kv_row("drain", format_drain(hit_length)));
    }
    rows
}

/// The object-count breakdown row: total, then circles / sliders / spinners.
/// Styled like a kv row so the column alignment holds across the detail block.
fn objects_row(circles: u32, sliders: u32, spinners: u32) -> ListItem<'static> {
    let total = circles + sliders + spinners;
    ListItem::new(Line::from(vec![
        format!("{:<width$}  ", "objects", width = KV_WIDTH)
            .fg(text_dim())
            .bold(),
        total.to_string().fg(text()),
        "  circles ".fg(text_faint()),
        circles.to_string().fg(text_dim()),
        "  sliders ".fg(text_faint()),
        sliders.to_string().fg(text_dim()),
        "  spinners ".fg(text_faint()),
        spinners.to_string().fg(text_dim()),
    ]))
}

/// A tier-coloured star meter: the `★X.XX` rating followed by a
/// [`SPREAD_BAR_WIDTH`] cell bar (one cell per two stars, saturating past 10).
/// Filled cells take the tier colour; empty cells are faint.
fn star_meter_spans(stars: f64) -> Vec<Span<'static>> {
    let filled = (stars / 2.0).round().clamp(0.0, SPREAD_BAR_WIDTH as f64) as usize;
    let color = stars_color(stars);
    let mut spans = vec![format!(" ★{stars:.2} ").fg(color)];
    if filled > 0 {
        spans.push("█".repeat(filled).fg(color));
    }
    if filled < SPREAD_BAR_WIDTH {
        spans.push("░".repeat(SPREAD_BAR_WIDTH - filled).fg(text_faint()));
    }
    spans
}

/// A success-rate bar row: `success  NN% ██████░░░░`, the fill being
/// `numerator / denominator`. A zero denominator (no plays recorded) renders the
/// counts as "no plays" — a beatmap never played has no rate to show.
fn ratio_bar_row(label: &'static str, numerator: u32, denominator: u32) -> ListItem<'static> {
    let mut spans: Vec<Span<'static>> = vec![
        format!("{label:<width$}  ", width = KV_WIDTH)
            .fg(text_dim())
            .bold(),
    ];
    if denominator == 0 {
        spans.push("no plays".fg(text_faint()));
    } else {
        let pct = (numerator as f64 / denominator as f64 * 100.0)
            .round()
            .clamp(0.0, 100.0) as u64;
        let filled = (pct as f64 / 100.0 * BAR_WIDTH as f64).round() as usize;
        spans.push(format!("{pct:>3}% ").fg(text()));
        if filled > 0 {
            spans.push("█".repeat(filled).fg(success()));
        }
        if filled < BAR_WIDTH {
            spans.push("░".repeat(BAR_WIDTH - filled).fg(text_faint()));
        }
    }
    ListItem::new(Line::from(spans))
}

/// A bar-meter row for one of AR/CS/OD/HP: label ([`KV_WIDTH`]-aligned,
/// matching [`kv_row`]), then the numeric value right-aligned in a 4-char
/// field, then a [`BAR_WIDTH`]-cell bar. Right-aligning the value (so `9.0`
/// and `10.0` occupy the same width) keeps the bar cells in one column across
/// rows and lined up with [`ratio_bar_row`]'s matching 5-char value field.
/// Filled cells in `accent`, empty in `text_faint`.
fn bar_row(label: &'static str, value: f64) -> ListItem<'static> {
    let filled = (value / 10.0 * BAR_WIDTH as f64)
        .round()
        .clamp(0.0, BAR_WIDTH as f64) as usize;
    let mut spans: Vec<Span<'static>> = vec![
        format!("{label:<width$}  ", width = KV_WIDTH)
            .fg(text_dim())
            .bold(),
        format!("{value:>4.1} ").fg(text()),
    ];
    if filled > 0 {
        spans.push("█".repeat(filled).fg(accent()));
    }
    if filled < BAR_WIDTH {
        spans.push("░".repeat(BAR_WIDTH - filled).fg(text_faint()));
    }
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

/// A UPPERCASE TRACKED `TEXT_DIM` section label inside the preview — the
/// contract's eyebrow for internal sections (no bold, no horizontal rule).
fn eyebrow_row(label: &'static str) -> ListItem<'static> {
    ListItem::new(Line::from(label.fg(text_dim())))
}

/// Whether [`append_set_extras`] would push any row for `d`, so the caller can
/// gate the `METADATA` eyebrow + separator on having content to show.
fn has_set_extras(d: &BeatmapDetails) -> bool {
    !d.tags.trim().is_empty()
        || !d.source.trim().is_empty()
        || !d.genre.trim().is_empty()
        || !d.language.trim().is_empty()
        || d.approved_date > 0
        || d.last_update > 0
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
