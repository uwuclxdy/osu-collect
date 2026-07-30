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
//! stops at that column while the image runs beside it and spans the pane's
//! full width from the image's last row down. Preview rows are therefore built
//! by a closure the render calls with the narrower beside-the-cover width, so a
//! row that anchors a figure to its right edge lands in one stable column
//! whichever band it falls in.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect, Size},
    style::Style,
    text::{Line, Span},
    widgets::{Clear, ListItem, Paragraph},
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

/// Builds the preview's field rows at the pane's text width: the column beside
/// the cover, or the full inner width when there is none. Deferred rather than
/// prebuilt because that width is only known once [`cover_layout`] has run,
/// which needs the geometry the render resolves.
pub type PreviewItems<'a> = Box<dyn Fn(u16) -> Vec<ListItem<'static>> + 'a>;

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
    pub list_items: Vec<ListItem<'static>>,
    pub list_selected: Option<usize>,
    pub list_offset: &'a Cell<usize>,
    pub preview_title: Cow<'static, str>,
    pub preview_meta: Option<Line<'static>>,
    pub preview_items: PreviewItems<'a>,
    pub preview_selected: Option<usize>,
    pub preview_offset: &'a Cell<usize>,
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
    widgets::render_scrollable_panel(
        frame,
        area,
        view.list_title.clone(),
        view.list_meta.clone(),
        view.list_items.clone(),
        view.list_selected.unwrap_or(0),
        // Highlight tint only while this pane owns focus AND a row is actually
        // selected — parked on the action bar (`list_selected == None`) nothing
        // in the list is highlighted, so the tint doesn't double up with the
        // action bar's. The row still scrolls into view either way (see
        // `render_list`'s doc contract).
        focused && view.list_selected.is_some(),
        None,
        focused,
        true,
        view.list_offset,
    );
}

fn render_preview_pane(frame: &mut Frame, area: Rect, view: &MasterDetail<'_>) {
    let focused = view.focused == Pane::Preview;
    // A read-only preview (`preview_selected == None`, e.g. the set browse) is a
    // scroll target, not a selectable list, so it takes no `bg_hover` selection
    // band even while descended — only a preview that marks a row highlights it.
    let highlight = focused && view.preview_selected.is_some();
    let selected = view.preview_selected.unwrap_or(0);

    let block = widgets::panel_block(
        view.preview_title.clone(),
        view.preview_meta.clone(),
        focused,
        false,
    );
    let inner = block.inner(area);

    // A ready cover takes a column at the pane's right edge, the metadata text
    // keeping everything left of it. `cover_layout` prefers the wide variant, then
    // collapses to the smaller square (still shown) to spare the title before it
    // ever wraps — the title wraps to two lines only when even the collapsed
    // column's text can't hold it.
    if let Some(cover) = view.preview_image
        && let Some((text_area, cover_area, protocol)) =
            cover_layout(inner, view.preview_lead.as_ref(), cover.square, cover.wide)
    {
        frame.render_widget(block, area);
        let items = with_lead(
            view.preview_lead.as_ref(),
            text_area.width,
            (view.preview_items)(text_area.width),
        );
        // Rows run the pane's full width and the image's own rows are cleared
        // back to the text column below, so everything under the image reflows
        // into the space the image never occupied.
        widgets::render_list(
            frame,
            inner,
            items,
            Some(selected),
            highlight,
            view.preview_offset,
        );
        frame.render_widget(Clear, cover_band(inner, text_area, cover_area));
        // `resize_encode_render` mutates the cached protocol, hence the `RefCell`
        // borrow under this immutable-app draw path.
        frame.render_stateful_widget(
            StatefulImage::new(),
            cover_area,
            &mut *protocol.borrow_mut(),
        );
        return;
    }

    // Text-only (no cover, or the cover collapsed to spare the title): same block,
    // the title lead prepended at the full inner width — one line if it fits, two
    // if not. Mirrors `render_scrollable_panel` (block + `render_list` + scrollbar)
    // minus the text-cursor pass a read-only preview never needs.
    frame.render_widget(block, area);
    let items = with_lead(
        view.preview_lead.as_ref(),
        inner.width,
        (view.preview_items)(inner.width),
    );
    widgets::render_list(
        frame,
        inner,
        items,
        Some(selected),
        highlight,
        view.preview_offset,
    );
}

/// The columns the cover owns for the rows it spans: the gap plus the image
/// itself, from the pane's top down to the image's last row. Wiping it is what
/// keeps a row that overruns the text column from running under the image while
/// rows below the image keep the full width.
fn cover_band(inner: Rect, text_area: Rect, cover_area: Rect) -> Rect {
    Rect {
        x: text_area.right(),
        y: inner.y,
        width: inner.width - text_area.width,
        height: cover_area.height,
    }
}

/// Whether the title lead fits on a single line at `width` (so the cover may
/// stay). No lead trivially fits — an id-only row keeps its cover.
fn title_fits_one_line(lead: Option<&PreviewLead>, width: u16) -> bool {
    lead.is_none_or(|lead| display_width(&lead.text) <= width as usize)
}

/// Prepend the wrapped title lead (at most [`MAX_TITLE_LINES`] lines at `width`)
/// above the field rows. `None` lead returns the field rows unchanged.
fn with_lead(
    lead: Option<&PreviewLead>,
    width: u16,
    items: Vec<ListItem<'static>>,
) -> Vec<ListItem<'static>> {
    let Some(lead) = lead else { return items };
    let mut out = Vec::with_capacity(items.len() + MAX_TITLE_LINES);
    for line in wrap_to_lines(&lead.text, width as usize, MAX_TITLE_LINES) {
        out.push(ListItem::new(Line::from(Span::styled(line, lead.style))));
    }
    out.extend(items);
    out
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
/// and the column every edge-anchored figure aligns to, not the pane's text as
/// a whole.
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
