//! Render for the Get Maps `Filter` source: the flat nzbasic attribute form
//! (preset seed-macro + special/mode/status chips, min-max range inputs, text
//! rows, sort/limit knobs, the `filter` CTA) that descends into the shared flat
//! browse ([`super::set_browse`]) once results arrive. The source strip is
//! drawn by the Home view; these form rows are pushed into the same Home panel.

use crate::app::{FindSource, FindStatusMsg, HomeField, InputField};
use crate::utils::format_bytes;
use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets;
use super::{danger, spinner_str, text_faint, warning};

const LABEL_PRESET: &str = "preset";
const LABEL_SPECIAL: &str = "special";
const LABEL_MODE: &str = "mode";
const LABEL_STATUS: &str = "status";
const LABEL_SORT: &str = "sort";
const LABEL_CTA: &str = "filter";

/// Shared label width so chips and inputs align their values. `pub(crate)` so
/// the Home view computes the caret column with the same offset.
pub(crate) const LABEL_WIDTH: usize = LABEL_SPECIAL.len();

/// Credit + risk line: the backing database is a community-hosted free
/// instance, so the copy names the source and warns it can be unavailable.
const CREDIT: &str = "data by nzbasic (batch beatmap downloader) · community-hosted";

/// Push the filter-source FORM rows into the Home panel.
pub fn push_form_rows(
    items: &mut widgets::FormItems<HomeField>,
    filter: &FindSource,
    focus: HomeField,
    editing: bool,
    tick: u64,
) {
    items.push_focusable(
        HomeField::FilterPreset,
        widgets::cycle_item(
            LABEL_PRESET,
            filter.preset_labels(),
            filter.preset_label(),
            focus == HomeField::FilterPreset,
            LABEL_WIDTH,
        ),
    );
    items.push_focusable(
        HomeField::FilterSpecial,
        widgets::cycle_item(
            LABEL_SPECIAL,
            filter.special_labels(),
            filter.special_label(),
            focus == HomeField::FilterSpecial,
            LABEL_WIDTH,
        ),
    );
    items.push_focusable(
        HomeField::FilterMode,
        widgets::cycle_item(
            LABEL_MODE,
            filter.mode_labels(),
            filter.mode_label(),
            focus == HomeField::FilterMode,
            LABEL_WIDTH,
        ),
    );
    items.push_focusable(
        HomeField::FilterStatus,
        widgets::cycle_item(
            LABEL_STATUS,
            filter.status_labels(),
            filter.status_label(),
            focus == HomeField::FilterStatus,
            LABEL_WIDTH,
        ),
    );
    items.push(widgets::spacer());

    for (field, input) in [
        (HomeField::FilterStars, &filter.stars),
        (HomeField::FilterAr, &filter.ar),
        (HomeField::FilterCs, &filter.cs),
        (HomeField::FilterOd, &filter.od),
        (HomeField::FilterHp, &filter.hp),
        (HomeField::FilterBpm, &filter.bpm),
        (HomeField::FilterLength, &filter.length),
        (HomeField::FilterArtist, &filter.artist),
        (HomeField::FilterCreator, &filter.creator),
        (HomeField::FilterTitle, &filter.title),
    ] {
        push_input(items, field, input, focus, editing);
    }
    items.push(widgets::spacer());

    let sort_labels = filter.sort_labels();
    items.push_focusable(
        HomeField::FilterSort,
        widgets::cycle_item(
            LABEL_SORT,
            &sort_labels,
            filter.sort_label(),
            focus == HomeField::FilterSort,
            LABEL_WIDTH,
        ),
    );
    push_input(items, HomeField::FilterLimit, &filter.limit, focus, editing);
    items.push(widgets::spacer());

    // A fetch in flight mirrors the search CTA: the static label swaps for an
    // inline braille spinner and the button is inert until results land.
    let loading = matches!(filter.status_msg, FindStatusMsg::Loading);
    let cta_label = if loading {
        format!("{} filtering", spinner_str(tick).trim())
    } else {
        LABEL_CTA.to_string()
    };
    items.push_focusable(
        HomeField::FilterRun,
        widgets::button_item(&cta_label, focus == HomeField::FilterRun, !loading),
    );

    // `view N maps` reopens the results browse without re-fetching; inert until
    // fresh results still matching the inputs are loaded (search parity).
    let loaded = filter.browse.rows.len();
    let view_current = loaded > 0 && filter.results_current();
    let view_label = if view_current {
        widgets::view_maps_label(loaded)
    } else {
        "view maps".to_string()
    };
    items.push_focusable(
        HomeField::FilterBrowse,
        widgets::button_item(&view_label, focus == HomeField::FilterBrowse, view_current),
    );
    if let Some(row) = status_row(&filter.status_msg) {
        items.push(row);
    }

    let (download_label, download_enabled) =
        widgets::download_button_label(filter.browse.selected_count());
    items.push_focusable(
        HomeField::Download,
        widgets::button_item(
            &download_label,
            focus == HomeField::Download,
            download_enabled,
        ),
    );

    items.push(widgets::spacer());
    items.push(ListItem::new(Line::from(Span::styled(
        format!("  {CREDIT}"),
        Style::default().fg(text_faint()),
    ))));
}

fn push_input(
    items: &mut widgets::FormItems<HomeField>,
    field: HomeField,
    input: &InputField,
    focus: HomeField,
    editing: bool,
) {
    items.push_focusable(
        field,
        widgets::input_item(input, focus == field, editing, LABEL_WIDTH),
    );
}

/// The inline outcome line beneath the buttons: the pre-download size summary
/// (`N sets · X GB` from the response's `SizeMap`) once results are in, a
/// warning for an empty match, the reason for a failure.
fn status_row(msg: &FindStatusMsg) -> Option<ListItem<'static>> {
    let (label, color) = match msg {
        FindStatusMsg::ReadyFilter { sets, total_bytes } => (
            format!(
                "{sets} set{} · {}",
                if *sets == 1 { "" } else { "s" },
                format_bytes(*total_bytes, "B")
            ),
            text_faint(),
        ),
        FindStatusMsg::Empty => ("no matches".to_string(), warning()),
        FindStatusMsg::Error(reason) => (reason.clone(), danger()),
        FindStatusMsg::Idle | FindStatusMsg::Loading | FindStatusMsg::ReadySearch { .. } => {
            return None;
        }
    };
    Some(ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(label, Style::default().fg(color)),
    ])))
}
