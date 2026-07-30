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

/// Hold `set_id` as the highlight past the render gate's dwell, the state a
/// session reaches after ~200ms parked on one row. Runs after the variants are
/// recorded, matching the production order (the prefetch settles first, the
/// fetch lands second), so the already-cached id is never re-marked `Pending`.
fn settle(covers: &mut Covers, set_id: u32) {
    for _ in 0..8 {
        covers.poll_prefetch(Some(set_id));
    }
}

/// A [`Covers`] with only the square variant ready — the wide upgrade never
/// fires, isolating the square right-hand column geometry.
fn covers_square_only(set_id: u32) -> Covers {
    let mut covers = Covers::new();
    let square = covers.picker.new_resize_protocol(gradient_cover(300, 300));
    covers.record_ready(set_id, Some(square), None);
    settle(&mut covers, set_id);
    covers
}

/// A [`Covers`] with both variants ready, so a wide-enough pane swaps in the
/// wide (shorter) card.
fn covers_both_variants(set_id: u32) -> Covers {
    let mut covers = Covers::new();
    let square = covers.picker.new_resize_protocol(gradient_cover(300, 300));
    let wide = covers.picker.new_resize_protocol(gradient_cover(800, 280));
    covers.record_ready(set_id, Some(square), Some(wide));
    settle(&mut covers, set_id);
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

/// The leftmost preview-pane column in `row` carrying an IMAGE background: one
/// the halfblocks encoder painted, as opposed to the flat theme fill the cover
/// band lays over its gap columns or the bare `Reset` of untouched text cells.
/// Nothing else in an unfocused preview paints a cell background, so this is
/// where the artwork starts. `None` when the row holds no cover.
fn cover_start_x(buffer: &ratatui::buffer::Buffer, row: u16) -> Option<u16> {
    let area = *buffer.area();
    let band = crate::tui::theme().bg;
    (PREVIEW_X..area.width).find(|&x| {
        let bg = buffer[(x, row)].bg;
        bg != Color::Reset && bg != band
    })
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
fn spread_star_ratings_align_in_one_column() {
    // Names of differing width ("A" vs "LongerName") must not ragged the star
    // ratings: the block is anchored to the row's right edge, so the name is what
    // gives, never the `★` column.
    let make = |version: &str, stars: f64| Beatmap {
        beatmapset_id: 7,
        mode_int: 0,
        version: version.to_string(),
        difficulty_rating: stars,
        ..Beatmap::default()
    };
    let meta = BeatmapSetMeta {
        beatmaps: vec![make("A", 2.0), make("LongerName", 5.47)],
        ..sample_meta(7)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 7,
        meta: Some(meta),
    }]);
    let buf = render_grid(&browse, None, 90, 30);
    let star_cols = star_columns(&buf);
    assert!(
        star_cols.len() == 1,
        "all spread star ratings share one column: {star_cols:?}"
    );
}

/// A minimal spread entry: only the fields the spread line reads.
fn spread_diff(version: &str, stars: f64) -> Beatmap {
    Beatmap {
        beatmapset_id: 7,
        mode_int: 0,
        version: version.to_string(),
        difficulty_rating: stars,
        ..Beatmap::default()
    }
}

/// The `★` columns used by every spread row in the preview pane.
fn star_columns(buffer: &ratatui::buffer::Buffer) -> std::collections::HashSet<u16> {
    let area = *buffer.area();
    (0..area.height)
        .flat_map(|y| {
            (PREVIEW_X..area.width)
                .filter(|&x| buffer[(x, y)].symbol() == "★")
                .collect::<Vec<u16>>()
        })
        .collect()
}

#[test]
fn spread_meters_spend_one_cell_per_star() {
    // The meter's length is the rating: 3.0 stars fill 3 of 10 cells, 7.0 fill 7.
    // A halved scale (or a 5-cell bar) puts both on different counts.
    let meta = BeatmapSetMeta {
        beatmaps: vec![spread_diff("Hard", 3.0), spread_diff("Extra", 7.0)],
        ..sample_meta(7)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 7,
        meta: Some(meta),
    }]);
    let buf = render_grid(&browse, None, 90, 30);
    let width = buf.area().width;
    for (rating, filled) in [("★3.00", 3usize), ("★7.00", 7usize)] {
        let y = preview_row_of(&buf, rating).expect("spread row renders") as u16;
        let row = preview_text_before(&buf, y, width);
        assert_eq!(
            row.matches('█').count(),
            filled,
            "{rating} fills one cell per star:\n{row}"
        );
        assert_eq!(
            row.matches('░').count(),
            10 - filled,
            "{rating} leaves the rest of a 10-cell meter empty:\n{row}"
        );
    }
}

#[test]
fn a_sentence_length_diff_name_keeps_its_rating_and_meter_on_screen() {
    // The real case: one diff name long enough to run past the preview's text
    // column. Padding every name to the widest used to push the whole `★` block
    // off the pane, taking every OTHER diff's rating with it.
    const LONG: &str = "If my voice has a place where it can belong, I wish for it to reach you";
    let meta = BeatmapSetMeta {
        beatmaps: vec![spread_diff("Easy", 2.0), spread_diff(LONG, 5.47)],
        ..sample_meta(7)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 7,
        meta: Some(meta),
    }]);
    let buf = render_grid(&browse, None, 90, 30);
    let width = buf.area().width;

    let y = preview_row_of(&buf, "★5.47").expect("the long diff keeps its rating") as u16;
    let row = preview_text_before(&buf, y, width);
    assert_eq!(
        row.matches('█').count() + row.matches('░').count(),
        10,
        "the whole meter stays on screen beside the long name:\n{row}"
    );
    assert!(
        row.contains('…'),
        "the name gives way to the rating, not the other way round:\n{row}"
    );

    let star_cols = star_columns(&buf);
    assert!(
        star_cols.len() == 1,
        "both spread rows anchor their rating to one column: {star_cols:?}"
    );
}

#[test]
fn detail_bar_meters_align_across_value_widths() {
    // AR/OD at 10.0 (a 4-char value) and CS/HP below 10 (3-char) must still
    // start their bar cells in one column, lined up with the success bar.
    let diff = Beatmap {
        beatmapset_id: 7,
        mode_int: 0,
        version: "X".to_string(),
        difficulty_rating: 5.0,
        bpm: 180.0,
        ar: 10.0,
        cs: 4.0,
        od: 10.0,
        hp: 6.0,
        total_length: 100,
        hit_length: 80,
        count_circles: 1,
        count_sliders: 1,
        count_spinners: 1,
        passcount: 1,
        playcount: 2,
        ..Beatmap::default()
    };
    let meta = BeatmapSetMeta {
        beatmaps: vec![diff],
        ..sample_meta(7)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 7,
        meta: Some(meta),
    }]);
    let buf = render_grid(&browse, None, 90, 30);
    let area = *buf.area();
    // Collect the first bar-cell column of every labeled detail row (ar/cs/od/
    // hp/success). kv-only rows (bpm/length/…) and the objects row have no bar
    // cells, so they contribute nothing; spread lines start with a caret.
    let mut bar_cols: std::collections::HashSet<u16> = std::collections::HashSet::new();
    for y in 0..area.height {
        let labeled = buf[(PREVIEW_X, y)]
            .symbol()
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase());
        if !labeled {
            continue;
        }
        for x in PREVIEW_X..area.width {
            let s = buf[(x, y)].symbol();
            if s == "█" || s == "░" {
                bar_cols.insert(x);
                break;
            }
        }
    }
    assert!(
        bar_cols.len() == 1,
        "AR/CS/OD/HP/success bar cells share one column: {bar_cols:?}"
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

/// The list pane only builds the rows the viewport resolves to, so a row's
/// position in the window is not its position in the browse. A cursor parked
/// past the first page is the only fixture where the two differ.
#[test]
fn a_scrolled_list_keeps_the_caret_and_checkbox_on_the_cursor_row() {
    let rows: Vec<BrowseRow> = (1..=60u32)
        .map(|id| BrowseRow {
            id,
            meta: Some(BeatmapSetMeta {
                title: format!("Song {id:03}"),
                ..sample_meta(id)
            }),
        })
        .collect();
    let mut browse = browse_with(rows);
    browse.descend();
    browse.scroll_to_edge(false);
    browse.toggle_selected();

    let buffer = render_grid(&browse, None, 90, 30);
    // The list pane spans x 0..36 at 90 columns; clipping there keeps the
    // preview's own copy of the title out of the read.
    let list_rows: Vec<String> = (0..buffer.area().height)
        .map(|y| (0..36).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();

    assert!(
        !list_rows.iter().any(|row| row.contains("Song 001")),
        "the fixture has to have scrolled off the first page:\n{list_rows:#?}"
    );
    let carets: Vec<&String> = list_rows.iter().filter(|row| row.contains('❯')).collect();
    assert_eq!(
        carets.len(),
        1,
        "exactly one list row carries the caret:\n{list_rows:#?}"
    );
    assert!(
        carets[0].contains("Song 060") && carets[0].contains("[x]"),
        "caret and checkbox both land on the cursor row: {}",
        carets[0]
    );
}

#[test]
fn a_cover_is_withheld_until_the_highlight_settles_on_its_row() {
    let browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(sample_meta(42)),
    }]);
    // Same geometry as `ready_cover_paints_a_right_column_beside_the_text`.
    const COVER_X: u16 = 68;

    let mut covers = Covers::new();
    let square = covers.picker.new_resize_protocol(gradient_cover(300, 300));
    covers.record_ready(42, Some(square), None);

    // One tick of dwell: the cover is decoded and cached, the highlight has only
    // just landed.
    covers.poll_prefetch(Some(42));
    assert_eq!(
        cover_start_x(&render_grid(&browse, Some(&covers), 90, 30), 1),
        None,
        "a ready cover stays off screen while the highlight is still moving"
    );

    // Four more ticks parked on the row cross the dwell (the first only armed it).
    for _ in 0..4 {
        covers.poll_prefetch(Some(42));
    }
    assert_eq!(
        cover_start_x(&render_grid(&browse, Some(&covers), 90, 30), 1),
        Some(COVER_X),
        "the same ready cover paints once the row has held the highlight"
    );

    // The dwell belongs to the id that earned it. A neighbouring row held long
    // enough to settle, then a move back before the next tick, leaves the
    // counter high for the WRONG id — reachable on any up-then-down scroll.
    for _ in 0..5 {
        covers.poll_prefetch(Some(43));
    }
    assert_eq!(
        cover_start_x(&render_grid(&browse, Some(&covers), 90, 30), 1),
        None,
        "a dwell earned by another row does not license this row's cover"
    );
}

#[test]
fn the_cover_band_carries_the_theme_background() {
    // The band is wiped so reflowed rows cannot run under the artwork, and the
    // wipe has to repaint the app's background: `Clear` alone leaves
    // `Color::Reset`, which is the raw terminal, and the OSC-11 override that
    // would otherwise hide that is emitted only for an `Rgb` theme colour. The
    // gap columns are the tell, since the image never repaints those.
    let browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(sample_meta(42)),
    }]);
    let buf = render_grid(&browse, Some(&covers_both_variants(42)), 90, 30);
    let art_x = cover_start_x(&buf, 1).expect("the cover renders on the first row");
    for x in [art_x - 2, art_x - 1] {
        assert_eq!(
            buf[(x, 1)].bg,
            crate::tui::theme().bg,
            "the gap column at {x} carries the theme background, not bare terminal"
        );
    }
}

#[test]
fn a_cover_narrows_the_width_the_preview_rows_build_against() {
    // The row builder is handed the BESIDE-the-cover width even though the list
    // renders at the full inner width, so an edge-anchored figure lands in one
    // column whichever band its row falls in. The spread's star block is that
    // figure: with a cover it stops at the text column, without one it runs to
    // the pane edge.
    let meta = BeatmapSetMeta {
        beatmaps: vec![spread_diff("Extra", 5.47)],
        ..sample_meta(42)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(meta),
    }]);

    let with_cover = render_grid(&browse, Some(&covers_both_variants(42)), 90, 30);
    let without = render_grid(&browse, None, 90, 30);
    let narrow = star_columns(&with_cover);
    let full = star_columns(&without);
    assert!(
        narrow.len() == 1 && full.len() == 1,
        "one star column either way: {narrow:?} / {full:?}"
    );
    assert!(
        narrow.iter().next() < full.iter().next(),
        "a cover pulls the star block left, to the text column: {narrow:?} vs {full:?}"
    );
}

#[test]
fn a_spread_past_ten_stars_keeps_one_star_column() {
    // `\u{2605}10.24` runs a column wider than `\u{2605}9.87`, so a right-anchored block that
    // measured each rating raw would step the `\u{2605}` left on the 10+ rows and leave
    // only the BAR aligned. 10-star diffs are routine, so this is the common
    // case rather than an edge.
    let meta = BeatmapSetMeta {
        beatmaps: vec![
            spread_diff("Extra", 9.87),
            spread_diff("Black Another", 10.24),
        ],
        ..sample_meta(7)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 7,
        meta: Some(meta),
    }]);
    let buf = render_grid(&browse, None, 90, 30);
    let star_cols = star_columns(&buf);
    assert!(
        star_cols.len() == 1,
        "a spread straddling ten stars pads the narrow ratings into one `\u{2605}` column: {star_cols:?}"
    );
    // Control on the dimension the subject turns on: with no 10+ rating in the
    // block nothing pads, so the tight rendering must still share one column.
    let tight = BeatmapSetMeta {
        beatmaps: vec![spread_diff("Extra", 9.87), spread_diff("Another", 8.12)],
        ..sample_meta(8)
    };
    let tight = browse_with(vec![BrowseRow {
        id: 8,
        meta: Some(tight),
    }]);
    let tight_cols = star_columns(&render_grid(&tight, None, 90, 30));
    assert!(
        tight_cols.len() == 1 && tight_cols != star_cols,
        "a block with no 10+ rating pads nothing, landing one column right: {tight_cols:?} vs {star_cols:?}"
    );
}

#[test]
fn a_cramped_spread_drops_the_meter_and_keeps_the_name_readable() {
    // A cover at 90 cols leaves the preview 22 text columns. Spending 10 of them
    // on a meter left `Se\u{2026}` where the name should be; the `\u{2605}X.XX` rating carries
    // the figure the meter only illustrates, so the meter is what gives way.
    let meta = BeatmapSetMeta {
        beatmaps: vec![spread_diff("Setu's Insane", 5.12)],
        ..sample_meta(42)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(meta),
    }]);

    let cramped = render_grid(&browse, Some(&covers_both_variants(42)), 90, 30);
    let width = cramped.area().width;
    let y = preview_row_of(&cramped, "\u{2605}5.12").expect("the rating survives a cramped pane")
        as u16;
    let row = preview_text_before(&cramped, y, width);
    assert_eq!(
        row.matches('\u{2588}').count() + row.matches('\u{2591}').count(),
        0,
        "a cramped spread drops the meter:\n{row}"
    );
    assert!(
        row.contains("Setu's Insane"),
        "the columns it frees leave the name readable in full:\n{row}"
    );

    // Positive control varying the one dimension the subject turns on — the text
    // width the spread gets — at an identical pane geometry, so `PREVIEW_X` still
    // scopes the read to the preview.
    let roomy = render_grid(&browse, None, 90, 30);
    let width = roomy.area().width;
    let y = preview_row_of(&roomy, "\u{2605}5.12").expect("spread row renders") as u16;
    let row = preview_text_before(&roomy, y, width);
    assert_eq!(
        row.matches('\u{2588}').count() + row.matches('\u{2591}').count(),
        10,
        "a pane with room keeps the whole meter:\n{row}"
    );
}

#[test]
fn rows_below_the_cover_reflow_to_the_full_pane_width() {
    // The cover reserves its narrow text column for its OWN rows only. An artist
    // beside the image still stops at that column; the object-count row, further
    // down than the image reaches, spends the width the image left behind.
    const ARTIST: &str = "A Very Long Artist Name That Overruns The Column";
    let diff = Beatmap {
        count_circles: 410,
        count_sliders: 288,
        count_spinners: 14,
        ..spread_diff("Expert", 5.47)
    };
    let meta = BeatmapSetMeta {
        artist: ARTIST.to_string(),
        beatmaps: vec![diff],
        ..sample_meta(42)
    };
    let browse = browse_with(vec![BrowseRow {
        id: 42,
        meta: Some(meta),
    }]);

    let buf = render_grid(&browse, Some(&covers_both_variants(42)), 90, 30);

    assert!(
        preview_row_of(&buf, "sliders 288").is_some(),
        "the object-count row runs past the cover's text column:\n{}",
        buffer_text(&buf)
    );

    let artist_y = preview_row_of(&buf, "A Very Long Artist").expect("artist renders") as u16;
    let cover_x = cover_start_x(&buf, artist_y).expect("the artist row sits beside the cover");
    for x in [cover_x - 2, cover_x - 1] {
        assert_eq!(
            buf[(x, artist_y)].symbol(),
            " ",
            "the gap before the cover stays clear of text at column {x}"
        );
    }
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
