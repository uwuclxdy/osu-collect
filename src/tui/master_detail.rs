//! Reusable two-pane master-detail browse view: a selection/checkbox list on
//! the left, a read-only preview of the highlighted row on the right, and an
//! optional status line above. It is a pure selector — the download action
//! lives on the source form, not inside the browse.
//!
//! Pure view: the caller builds every row and owns all state (selection,
//! scroll offsets, focus); this module only lays out and draws. Narrow
//! terminals collapse to whichever pane currently holds focus.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{ListItem, Paragraph},
};
use ratatui_image::StatefulImage;
use ratatui_image::protocol::StatefulProtocol;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};

use super::widgets;

/// Which pane holds focus in a descended two-pane browse view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    List,
    Preview,
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
/// Minimum preview inner height before a cover band is carved: below this the
/// band would crowd out the metadata text, so the preview stays text-only.
const MIN_IMAGE_INNER_HEIGHT: u16 = 6;

/// A prepared two-pane master-detail browse view. See the module docs for the
/// layout contract.
///
/// Titles are `Cow<'static, str>` — a `&'static str` panel constant for a fixed
/// section title, or an owned `String` for a proper-noun title (the preview
/// named after the highlighted item). Rows carry `'static` content (built fresh
/// each frame from owned `String`s, never borrowed). `'a` covers only the
/// scroll-offset cells, which genuinely borrow the caller's persisted state.
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
    pub preview_items: Vec<ListItem<'static>>,
    pub preview_selected: Option<usize>,
    pub preview_offset: &'a Cell<usize>,
    /// The highlighted row's ready cover protocol. `Some` carves a top image
    /// band above the preview text (when the pane has room); `None` (the
    /// default for image-less consumers) renders the preview text-only.
    pub preview_image: Option<&'a RefCell<StatefulProtocol>>,
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

    // A ready cover for the highlighted row, and a pane tall enough to spare
    // room for text below it, carve a top image band inside the panel border.
    if let Some(protocol) = view.preview_image {
        let block = widgets::panel_block(
            view.preview_title.clone(),
            view.preview_meta.clone(),
            focused,
            false,
        );
        let inner = block.inner(area);
        if inner.height >= MIN_IMAGE_INNER_HEIGHT {
            frame.render_widget(block, area);
            let [band_area, text_area] = Layout::vertical([
                Constraint::Length(image_band_height(inner.height)),
                Constraint::Min(0),
            ])
            .areas(inner);
            // `StatefulImage` resizes-to-fit the band preserving aspect
            // (letterboxes). `resize_encode_render` mutates the cached protocol,
            // hence the `RefCell` borrow under this immutable-app draw path.
            frame.render_stateful_widget(
                StatefulImage::new(),
                band_area,
                &mut *protocol.borrow_mut(),
            );
            widgets::render_list(
                frame,
                text_area,
                view.preview_items.clone(),
                Some(selected),
                highlight,
                view.preview_offset,
            );
            return;
        }
    }

    widgets::render_scrollable_panel(
        frame,
        area,
        view.preview_title.clone(),
        view.preview_meta.clone(),
        view.preview_items.clone(),
        selected,
        highlight,
        None,
        focused,
        false,
        view.preview_offset,
    );
}

/// Cover-band height: about a third of the preview's inner height, clamped so a
/// tall pane never lets the wide (~2.9:1) card swallow the metadata and a short
/// one still shows a legible strip. The image letterboxes within whatever it gets.
fn image_band_height(inner_height: u16) -> u16 {
    (inner_height / 3).clamp(3, 8)
}

#[cfg(test)]
#[path = "../../tests/unit/tui_master_detail.rs"]
mod tests;
