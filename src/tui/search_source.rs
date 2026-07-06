//! Render for the Get Maps `Search` source: a small form (query input + three
//! mode/status/sort cycle rows, each showing every option with the active one
//! bracketed + the `search` CTA + a status line) that descends into the
//! shared flat browse ([`super::set_browse`]) once results arrive. The source
//! strip is drawn by the Home view; these form rows are pushed into the same
//! Home panel. The results browse (and its status/action line) is rendered by
//! [`super::set_browse`], shared with collection browse&pick.

use crate::app::{HomeField, SearchSource, SearchStatusMsg};
use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets;
use super::{danger, spinner_str, warning};

const LABEL_MODE: &str = "mode";
const LABEL_STATUS: &str = "status";
const LABEL_SORT: &str = "sort";
const LABEL_CTA: &str = "search";

/// Shared label width so the query field and the three chips align their values.
const LABEL_WIDTH: usize = LABEL_STATUS.len();

/// Left title of the results browse (the left pane of the master-detail).
pub const BROWSE_LIST_TITLE: &str = " RESULTS ";

/// Push the search-source FORM rows into the Home panel: the query input, the
/// mode/status/sort cycle rows (every option shown, active one bracketed while
/// focused), the `search` CTA, and a status line for the last run.
pub fn push_form_rows(
    items: &mut widgets::FormItems<HomeField>,
    search: &SearchSource,
    focus: HomeField,
    editing: bool,
    tick: u64,
) {
    items.push_focusable(
        HomeField::SearchQuery,
        widgets::input_item(&search.query, focus == HomeField::SearchQuery, editing, 0),
    );
    items.push(widgets::spacer());

    items.push_focusable(
        HomeField::SearchMode,
        widgets::cycle_item(
            LABEL_MODE,
            search.mode_labels(),
            search.mode_label(),
            focus == HomeField::SearchMode,
            LABEL_WIDTH,
        ),
    );
    items.push_focusable(
        HomeField::SearchStatus,
        widgets::cycle_item(
            LABEL_STATUS,
            search.status_labels(),
            search.status_label(),
            focus == HomeField::SearchStatus,
            LABEL_WIDTH,
        ),
    );
    let sort_labels = search.sort_labels();
    items.push_focusable(
        HomeField::SearchSort,
        widgets::cycle_item(
            LABEL_SORT,
            &sort_labels,
            search.sort_label(),
            focus == HomeField::SearchSort,
            LABEL_WIDTH,
        ),
    );
    items.push(widgets::spacer());

    // A search in flight mirrors the scan CTA: the static `search` label swaps
    // for an inline braille spinner and the button is inert until results land.
    let loading = matches!(search.status_msg, SearchStatusMsg::Loading);
    let cta_label = if loading {
        format!("{} searching", spinner_str(tick).trim())
    } else {
        LABEL_CTA.to_string()
    };
    items.push_focusable(
        HomeField::SearchRun,
        widgets::button_item(&cta_label, focus == HomeField::SearchRun, !loading),
    );

    // `view N maps` reopens the results browse without re-running the query.
    // Always rendered (disabled `view maps` until fresh results load — or once
    // the inputs drift from the loaded ones, so it never offers stale results)
    // so focus always lands on a real row; an empty / failed search adds a
    // one-line outcome beneath it.
    let loaded = search.browse.rows.len();
    let view_current = loaded > 0 && search.results_current();
    let view_label = if view_current {
        widgets::view_maps_label(loaded)
    } else {
        "view maps".to_string()
    };
    items.push_focusable(
        HomeField::SearchBrowse,
        widgets::button_item(&view_label, focus == HomeField::SearchBrowse, view_current),
    );
    if let Some(row) = status_row(&search.status_msg) {
        items.push(row);
    }

    // The download button dispatches the picked results; disabled until at least
    // one set is checked in the results browse. Grouped with the actions above.
    let (download_label, download_enabled) =
        widgets::download_button_label(search.browse.selected_count());
    items.push_focusable(
        HomeField::Download,
        widgets::button_item(
            &download_label,
            focus == HomeField::Download,
            download_enabled,
        ),
    );
}

/// The inline outcome line for an empty or failed search, or `None` otherwise
/// (idle / in flight / results loaded — the loaded case has its own `view N
/// maps` button, the in-flight spinner lives on the search CTA).
fn status_row(msg: &SearchStatusMsg) -> Option<ListItem<'static>> {
    let (label, color) = match msg {
        SearchStatusMsg::Empty => ("no results".to_string(), warning()),
        SearchStatusMsg::Error(reason) => (reason.clone(), danger()),
        SearchStatusMsg::Idle | SearchStatusMsg::Loading | SearchStatusMsg::Ready { .. } => {
            return None;
        }
    };
    Some(ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(label, Style::default().fg(color)),
    ])))
}
