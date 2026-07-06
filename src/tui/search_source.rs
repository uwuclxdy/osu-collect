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
use super::{accent, danger, success, text_dim, text_faint, warning};

const LABEL_MODE: &str = "mode";
const LABEL_STATUS: &str = "status";
const LABEL_SORT: &str = "sort";
const LABEL_CTA: &str = "search";

/// Shared label width so the query field and the three chips align their values.
const LABEL_WIDTH: usize = LABEL_STATUS.len();

/// Left title of the results browse (the left pane of the master-detail).
pub const BROWSE_LIST_TITLE: &str = "results";

/// Push the search-source FORM rows into the Home panel: the query input, the
/// mode/status/sort cycle rows (every option shown, active one bracketed while
/// focused), the `search` CTA, and a status line for the last run.
pub fn push_form_rows(
    items: &mut widgets::FormItems<HomeField>,
    search: &SearchSource,
    focus: HomeField,
    editing: bool,
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

    items.push_focusable(
        HomeField::SearchRun,
        widgets::button_item(LABEL_CTA, focus == HomeField::SearchRun, true),
    );

    if let Some(row) = status_row(&search.status_msg) {
        items.push(row);
    }
    items.push(widgets::spacer());

    // The download button dispatches the picked results; disabled until at least
    // one set is checked in the results browse. Shares the update source's label.
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

/// The status line drawn above the results browse: `X of TOTAL · K selected`,
/// plus a `m load more` hint when another page is available.
pub fn browse_status(search: &SearchSource) -> Line<'static> {
    let shown = search.browse.rows.len();
    let selected = search.browse.selected_count();
    let total = match search.status_msg {
        SearchStatusMsg::Ready { total } => total,
        _ => shown as u64,
    };
    let mut spans = vec![
        Span::styled(shown.to_string(), Style::default().fg(accent()).bold()),
        Span::styled(format!(" of {total}  ·  "), Style::default().fg(text_dim())),
        Span::styled(
            format!("{selected} selected"),
            Style::default().fg(text_dim()),
        ),
    ];
    if search.next_cursor.is_some() {
        spans.push(Span::styled(
            "   ·   m load more",
            Style::default().fg(text_faint()),
        ));
    }
    Line::from(spans)
}

/// The inline status line for the last search, or `None` when idle.
fn status_row(msg: &SearchStatusMsg) -> Option<ListItem<'static>> {
    let (text, color) = match msg {
        SearchStatusMsg::Idle => return None,
        SearchStatusMsg::Loading => ("searching…".to_string(), text_dim()),
        SearchStatusMsg::Ready { total } => {
            let word = if *total == 1 { "result" } else { "results" };
            (format!("{total} {word}"), success())
        }
        SearchStatusMsg::Empty => ("no results".to_string(), warning()),
        SearchStatusMsg::Error(reason) => (reason.clone(), danger()),
    };
    Some(ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(text, Style::default().fg(color)),
    ])))
}
