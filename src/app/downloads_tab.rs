//! Downloads-tab view state + row model.
//!
//! The tab is a two-pane master-detail over every run: live pages (active,
//! then settled-retained newest-first) followed by persisted past-run records.
//! This module owns only the list cursor and pane focus; the rows are built
//! per frame by `App::downloads_rows` from `App.downloads` + `App.history`.

use crate::app::collection::CollectionPage;
use crate::app::download_history::HistoryRecord;
use std::cell::Cell;

/// Rows moved per page-scroll keystroke on the run list.
const PAGE_STEP: usize = 10;

/// One row of the Downloads list. A `Page` is a live run this session (active
/// or settled-retained, distinguished by its stage); a `Record` is a persisted
/// past run (a previous session, or an evicted / cancelled page).
pub enum DownloadsRow<'a> {
    Page(&'a CollectionPage),
    Record(&'a HistoryRecord),
}

/// Cursor + pane focus for the Downloads tab. `enter` descends into the
/// preview (where the download-control keys live), `esc`/`←` ascend.
#[derive(Debug, Default)]
pub struct DownloadsTab {
    /// Cursor row in the run list; the preview auto-follows it.
    pub selected: usize,
    /// Whether the preview pane holds focus (descended).
    pub preview_focused: bool,
    /// List scroll offset, clamped by the renderer (interior mutability
    /// mirrors the other browse offsets).
    pub list_offset: Cell<usize>,
}

impl DownloadsTab {
    /// Step the cursor up one row, wrapping past the top to the last row (the
    /// selector-list convention shared with the browses / form fields).
    pub fn select_prev(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        self.selected = (self.selected.min(len - 1) + len - 1) % len;
    }

    /// Step the cursor down one row, wrapping past the last row back to the top.
    pub fn select_next(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        self.selected = (self.selected.min(len - 1) + 1) % len;
    }

    /// Page the cursor up/down (`Ctrl+u` / `Ctrl+d`), clamped — matches the
    /// download page's own paging step.
    pub fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(PAGE_STEP);
    }

    pub fn page_down(&mut self, len: usize) {
        self.selected = (self.selected + PAGE_STEP).min(len.saturating_sub(1));
    }

    /// Keep the cursor on a real row after rows are removed; an empty list
    /// also drops the preview focus (there is nothing to preview).
    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
            self.preview_focused = false;
        } else {
            self.selected = self.selected.min(len - 1);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/downloads_tab.rs"]
mod tests;
