//! Reusable two-pane master-detail browse view: a selection/checkbox list on
//! the left, a read-only preview of the highlighted row on the right, and an
//! optional status line above. It is a pure selector — the download action
//! lives on the source form, not inside the browse.
//!
//! Pure view: the caller builds every row and owns all state (selection,
//! scroll offsets, focus); this module only lays out and draws. Narrow
//! terminals collapse to whichever pane currently holds focus.
//!
//! A cover takes a right-hand column for its own rows only: the preview text
//! stops at that column while the image runs beside it, then spans the pane's
//! full width from the first row BELOW the image down. Preview rows are
//! therefore built by a closure the render calls with both widths plus the row
//! index where the band widens ([`PreviewWidths`]), so a row clear of the image
//! spends the columns the image left behind instead of a budget it no longer
//! pays for. A multi-row block that would straddle that index waits for it
//! instead ([`PreviewWidths::pushdown`]).
//!
//! The cover is drawn only while the preview sits at its top row. No image
//! protocol clips, so a scrolled pane cannot slide text under pinned artwork:
//! it renders text-only at the full width until it is scrolled back up.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect, Size},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, ListItem, Paragraph},
};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};

use super::widgets;

/// Which pane holds focus in a descended two-pane browse view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    List,
    Preview,
}

/// The highlighted row's ready cover protocols, one per aspect. The preview
/// always seats the cover in a right-hand column, text to its left; it renders
/// the wide `card@2x` once that column has grown past [`WIDE_COVER_WIDTH`] and
/// the variant is loaded, otherwise the square `list@2x`. Either field is `None`
/// when that variant is unfetched or failed.
#[derive(Clone, Copy)]
pub struct PreviewCover<'a> {
    pub square: Option<&'a RefCell<StatefulProtocol>>,
    pub wide: Option<&'a RefCell<StatefulProtocol>>,
}

/// The preview's lead line (the set title), rendered above `preview_items`. It
/// wraps to at most [`MAX_TITLE_LINES`] at the render width, but only ever needs
/// to when there's no cover: a title that won't fit on one line beside the cover
/// collapses the cover first (see [`render_preview_pane`]), so it wraps only when
/// even the full width can't hold it. `None` renders `preview_items` alone.
pub struct PreviewLead {
    pub text: String,
    pub style: Style,
}

/// Builds the preview's field rows at the pane's text widths. Deferred rather
/// than prebuilt because those are only known once [`cover_layout`] has run,
/// which needs the geometry the render resolves.
pub type PreviewItems<'a> = Box<dyn Fn(PreviewWidths) -> Vec<ListItem<'static>> + 'a>;

/// The two text widths a preview row can have, and the row index where it
/// changes: rows the cover spans stop at `beside`, the first row BELOW the
/// image's last one and everything under it get the pane's `full` inner width
/// (see [`cover_band`]). `seam` counts in the builder's own row indices, the lead
/// lines already discounted, so a builder can size each row off `rows.len()` as
/// it pushes.
///
/// A pane with no cover reports the same width for both and a `seam` of 0.
#[derive(Clone, Copy)]
pub struct PreviewWidths {
    pub beside: u16,
    pub full: u16,
    pub seam: usize,
    /// Rows between the cover's last one and the pane's bottom: what a block
    /// [`Self::pushdown`] holds back has to fit into. The whole inner height when
    /// there is no cover.
    pub below: usize,
}

impl PreviewWidths {
    /// The width the row at builder-local `index` renders in.
    ///
    /// A multi-row block that anchors a figure to a shared column must ask for
    /// its FIRST row: sizing the whole block to one band keeps one column, where
    /// asking per row would step the figure out mid-block as the band widens.
    pub fn at(self, index: usize) -> u16 {
        if index < self.seam {
            self.beside
        } else {
            self.full
        }
    }

    /// Blank rows to insert before a `rows`-row block starting at builder-local
    /// `start`, so it clears the cover and lays out at [`Self::full`] rather than
    /// squeezing a whole block into the narrow band for the sake of its first row
    /// or two.
    ///
    /// Zero when the block already clears the image, and zero when the held-back
    /// block would not fit below it: on a pane the cover nearly fills, pushing it
    /// down costs the rows it was going to be read in, which beats the columns
    /// waiting would win.
    ///
    /// The fit it checks is the BLOCK's. Rows the caller pushes after the block
    /// move down by the return value too, so an already-overrunning preview puts
    /// that many more rows past its bottom edge — a page key away, not lost.
    /// Guarding the whole tail instead would forfeit the width on exactly the
    /// previews that are overrunning anyway.
    pub fn pushdown(self, start: usize, rows: usize) -> usize {
        if start >= self.seam || rows > self.below {
            return 0;
        }
        self.seam - start
    }
}

/// Below this width a side-by-side split would crush the list column, so the
/// view falls back to single-pane (mirrors `login_split`'s threshold).
const MIN_SPLIT_WIDTH: u16 = 60;
/// Below this height there isn't room for a split plus the status/action rows.
const COMPACT_HEIGHT: u16 = 14;
/// List-pane width bounds: wide enough to read a row, never so wide it
/// crowds out the preview on an ultra-wide terminal.
const LIST_WIDTH_MIN: u16 = 28;
const LIST_WIDTH_MAX: u16 = 52;
/// Minimum cover-column width; below this the artwork reads as noise, so a
/// cramped pane drops the cover and keeps the text.
const COVER_WIDTH_MIN: u16 = 12;
/// Once the cover column reaches this width the wide `card@2x` reads better than
/// the square `list@2x`; below it the square wins. The column grows with the
/// pane (see [`cover_width_allowance`]), so this doubles as the "enough space"
/// threshold that swaps in the wide variant.
const WIDE_COVER_WIDTH: u16 = 26;
/// Blank columns between the metadata text and the cover.
const COVER_GAP: u16 = 2;
/// The metadata text's floor: the widest static kv key (`favourites`) plus its
/// separator and a readable value. A pane that can't seat this beside a cover
/// drops the cover instead of squeezing the text.
const MIN_TEXT_WIDTH: u16 = 22;
/// Minimum preview inner height before a cover is carved: shorter than this the
/// image is a smear rather than artwork.
const MIN_IMAGE_INNER_HEIGHT: u16 = 4;
/// The set title's line budget in the preview: enough that a long title beside
/// the cover isn't cut to one truncated line, capped so it stays compact.
const MAX_TITLE_LINES: usize = 2;

/// A prepared two-pane master-detail browse view. See the module docs for the
/// layout contract.
///
/// Titles are `Cow<'static, str>` — a `&'static str` panel constant for a fixed
/// section title, or an owned `String` for a proper-noun title (the preview
/// named after the highlighted item). Rows carry `'static` content (built fresh
/// each frame from owned `String`s, never borrowed). `'a` covers the
/// scroll-offset cells and the [`PreviewItems`] builder, which genuinely borrow
/// the caller's state.
///
/// Each pane takes an optional title-right `*_meta` line (a short count / state
/// in the top border break — see [`widgets::panel_block`]).
pub struct MasterDetail<'a> {
    pub status: Option<Line<'static>>,
    pub list_title: Cow<'static, str>,
    pub list_meta: Option<Line<'static>>,
    /// Total row count, and a builder for the slice the viewport resolves to.
    /// Deferred like [`PreviewItems`], for a different reason: a flat browse can
    /// hold thousands of rows and only the visible handful reaches the screen.
    pub list_len: usize,
    pub list_items: widgets::ListRows<'a>,
    pub list_selected: Option<usize>,
    pub list_offset: &'a Cell<usize>,
    pub preview_title: Cow<'static, str>,
    pub preview_meta: Option<Line<'static>>,
    pub preview_items: PreviewItems<'a>,
    pub preview_selected: Option<usize>,
    pub preview_offset: &'a Cell<usize>,
    /// Written by the render: the largest offset `preview_offset` can hold at the
    /// pane's current size. A caller jumping to the bottom sets the offset from
    /// this rather than from a value it hopes is past the end, so the next page
    /// key steps back from a real row index.
    pub preview_max_offset: &'a Cell<usize>,
    /// The highlighted row's ready cover protocols. `Some` seats a right-hand
    /// image column (square or wide, per pane size) when there's room; `None`
    /// (the default for image-less consumers) renders the preview text-only.
    pub preview_image: Option<PreviewCover<'a>>,
    /// The preview's lead (the set title), drawn above `preview_items`. Kept on
    /// one line beside a cover; the cover collapses before this wraps. `None`
    /// renders `preview_items` alone.
    pub preview_lead: Option<PreviewLead>,
    pub focused: Pane,
}

/// Renders the view: status row (if any), then the split or single pane body.
pub fn render(frame: &mut Frame, area: Rect, view: &MasterDetail<'_>) {
    let (status_area, middle_area) = split_area(area, view.status.is_some());

    if let (Some(status_area), Some(status)) = (status_area, view.status.clone()) {
        frame.render_widget(Paragraph::new(status), status_area);
    }

    render_panes(frame, middle_area, view);
}

/// Carves `area` into `(status?, middle)` top-to-bottom: an optional 1-row
/// status line, then the panes fill the rest.
fn split_area(area: Rect, has_status: bool) -> (Option<Rect>, Rect) {
    if !has_status {
        return (None, area);
    }
    let [status_area, middle_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    (Some(status_area), middle_area)
}

/// Splits the middle area into list+preview when there's room for both,
/// otherwise renders only the focused pane full-width.
fn render_panes(frame: &mut Frame, area: Rect, view: &MasterDetail<'_>) {
    let wide = area.width >= MIN_SPLIT_WIDTH && area.height >= COMPACT_HEIGHT;
    if wide {
        // Divide before multiply (matches `login_split`) so the intermediate
        // never overflows `u16` on an extreme-width terminal.
        let list_width = (area.width / 5 * 2).clamp(LIST_WIDTH_MIN, LIST_WIDTH_MAX);
        let [list_area, preview_area] =
            Layout::horizontal([Constraint::Length(list_width), Constraint::Min(0)]).areas(area);
        render_list_pane(frame, list_area, view);
        render_preview_pane(frame, preview_area, view);
    } else {
        match view.focused {
            Pane::List => render_list_pane(frame, area, view),
            Pane::Preview => render_preview_pane(frame, area, view),
        }
    }
}

fn render_list_pane(frame: &mut Frame, area: Rect, view: &MasterDetail<'_>) {
    let focused = view.focused == Pane::List;
    let block = widgets::panel_block(
        view.list_title.clone(),
        view.list_meta.clone(),
        focused,
        true,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // Mirrors `render_scrollable_panel` (block + list + scrollbar) minus the
    // text-cursor pass a browse list never needs, over the windowed builder.
    widgets::render_windowed_list(
        frame,
        inner,
        view.list_len,
        &view.list_items,
        Some(view.list_selected.unwrap_or(0)),
        // Highlight tint only while this pane owns focus AND a row is actually
        // selected — parked on the action bar (`list_selected == None`) nothing
        // in the list is highlighted, so the tint doesn't double up with the
        // action bar's. The row still scrolls into view either way (see
        // `render_list`'s doc contract).
        focused && view.list_selected.is_some(),
        view.list_offset,
    );
}

fn render_preview_pane(frame: &mut Frame, area: Rect, view: &MasterDetail<'_>) {
    let focused = view.focused == Pane::Preview;
    // A read-only preview (`preview_selected == None`, e.g. the set browse) is a
    // scroll target, not a selectable list, so it takes no `bg_hover` selection
    // band even while descended — only a preview that marks a row highlights it.
    let highlight = focused && view.preview_selected.is_some();
    // A blurred preview is pinned to its top row: it is a read-only detail of
    // whatever the list highlights, and that highlight moves out from under it.
    if !focused {
        view.preview_offset.set(0);
    }
    let scrolled = view.preview_offset.get() > 0;

    let block = widgets::panel_block(
        view.preview_title.clone(),
        view.preview_meta.clone(),
        focused,
        false,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // A ready cover takes a column at the pane's right edge, the metadata text
    // keeping everything left of it. `cover_layout` prefers the wide variant, then
    // collapses to the smaller square (still shown) to spare the title before it
    // ever wraps — the title wraps to two lines only when even the collapsed
    // column's text can't hold it.
    let cover = view.preview_image.and_then(|cover| {
        cover_layout(inner, view.preview_lead.as_ref(), cover.square, cover.wide)
    });
    let cover_rows = cover.map_or(0, |(_, cover_area, _)| cover_area.height);
    let text_width = cover.map_or(inner.width, |(text_area, ..)| text_area.width);

    let mut items = preview_rows(view, inner, cover_rows, text_width, scrolled);
    // The row count is the same at every offset (see [`preview_rows`]), so the
    // clamp is exact here rather than a frame behind — and resolving it BEFORE
    // the cover decision is what brings the artwork back in the same frame a
    // taller pane leaves nothing to scroll. A key-driven loop may draw no other.
    let max_offset = items.len().saturating_sub(inner.height as usize);
    let offset = view.preview_offset.get().min(max_offset);
    view.preview_offset.set(offset);
    // Where the bottom is, for a caller that wants to jump to it. Reported rather
    // than left for the caller to guess: a sentinel offset "past the end" reads as
    // the bottom to the clamp but not to a page key, which would then subtract a
    // page from the sentinel and land back on the bottom.
    view.preview_max_offset.set(max_offset);
    if scrolled && offset == 0 {
        // Clamped back to the top, so a cover is drawn after all and the rows it
        // sits beside have to leave it its column. The only frame built twice.
        let narrow = preview_rows(view, inner, cover_rows, text_width, false);
        // Both builds exist only here, so this is the one place the count
        // invariance `preview_rows` rests on can be checked at all.
        debug_assert_eq!(
            narrow.len(),
            items.len(),
            "a preview row builder changed its ROW COUNT with the width, which \
             strands the rows a pushdown pad puts past the pane's bottom edge"
        );
        items = narrow;
    }
    let scrolled = offset > 0;
    // Rows run the pane's full width and the image's own rows are cleared back to
    // the text column below, so everything under the image reflows into the space
    // the image never occupied.
    widgets::render_list(
        frame,
        inner,
        items,
        view.preview_selected,
        highlight,
        view.preview_offset,
    );

    // The cover belongs to the preview's TOP. It is an overlay pinned to the
    // pane's first rows and no image protocol clips, so a scrolled preview drops
    // it rather than sliding rows under artwork.
    let Some((_, cover_area, protocol)) = cover.filter(|_| !scrolled) else {
        return;
    };
    // `Clear` alone resets the band to `Color::Reset`, which is the raw terminal
    // background rather than the app's: `tui::draw` paints the theme bg over every
    // frame, and the OSC-11 override that would otherwise cover for it is emitted
    // only for an `Rgb` theme colour. Repainting the band keeps the gap columns
    // (which the image itself never touches) and a failed encode from showing
    // through.
    let band = cover_band(inner, cover_area);
    frame.render_widget(Clear, band);
    frame.render_widget(Block::default().bg(super::bg()), band);
    // `resize_encode_render` mutates the cached protocol, hence the `RefCell`
    // borrow under this immutable-app draw path.
    frame.render_stateful_widget(
        StatefulImage::new(),
        cover_area,
        &mut *protocol.borrow_mut(),
    );
}

/// The preview's rows for one frame. `scrolled` widens only the rows a cover
/// would sit beside — the seam, the pushdown and therefore the ROW COUNT are the
/// cover's geometry at every offset, so scrolling is a pure window move.
///
/// A count that shrank once scrolled would strand the rows a [`PreviewWidths::pushdown`]
/// pad pushes past the bottom edge: the clamp would pull the offset back to
/// exactly where the pad reappears, and no key could reach them. The lead is
/// built at the beside-the-cover width for the same reason — a title that wraps
/// at one width and not the other shifts every row index below it mid-scroll.
fn preview_rows(
    view: &MasterDetail<'_>,
    inner: Rect,
    cover_rows: u16,
    text_width: u16,
    scrolled: bool,
) -> Vec<ListItem<'static>> {
    let mut items = lead_items(view.preview_lead.as_ref(), text_width);
    items.extend((view.preview_items)(PreviewWidths {
        beside: if scrolled { inner.width } else { text_width },
        full: inner.width,
        seam: seam_row(cover_rows, items.len()),
        below: inner.height.saturating_sub(cover_rows) as usize,
    }));
    items
}

/// The columns the cover owns for the rows it spans: the gap plus the image
/// itself, from the pane's top down to the image's last row. Wiping it is what
/// keeps a row that overruns the text column from running under the image while
/// rows below the image keep the full width. Derived from `cover_area` alone so
/// the two rects cannot drift apart, and clamped to `inner`: `Clear` intersects
/// with the whole frame buffer, so an over-tall fitted size would otherwise wipe
/// the panel's bottom border.
fn cover_band(inner: Rect, cover_area: Rect) -> Rect {
    Rect {
        x: cover_area.x.saturating_sub(COVER_GAP),
        y: inner.y,
        width: cover_area.width + COVER_GAP,
        height: cover_area.height.min(inner.height),
    }
}

/// Whether the title lead fits on a single line at `width` (so the cover may
/// stay). No lead trivially fits — an id-only row keeps its cover.
fn title_fits_one_line(lead: Option<&PreviewLead>, width: u16) -> bool {
    lead.is_none_or(|lead| display_width(&lead.text) <= width as usize)
}

/// The builder-row index where the cover's band ends: the image spans
/// `cover_rows` screen rows down from the pane's top, and `lead_rows` of them
/// belong to the title rather than to the builder. Saturating, so a cover
/// shorter than the lead leaves the builder no beside-the-cover rows at all.
///
/// There is no scroll term: a cover is drawn only while the pane sits at offset
/// 0 (see [`render_preview_pane`]), so a screen row and a builder row differ by
/// the lead alone.
fn seam_row(cover_rows: u16, lead_rows: usize) -> usize {
    (cover_rows as usize).saturating_sub(lead_rows)
}

/// The title lead's rows, wrapped to at most [`MAX_TITLE_LINES`] lines at
/// `width`. Empty for no lead (an id-only row), which is also what makes the
/// caller's `items.len()` a straight lead-line count.
fn lead_items(lead: Option<&PreviewLead>, width: u16) -> Vec<ListItem<'static>> {
    let Some(lead) = lead else { return Vec::new() };
    wrap_to_lines(&lead.text, width as usize, MAX_TITLE_LINES)
        .into_iter()
        .map(|line| ListItem::new(Line::from(Span::styled(line, lead.style))))
        .collect()
}

/// Resolve the cover layout: `(text, cover, protocol)`, or `None` for text-only
/// (the pane can't seat any cover beside readable text).
///
/// Priority is **collapse-before-wrap**: prefer the wide `card@2x` in a big
/// column, but only when the pane has room for it AND the title fits on one line
/// beside it. Otherwise collapse to the square `list@2x` in a NARROWER column —
/// the cover is still shown, and the wider text it leaves keeps a longer title on
/// one line; the title wraps to two only when even that column's text can't hold
/// it. A still-fetching preferred variant falls through to the other so the cover
/// never blanks while both are in flight.
fn cover_layout<'a>(
    inner: Rect,
    lead: Option<&PreviewLead>,
    square: Option<&'a RefCell<StatefulProtocol>>,
    wide: Option<&'a RefCell<StatefulProtocol>>,
) -> Option<(Rect, Rect, &'a RefCell<StatefulProtocol>)> {
    if inner.height < MIN_IMAGE_INNER_HEIGHT {
        return None;
    }

    // Wide when the pane has room for it and the title fits on one line beside it.
    if let Some(w) = wide
        && let Some(allowance) = wide_cover_width(inner.width)
        && let Some((text_area, cover_area)) = place_cover(inner, w, allowance)
        && title_fits_one_line(lead, text_area.width)
    {
        return Some((text_area, cover_area, w));
    }

    // Collapse to the smaller square (still shown). Prefer the square protocol;
    // fall back to the wide one when only it has loaded.
    let collapsed = square.or(wide)?;
    let allowance = square_cover_width(inner.width)?;
    let (text_area, cover_area) = place_cover(inner, collapsed, allowance)?;
    Some((text_area, cover_area, collapsed))
}

/// Fit `protocol` into a right-anchored column of at most `allowance` width,
/// returning `(text, cover)` — the text taking everything left of the fitted
/// image. `None` if the image rounds away to nothing.
///
/// The text rect spans the whole pane height while the cover rect stops at the
/// image's last row: rows past that one are drawn at the pane's full width (see
/// [`cover_band`]), so the narrow text width bounds the rows beside the image
/// alone, not the pane's text as a whole.
///
/// The cover rect is sized to what the image will *actually* occupy rather than
/// the allowance it was offered: [`Resize::Fit`] letterboxes within its area and
/// anchors top-left, so a fixed column would float the artwork mid-pane with dead
/// space against the border. Asking the protocol for its own fitted size (it
/// accounts for the terminal's font aspect, and never upscales past the source)
/// puts the right edge exactly on the border. Since the fitted image can only be
/// narrower than the allowance, whose formula already reserves [`MIN_TEXT_WIDTH`],
/// the text width never needs re-checking.
fn place_cover(
    inner: Rect,
    protocol: &RefCell<StatefulProtocol>,
    allowance: u16,
) -> Option<(Rect, Rect)> {
    let fitted = protocol
        .borrow()
        .size_for(Resize::Fit(None), Size::new(allowance, inner.height));
    if fitted.width == 0 || fitted.height == 0 {
        return None;
    }
    let text_width = inner.width.checked_sub(fitted.width + COVER_GAP)?;
    let text_area = Rect {
        width: text_width,
        ..inner
    };
    let cover_area = Rect {
        x: inner.right() - fitted.width,
        y: inner.y,
        width: fitted.width,
        height: fitted.height,
    };
    Some((text_area, cover_area))
}

/// The wide cover column: up to three fifths of the inner width, floored so the
/// text keeps [`MIN_TEXT_WIDTH`]. `None` until it reaches [`WIDE_COVER_WIDTH`] —
/// below that the pane has no room for a wide cover, so the square is used.
fn wide_cover_width(inner_width: u16) -> Option<u16> {
    // Divide before multiply (matches `render_panes`) so the intermediate never
    // overflows `u16` on an extreme-width terminal.
    cover_column(inner_width, 3).filter(|&w| w >= WIDE_COVER_WIDTH)
}

/// The square (collapsed) cover column: up to two fifths of the inner width,
/// narrower than the wide column so collapsing to it frees room for the title.
/// `None` when even this can't reach [`COVER_WIDTH_MIN`] beside readable text.
fn square_cover_width(inner_width: u16) -> Option<u16> {
    cover_column(inner_width, 2).filter(|&w| w >= COVER_WIDTH_MIN)
}

/// `fifths`/5 of the inner width, capped at what leaves the text its
/// [`MIN_TEXT_WIDTH`] floor. `None` when there's no room for even the text.
fn cover_column(inner_width: u16, fifths: u16) -> Option<u16> {
    let spare = inner_width.checked_sub(COVER_GAP + MIN_TEXT_WIDTH)?;
    Some((inner_width / 5 * fifths).min(spare))
}

/// Greedy word-wrap `text` to at most `max_lines` lines of display width
/// `width`, ellipsising the final line when the text overruns the budget. Empty
/// for a zero `width` or `max_lines`.
///
/// ratatui exposes no wrap-point data (`Paragraph::wrap` renders wrapped text
/// but returns none), so wrapping-as-data stays custom (`ratatui-pro`
/// `limitations.md`). Widths are display columns via [`Span::width`], so
/// full-width (CJK) titles wrap by cells, not bytes.
fn wrap_to_lines(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let mut lines = greedy_wrap(text, width);
    if lines.len() <= max_lines {
        return lines;
    }
    lines.truncate(max_lines);
    if let Some(last) = lines.last_mut() {
        *last = truncate_with_ellipsis(last, width);
    }
    lines
}

/// Wrap into as many lines as needed, each within `width`. A single word wider
/// than `width` is hard-split at a char boundary.
fn greedy_wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        for piece in hard_split(word, width) {
            push_piece(&piece, width, &mut lines, &mut current);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Append one already-`width`-bounded `piece` to the wrap in progress: extend
/// the current line when it still fits (with a separating space), else flush the
/// current line and start a new one with `piece`.
fn push_piece(piece: &str, width: usize, lines: &mut Vec<String>, current: &mut String) {
    if current.is_empty() {
        current.push_str(piece);
    } else if display_width(current) + 1 + display_width(piece) <= width {
        current.push(' ');
        current.push_str(piece);
    } else {
        lines.push(std::mem::replace(current, piece.to_string()));
    }
}

/// Split `word` into consecutive pieces each within `width` display columns; a
/// word that already fits returns `[word]`.
fn hard_split(word: &str, width: usize) -> Vec<String> {
    if display_width(word) <= width {
        return vec![word.to_string()];
    }
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    for ch in word.chars() {
        if !chunk.is_empty() && display_width(&chunk) + char_width(ch) > width {
            chunks.push(std::mem::take(&mut chunk));
        }
        chunk.push(ch);
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

/// Trim `s` to `width` display columns and append `…` as a truncation marker
/// (dropping trailing columns to make room for it). Empty for a zero `width`.
fn truncate_with_ellipsis(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let budget = width - 1;
    let mut out = String::new();
    for ch in s.chars() {
        if display_width(&out) + char_width(ch) > budget {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

/// Display width of `s` in terminal columns (unicode-aware), via ratatui's own
/// span measurement.
fn display_width(s: &str) -> usize {
    Span::raw(s).width()
}

/// Display width of a single char.
fn char_width(ch: char) -> usize {
    display_width(ch.encode_utf8(&mut [0u8; 4]))
}

#[cfg(test)]
#[path = "../../tests/unit/tui_master_detail.rs"]
mod tests;
