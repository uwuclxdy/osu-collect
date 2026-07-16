//! Set-browse render tests: the nzbasic-only detail columns folded into the
//! preview, the epoch formatters (millis `approved_date` vs seconds
//! `last_update`), and the first-diff-per-set fold.

use super::*;
use crate::app::covers::Covers;
use ratatui::style::Color;
use ratatui::{Terminal, backend::TestBackend};
use std::collections::HashMap;

fn sample_meta(id: u32) -> BeatmapSetMeta {
    BeatmapSetMeta {
        id,
        title: "Song Title".to_string(),
        artist: "Artist".to_string(),
        creator: "Mapper".to_string(),
        status: "ranked".to_string(),
        favourite_count: 12_345,
        play_count: 1_234_567,
        nsfw: false,
        video: false,
    }
}

/// A representative diff row. `approved_date` is epoch **millis**, `last_update`
/// epoch **seconds**: the same two calendar dates only decode correctly when each
/// uses its own scale, so the render asserts both.
fn sample_details(set_id: u32) -> BeatmapDetails {
    BeatmapDetails {
        id: set_id * 10,
        set_id,
        title: "Song Title".to_string(),
        artist: "Artist".to_string(),
        creator: "Mapper".to_string(),
        version: "Insane".to_string(),
        stars: 5.4,
        bpm: 180.0,
        ar: 9.0,
        cs: 4.0,
        od: 8.0,
        hp: 6.0,
        status: None,
        mode: None,
        total_length: 210,
        favourite_count: 12_345,
        play_count: 1_234_567,
        size: 0,
        hash: "abcdef0123456789fedcba".to_string(),
        tags: "anime tv-size vocaloid".to_string(),
        source: "Attack on Titan".to_string(),
        genre: "Anime".to_string(),
        language: "Japanese".to_string(),
        max_combo: 1_234,
        hit_length: 118,
        pass_count: 45_678,
        approved_date: 1_600_000_000_000, // ms → 2020-09-13
        last_update: 1_600_500_000,       // s  → 2020-09-19
    }
}

fn browse_with(rows: Vec<BrowseRow>) -> SetBrowse {
    let mut browse = SetBrowse::new();
    browse.set_rows(rows, &HashMap::new());
    browse
}

fn render_browse(browse: &SetBrowse) -> String {
    let backend = TestBackend::new(90, 30);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");
    terminal
        .draw(|frame| {
            render(
                frame,
                Rect::new(0, 0, 90, 30),
                browse,
                " RESULTS ",
                Line::from(""),
                0,
                None,
            );
        })
        .expect("browse should render");
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

/// A vertical-gradient cover so the halfblocks encoder emits colored cells
/// (adjacent rows differ), letting the render assert a populated cover column.
/// 300x300 mirrors the real `list@2x` asset: `Resize::Fit` never upscales, so a
/// smaller fixture would cap the column below its allowance and stop exercising
/// the flush-right geometry.
fn gradient_cover() -> image::DynamicImage {
    let img = image::RgbImage::from_fn(300, 300, |_, y| {
        let v = (y * 255 / 299) as u8;
        image::Rgb([v, 128, 255 - v])
    });
    image::DynamicImage::ImageRgb8(img)
}

/// A [`Covers`] whose `set_id` cover is already `Ready`, built with the
/// test-safe halfblocks picker (no terminal query, no network).
fn covers_ready(set_id: u32) -> Covers {
    let mut covers = Covers::new();
    let protocol = covers.picker.new_resize_protocol(gradient_cover());
    covers.record_ready(set_id, protocol);
    covers
}

/// First inner column of the preview pane in a 90-wide split: list pane is 36
/// wide, preview border at x=36, +1 border +1 padding. Sampling from here scopes
/// assertions to the preview, away from the list pane's text + cursor highlight.
const PREVIEW_X: u16 = 38;

/// Render at `w`x`h` with an optional cover store, returning the buffer.
fn render_grid(
    browse: &SetBrowse,
    covers: Option<&Covers>,
    w: u16,
    h: u16,
) -> ratatui::buffer::Buffer {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");
    terminal
        .draw(|frame| {
            render(
                frame,
                Rect::new(0, 0, w, h),
                browse,
                " RESULTS ",
                Line::from(""),
                0,
                covers,
            );
        })
        .expect("browse should render");
    terminal.backend().buffer().clone()
}

/// First preview-pane row (`x >= PREVIEW_X`) whose text contains `needle`.
fn preview_row_of(buffer: &ratatui::buffer::Buffer, needle: &str) -> Option<usize> {
    let area = *buffer.area();
    (0..area.height)
        .find(|&y| {
            (PREVIEW_X..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains(needle)
        })
        .map(|y| y as usize)
}

/// The leftmost preview-pane column in `row` carrying a real (non-`Reset`)
/// background — the halfblocks encoder always paints one and nothing else in an
/// unfocused preview does, so this is where the cover column starts. `None` when
/// the row holds no cover.
fn cover_start_x(buffer: &ratatui::buffer::Buffer, row: u16) -> Option<u16> {
    let area = *buffer.area();
    (PREVIEW_X..area.width).find(|&x| buffer[(x, row)].bg != Color::Reset)
}

/// Whether ANY cell in rows `[0, rows)` carries a real background — used on a
/// single-pane preview (no list pane to filter out), so a colored cell there can
/// only be a cover.
fn any_colored_in_top_rows(buffer: &ratatui::buffer::Buffer, rows: u16) -> bool {
    let area = *buffer.area();
    (0..rows.min(area.height)).any(|y| (0..area.width).any(|x| buffer[(x, y)].bg != Color::Reset))
}

/// Row `y` of the preview, clipped to the columns left of `end_x`.
fn preview_text_before(buffer: &ratatui::buffer::Buffer, y: u16, end_x: u16) -> String {
    (PREVIEW_X..end_x)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// The whole buffer as one string, for a presence check that ignores geometry.
fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn nzbasic_details_render_extra_columns_with_scale_correct_dates() {
    let mut browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(sample_meta(42)),
    }]);
    browse.record_details(vec![sample_details(42)]);

    let text = render_browse(&browse);

    // Set-level columns.
    assert!(text.contains("genre"), "genre row missing:\n{text}");
    assert!(text.contains("Anime"), "genre value missing");
    assert!(text.contains("source"), "source row missing");
    assert!(text.contains("language"), "language row missing");
    // Millis vs seconds each decode to their own date — a single formatter would
    // put one of these in the wrong millennium.
    assert!(
        text.contains("2020-09-13"),
        "ranked (millis) date missing:\n{text}"
    );
    assert!(
        text.contains("2020-09-19"),
        "updated (seconds) date missing:\n{text}"
    );
    // Per-diff figures behind the divider.
    assert!(text.contains("(one diff)"), "diff divider missing");
    assert!(text.contains("1:58"), "drain time missing"); // 118s
    assert!(text.contains("abcdef0123…"), "short hash missing");
}

#[test]
fn osu_route_preview_has_no_detail_columns() {
    // A meta'd row with no recorded details (the osu route) shows only the
    // osu-batch metadata — no divider, no genre.
    let browse = browse_with(vec![BrowseRow {
        id: 7,
        meta: Some(sample_meta(7)),
    }]);
    let text = render_browse(&browse);
    assert!(
        text.contains("Song Title"),
        "base metadata should still render"
    );
    assert!(!text.contains("(one diff)"), "no divider without details");
    assert!(!text.contains("genre"), "no genre row without details");
}

#[test]
fn record_details_keeps_first_diff_per_set() {
    let mut browse = browse_with(vec![BrowseRow { id: 42, meta: None }]);
    let mut first = sample_details(42);
    first.max_combo = 111;
    let mut second = sample_details(42);
    second.max_combo = 999;
    browse.record_details(vec![first, second]);
    assert_eq!(
        browse.details_for(42).map(|d| d.max_combo),
        Some(111),
        "the first diff row for a set wins"
    );
    assert!(
        browse.details_for(99).is_none(),
        "unknown set has no details"
    );
}

#[test]
fn epoch_formatter_splits_scales_and_skips_the_sentinel() {
    // 2020-09-13 whether fed as the divided-millis or the raw-seconds path.
    assert_eq!(
        format_epoch_date(1_600_000_000_000 / 1000).as_deref(),
        Some("2020-09-13")
    );
    assert_eq!(
        format_epoch_date(1_600_000_000).as_deref(),
        Some("2020-09-13")
    );
    // The never-ranked sentinel (-62135596800000 ms) and a zero/blank date both
    // render nothing rather than year 0001 or a January-1970 artefact.
    assert_eq!(format_epoch_date(-62_135_596_800_000 / 1000), None);
    assert_eq!(format_epoch_date(0), None);
}

#[test]
fn drain_and_hash_formatters() {
    assert_eq!(format_drain(118), "1:58");
    assert_eq!(format_drain(9), "0:09");
    assert_eq!(format_drain(600), "10:00");
    assert_eq!(short_hash("abcdef0123456789"), "abcdef0123…");
    assert_eq!(short_hash("short"), "short");
}

#[test]
fn ready_cover_paints_a_right_column_beside_the_text() {
    let browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(sample_meta(42)),
    }]);
    let covers = covers_ready(42);

    // 90 wide: list pane 36, preview inner spans x=38..88 (50 columns). The
    // cover is offered 20 (50/5*2) and the 300x300 source fills all of them at
    // the halfblocks picker's fixed 10x20 font, so it lands at 88-20 = 68.
    const COVER_X: u16 = 68;
    const LAST_INNER_X: u16 = 87;

    let with_cover = render_grid(&browse, Some(&covers), 90, 30);
    let without = render_grid(&browse, None, 90, 30);

    assert_eq!(
        cover_start_x(&with_cover, 1),
        Some(COVER_X),
        "the cover column should start where its fitted width leaves the right edge"
    );
    assert!(
        with_cover[(LAST_INNER_X, 1)].bg != Color::Reset,
        "the cover should sit flush against the preview's last inner column"
    );
    assert_eq!(
        cover_start_x(&without, 1),
        None,
        "no cover means no colored cells in the preview"
    );

    // The whole point of a column over a band: the metadata keeps its rows.
    let title_with = preview_row_of(&with_cover, "Song Title").expect("title renders with cover");
    let title_without =
        preview_row_of(&without, "Song Title").expect("title renders without cover");
    assert_eq!(
        title_with, title_without,
        "a right-column cover must not push the text down \
         (with cover row {title_with}, text-only row {title_without})"
    );
    assert!(
        preview_text_before(&with_cover, title_with as u16, COVER_X).contains("Song Title"),
        "the title must render entirely left of the cover column"
    );
}

#[test]
fn short_pane_skips_the_cover_even_when_ready() {
    let mut browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(sample_meta(42)),
    }]);
    // Focus the preview so the small-terminal fallback renders it full-width
    // (not the list pane); its inner height (3 = 5 − 2 borders) is below
    // MIN_IMAGE_INNER_HEIGHT (4), so the cover is skipped, text-only.
    browse.focus_preview();
    let covers = covers_ready(42);

    let buffer = render_grid(&browse, Some(&covers), 90, 5);
    assert!(
        !any_colored_in_top_rows(&buffer, 4),
        "a short pane must not carve a cover column"
    );
    assert!(
        buffer_text(&buffer).contains("Song Title"),
        "the preview text still renders when the cover is skipped"
    );
}

#[test]
fn narrow_pane_skips_the_cover_to_keep_the_text_readable() {
    let mut browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(sample_meta(42)),
    }]);
    // Single-pane preview 38 wide → 34 inner columns, one short of the
    // COVER_WIDTH_MIN + gap + MIN_TEXT_WIDTH floor, so the text takes all of it.
    browse.focus_preview();
    let covers = covers_ready(42);

    let buffer = render_grid(&browse, Some(&covers), 38, 20);
    assert!(
        !any_colored_in_top_rows(&buffer, 20),
        "a narrow pane must not carve a cover column"
    );
    assert!(
        buffer_text(&buffer).contains("Song Title"),
        "the preview text still renders when the cover is skipped"
    );
}
