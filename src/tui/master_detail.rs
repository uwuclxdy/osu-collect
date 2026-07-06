//! Reusable two-pane master-detail browse view: a selection/checkbox list on
//! the left, a read-only preview of the highlighted row on the right, an
//! optional status line above, and an action bar below.
//!
//! Pure view: the caller builds every row and owns all state (selection,
//! scroll offsets, focus); this module only lays out and draws. Narrow
//! terminals collapse to whichever pane currently holds focus.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{ListItem, Paragraph},
};
use std::cell::Cell;

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

/// A prepared two-pane master-detail browse view. See the module docs for the
/// layout contract.
///
/// List/preview titles and rows are `'static` because the panel/list helpers
/// they're drawn through ([`widgets::render_scrollable_panel`],
/// [`widgets::panel_block`]) are `'static`-only throughout this crate (every
/// row is built fresh each frame from owned `String`s, never borrowed). `'a`
/// covers only the scroll-offset cells, which genuinely borrow the caller's
/// persisted state.
pub struct MasterDetail<'a> {
    pub status: Option<Line<'static>>,
    pub list_title: &'static str,
    pub list_items: Vec<ListItem<'static>>,
    pub list_selected: Option<usize>,
    pub list_offset: &'a Cell<usize>,
    pub preview_title: &'static str,
    pub preview_items: Vec<ListItem<'static>>,
    pub preview_selected: Option<usize>,
    pub preview_offset: &'a Cell<usize>,
    pub focused: Pane,
    pub action: Line<'static>,
    pub action_selected: bool,
}

/// Renders the view: status row (if any), the split or single pane body, a
/// blank spacer row, then the action bar.
pub fn render(frame: &mut Frame, area: Rect, view: &MasterDetail<'_>) {
    let (status_area, middle_area, action_area) = split_area(area, view.status.is_some());

    if let (Some(status_area), Some(status)) = (status_area, view.status.clone()) {
        frame.render_widget(Paragraph::new(status), status_area);
    }

    render_panes(frame, middle_area, view);
    render_action(frame, action_area, view);
}

/// Carves `area` into `(status?, middle, action)` top-to-bottom. The bottom 2
/// rows are always reserved for the action bar: a blank spacer, then the
/// action row itself — the spacer is never returned since nothing draws
/// there (the app-wide background already shows through).
fn split_area(area: Rect, has_status: bool) -> (Option<Rect>, Rect, Rect) {
    let mut constraints = Vec::with_capacity(4);
    if has_status {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Min(0));
    constraints.push(Constraint::Length(1)); // spacer
    constraints.push(Constraint::Length(1)); // action
    let chunks = Layout::vertical(constraints).split(area);

    if has_status {
        (Some(chunks[0]), chunks[1], chunks[3])
    } else {
        (None, chunks[0], chunks[2])
    }
}

/// Splits the middle area into list+preview when there's room for both,
/// otherwise renders only the focused pane full-width.
fn render_panes(frame: &mut Frame, area: Rect, view: &MasterDetail<'_>) {
    let wide = area.width >= MIN_SPLIT_WIDTH && area.height >= COMPACT_HEIGHT;
    if wide {
        // Divide before multiply (matches `login_split`) so the intermediate
        // never overflows `u16` on an extreme-width terminal.
        let list_width = (area.width / 5 * 2).clamp(LIST_WIDTH_MIN, LIST_WIDTH_MAX);
        let cols =
            Layout::horizontal([Constraint::Length(list_width), Constraint::Min(0)]).split(area);
        render_list_pane(frame, cols[0], view);
        render_preview_pane(frame, cols[1], view);
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
        view.list_title,
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
    widgets::render_scrollable_panel(
        frame,
        area,
        view.preview_title,
        view.preview_items.clone(),
        view.preview_selected.unwrap_or(0),
        focused,
        None,
        focused,
        false,
        view.preview_offset,
    );
}

fn render_action(frame: &mut Frame, area: Rect, view: &MasterDetail<'_>) {
    let style = if view.action_selected {
        widgets::highlight_style()
    } else {
        Style::default()
    };
    frame.render_widget(Paragraph::new(view.action.clone()).style(style), area);
}

#[cfg(test)]
#[path = "../../tests/unit/tui_master_detail.rs"]
mod tests;
