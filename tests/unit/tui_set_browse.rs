//! Set-browse render tests: the per-difficulty section (tier-colored star
//! header + AR/CS/OD/HP bar meters) fed from either the osu route's nested
//! `beatmaps[]` array or the nzbasic route's `BeatmapDetails`; the epoch
//! formatters (millis `approved_date` vs seconds `last_update`); and the
//! hardest-diff-per-set fold.

use super::*;
use crate::app::covers::Covers;
use osu_downloader::search::Beatmap;
use ratatui::style::Color;
use ratatui::{Terminal, backend::TestBackend};
use std::collections::HashMap;

fn sample_meta(id: u32) -> BeatmapSetMeta {
    BeatmapSetMeta {
        id,
        title: "Song Title".to_string(),
        title_unicode: String::new(),
        artist: "Artist".to_string(),
        artist_unicode: String::new(),
        creator: "Mapper".to_string(),
        status: "ranked".to_string(),
        favourite_count: 12_345,
        play_count: 1_234_567,
        nsfw: false,
        video: false,
        beatmaps: Vec::new(),
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

/// A vertical-gradient cover at `w`x`h` so the halfblocks encoder emits colored
/// cells (adjacent rows differ), letting the render assert a populated cover.
/// Real asset sizes matter: `Resize::Fit` never upscales, so a fixture smaller
/// than the layout's allowance would cap the image early and stop exercising the
/// flush geometry. Square (`list@2x`, 300x300) and wide (`card@2x`, 800x280).
fn gradient_cover(w: u32, h: u32) -> image::DynamicImage {
    let span = h.max(1) - 1;
    let img = image::RgbImage::from_fn(w, h, |_, y| {
        let v = (y * 255 / span.max(1)) as u8;
        image::Rgb([v, 128, 255 - v])
    });
    image::DynamicImage::ImageRgb8(img)
}

/// A [`Covers`] with only the square variant ready — the wide upgrade never
/// fires, isolating the square right-hand column geometry.
fn covers_square_only(set_id: u32) -> Covers {
    let mut covers = Covers::new();
    let square = covers.picker.new_resize_protocol(gradient_cover(300, 300));
    covers.record_ready(set_id, Some(square), None);
    covers
}

/// A [`Covers`] with both variants ready, so a wide-enough pane swaps in the
/// wide (shorter) card.
fn covers_both_variants(set_id: u32) -> Covers {
    let mut covers = Covers::new();
    let square = covers.picker.new_resize_protocol(gradient_cover(300, 300));
    let wide = covers.picker.new_resize_protocol(gradient_cover(800, 280));
    covers.record_ready(set_id, Some(square), Some(wide));
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

/// How many rows carry a real background at column `x` — the cover's height,
/// since only the image paints a cell background in an unfocused preview.
fn colored_rows_at(buffer: &ratatui::buffer::Buffer, x: u16) -> usize {
    let area = *buffer.area();
    (0..area.height)
        .filter(|&y| buffer[(x, y)].bg != Color::Reset)
        .count()
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
    // Per-difficulty section: header (version + tier-colored star), BPM kv
    // row, AR/CS/OD/HP bar meters, and the nzbasic-only combo/pass/hash rows.
    assert!(text.contains("Insane"), "difficulty name missing:\n{text}");
    assert!(text.contains("★5.40"), "star rating missing:\n{text}");
    assert!(text.contains("█"), "bar meter cells missing:\n{text}");
    assert!(text.contains("180"), "bpm value missing:\n{text}");
    assert!(text.contains("1:58"), "drain time missing"); // 118s
    assert!(text.contains("abcdef0123…"), "short hash missing");
}

#[test]
fn osu_route_preview_has_no_detail_columns() {
    // A meta'd row with no recorded details and no nested beatmaps (the osu
    // route before the `beatmaps[]` array is captured) shows only the osu-batch
    // metadata — no diff section, no star rating, no genre.
    let browse = browse_with(vec![BrowseRow {
        id: 7,
        meta: Some(sample_meta(7)),
    }]);
    let text = render_browse(&browse);
    assert!(
        text.contains("Song Title"),
        "base metadata should still render"
    );
    assert!(!text.contains("★"), "no star rating without a diff section");
    assert!(!text.contains("genre"), "no genre row without details");
}

#[test]
fn osu_route_beatmaps_render_diff_spread() {
    // An osu-route row whose `beatmaps[]` array carries two diffs: the spread
    // lists both (one line each, star meter), the focused diff defaults to the
    // hardest (Expert), and its full attribute block renders below — bpm, the
    // AR/CS/OD/HP bar meters, the object-count breakdown, and the success-rate
    // bar. No `+N more` hint: every diff is listed.
    let beatmaps = vec![
        Beatmap {
            id: 1,
            beatmapset_id: 7,
            mode_int: 0,
            version: "Easy".to_string(),
            difficulty_rating: 2.0,
            bpm: 180.0,
            ar: 5.0,
            cs: 3.0,
            od: 4.0,
            hp: 4.0,
            total_length: 240,
            hit_length: 200,
            count_circles: 100,
            count_sliders: 50,
            count_spinners: 2,
            passcount: 80_000,
            playcount: 100_000,
        },
        Beatmap {
            id: 2,
            beatmapset_id: 7,
            mode_int: 0,
            version: "Expert".to_string(),
            difficulty_rating: 5.47,
            bpm: 180.0,
            ar: 9.0,
            cs: 4.0,
            od: 8.0,
            hp: 6.0,
            total_length: 240,
            hit_length: 200,
            count_circles: 410,
            count_sliders: 288,
            count_spinners: 14,
            passcount: 45_000,
            playcount: 70_000,
        },
    ];
    let meta = BeatmapSetMeta {
        beatmaps,
        ..sample_meta(7)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 7,
        meta: Some(meta),
    }]);
    let text = render_browse(&browse);
    // The spread lists every diff.
    assert!(
        text.contains("Easy"),
        "spread lists the easier diff:\n{text}"
    );
    assert!(
        text.contains("Expert"),
        "spread lists the hardest diff:\n{text}"
    );
    assert!(
        text.contains("★5.47"),
        "focused (hardest) star rating:\n{text}"
    );
    assert!(
        text.contains("★2.00"),
        "spread shows the easier diff's stars:\n{text}"
    );
    assert!(
        !text.contains("+1 more"),
        "no count hint — every diff is listed"
    );
    // The focused diff's attribute block.
    assert!(text.contains("180"), "bpm value missing:\n{text}");
    assert!(text.contains("█"), "bar meter cells missing:\n{text}");
    assert!(
        text.contains("objects"),
        "object-count row missing:\n{text}"
    );
    assert!(text.contains("410"), "focused diff's circle count:\n{text}");
    // 45000/70000 ≈ 64.3% → 64%.
    assert!(
        text.contains("64%"),
        "success-rate bar from passcount/playcount:\n{text}"
    );
    assert!(
        !text.contains("max combo"),
        "combo is nzbasic-only and must not render on the osu route"
    );
}

#[test]
fn unicode_title_and_artist_render_when_distinct() {
    // Cyrillic is single-cell, so it survives the buffer's per-cell join
    // unbroken (CJK would be split across two cells and defeat `.contains`).
    let meta = BeatmapSetMeta {
        title_unicode: "Песня".to_string(),
        artist_unicode: "Артист".to_string(),
        ..sample_meta(7)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 7,
        meta: Some(meta),
    }]);
    let text = render_browse(&browse);
    assert!(
        text.contains("Песня"),
        "unicode title under the romanised lead:\n{text}"
    );
    assert!(
        text.contains("Артист"),
        "unicode artist beside the romanised one:\n{text}"
    );
    // Romanised forms are still present alongside.
    assert!(text.contains("Song Title"));
    assert!(text.contains("Artist"));
}

#[test]
fn diff_cursor_cycles_the_focused_detail() {
    // Three diffs with distinct bpm, so the focused detail block is identifiable
    // by its bpm value. The default focus is the hardest diff (Expert).
    let make = |version: &str, stars: f64, bpm: f64| Beatmap {
        beatmapset_id: 7,
        mode_int: 0,
        version: version.to_string(),
        difficulty_rating: stars,
        bpm,
        ..Beatmap::default()
    };
    let meta = BeatmapSetMeta {
        beatmaps: vec![
            make("Easy", 2.0, 120.0),
            make("Normal", 3.4, 150.0),
            make("Expert", 5.47, 180.0),
        ],
        ..sample_meta(7)
    };
    let mut browse = browse_with(vec![BrowseRow {
        id: 7,
        meta: Some(meta),
    }]);
    browse.descend();
    browse.focus_preview();
    let hardest = render_browse(&browse);
    assert!(
        hardest.contains("180"),
        "default focus is the hardest diff (Expert):\n{hardest}"
    );

    // While the preview owns focus, ↑ steps the difficulty cursor within the
    // spread rather than scrolling the list.
    browse.scroll_up();
    let normal = render_browse(&browse);
    assert!(
        normal.contains("150"),
        "one step up focuses Normal:\n{normal}"
    );
    assert!(
        !normal.contains("180"),
        "Expert's bpm leaves the detail block once it is no longer focused:\n{normal}"
    );
}

#[test]
fn record_details_keeps_hardest_diff_per_set() {
    let mut browse = browse_with(vec![BrowseRow { id: 42, meta: None }]);
    let mut first = sample_details(42);
    first.stars = 3.0;
    first.max_combo = 111;
    let mut second = sample_details(42);
    second.stars = 5.4;
    second.max_combo = 999;
    // A third diff tied with the second on stars: the first-seen at the top
    // rating wins (strict `>`), so this one must NOT replace the second.
    let mut third = sample_details(42);
    third.stars = 5.4;
    third.max_combo = 777;
    browse.record_details(vec![first, second, third]);
    assert_eq!(
        browse.details_for(42).map(|d| d.max_combo),
        Some(999),
        "the hardest diff (highest stars) for a set wins, first-seen on ties"
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
    let covers = covers_square_only(42);

    // 90 wide: list pane 36, preview inner spans x=38..88 (50 columns). Square-only
    // (no wide), so the square column is offered 20 (50/5*2), which the 300x300
    // source fills at the halfblocks 10x20 font, landing flush-right at 88-20 = 68.
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
    let covers = covers_square_only(42);

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
    let covers = covers_square_only(42);

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

#[test]
fn a_wide_pane_swaps_in_the_shorter_wide_variant() {
    let browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(sample_meta(42)),
    }]);
    // 90x30: the column is offered 26, at/above WIDE_COVER_WIDTH, so with both
    // variants loaded the wide card is chosen. It's a wide crop, so at the same
    // column width it paints fewer rows than the square.
    const LAST_INNER_X: u16 = 87;
    let square = render_grid(&browse, Some(&covers_square_only(42)), 90, 30);
    let both = render_grid(&browse, Some(&covers_both_variants(42)), 90, 30);

    let square_rows = colored_rows_at(&square, LAST_INNER_X);
    let wide_rows = colored_rows_at(&both, LAST_INNER_X);
    assert!(
        square_rows > 0 && wide_rows > 0,
        "both stores paint a right-anchored cover"
    );
    assert!(
        wide_rows < square_rows,
        "the wide variant is shorter than the square at the same column width \
         (wide {wide_rows} rows, square {square_rows} rows)"
    );
    // Both are still right-anchored: the last inner column carries the cover.
    assert!(
        both[(LAST_INNER_X, 1)].bg != Color::Reset,
        "the wide variant is flush against the right edge too"
    );
}

#[test]
fn a_narrow_pane_keeps_the_square_even_with_the_wide_loaded() {
    let browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(sample_meta(42)),
    }]);
    // Both split (>=60 wide). 90-wide → preview inner 50 → column offered 26 (wide).
    // 72-wide → preview inner 40 → offered 16 (below WIDE_COVER_WIDTH 26), so the
    // square stays even with the wide loaded: a taller column than the wide crop.
    // The cover is right-anchored, so its column is the last inner col (width-3).
    let wide = render_grid(&browse, Some(&covers_both_variants(42)), 90, 30);
    let narrow = render_grid(&browse, Some(&covers_both_variants(42)), 72, 30);

    let wide_rows = colored_rows_at(&wide, 90 - 3);
    let narrow_rows = colored_rows_at(&narrow, 72 - 3);
    assert!(
        narrow_rows > wide_rows,
        "the narrow pane keeps the taller square while the wide pane goes wide \
         (narrow {narrow_rows} rows, wide {wide_rows} rows)"
    );
}

#[test]
fn a_short_title_keeps_the_cover_on_one_line() {
    let browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(sample_meta(42)),
    }]);
    // "Song Title" fits the ~22-col text column beside the cover, so the cover
    // stays and the title is one line (artist directly below it).
    let buffer = render_grid(&browse, Some(&covers_both_variants(42)), 90, 30);
    assert!(
        cover_start_x(&buffer, 1).is_some(),
        "a title that fits keeps its cover"
    );
    let title = preview_row_of(&buffer, "Song Title").expect("title renders");
    let artist = preview_row_of(&buffer, "Artist").expect("artist renders");
    assert_eq!(
        artist,
        title + 1,
        "a one-line title puts artist on the next row"
    );
}

#[test]
fn a_title_too_long_for_the_wide_column_collapses_to_the_square() {
    // 90x30: the wide text column is ~22, the square (collapsed) one ~28. A 23-col
    // title doesn't fit beside the wide crop but fits beside the square, so the
    // cover DOWNGRADES wide→square (still shown) and the title stays one line.
    let mid_title = BeatmapSetMeta {
        title: "A Medium Length Beatmap".to_string(), // 23 cols
        ..sample_meta(42)
    };
    let short = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(sample_meta(42)),
    }]);
    let mid = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(mid_title),
    }]);

    const LAST_INNER_X: u16 = 87;
    let short_buf = render_grid(&short, Some(&covers_both_variants(42)), 90, 30);
    let mid_buf = render_grid(&mid, Some(&covers_both_variants(42)), 90, 30);

    let short_rows = colored_rows_at(&short_buf, LAST_INNER_X);
    let mid_rows = colored_rows_at(&mid_buf, LAST_INNER_X);
    assert!(
        short_rows > 0 && mid_rows > 0,
        "the cover is shown in both — collapsing downgrades, never removes it"
    );
    assert!(
        mid_rows > short_rows,
        "the mid title collapses the short wide crop to the taller square \
         (short {short_rows} rows, mid {mid_rows} rows)"
    );
    // The collapsed square's wider text keeps the title on one line.
    let title = preview_row_of(&mid_buf, "A Medium Length Beatmap").expect("title renders");
    let artist = preview_row_of(&mid_buf, "Artist").expect("artist renders");
    assert_eq!(
        artist,
        title + 1,
        "the collapsed cover keeps the title one line"
    );
}

#[test]
fn a_title_too_long_for_even_the_collapsed_column_wraps_to_two_lines() {
    // Longer than even the ~28-col square text column, so it wraps to two lines —
    // but the square cover is STILL shown; wrapping is the last resort after
    // collapsing, not instead of it.
    let very_long = BeatmapSetMeta {
        title: "A Truly Enormous Beatmap Title Overruns".to_string(),
        ..sample_meta(42)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(very_long),
    }]);

    let buffer = render_grid(&browse, Some(&covers_both_variants(42)), 90, 30);
    assert!(
        colored_rows_at(&buffer, 90 - 3) > 0,
        "the square cover is still shown while the title wraps"
    );
    let first = preview_row_of(&buffer, "A Truly Enormous").expect("title line 1 renders");
    let artist = preview_row_of(&buffer, "Artist").expect("artist renders");
    assert_eq!(
        artist,
        first + 2,
        "an over-column title wraps to two lines, pushing artist down two rows"
    );
}
