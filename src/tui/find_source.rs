//! Render for the Get Maps `Find` source: one union criteria form (free-text
//! query + preset/special/mode/status/sort chips + per-diff range inputs +
//! artist/mapper/title texts + limit) that auto-routes to an osu! API search or
//! an nzbasic BBD filter. A read-only resolved-backend indicator sits directly
//! above the CTA so the route (`via osu! api` / `via nzbasic`, or a conflict) is
//! visible before the run fires. The form descends into the shared flat browse
//! ([`super::set_browse`]) once results arrive. The source strip is drawn by the
//! Home view; these rows are pushed into the same Home panel.

use crate::app::{FindRoute, FindSource, FindStatusMsg, HomeField, InputField};
use crate::utils::format_bytes;
use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets;
use super::{danger, line, spinner_str, text_dim, text_faint, warning};

const LABEL_PRESET: &str = "preset";
const LABEL_SPECIAL: &str = "special";
const LABEL_MODE: &str = "mode";
const LABEL_STATUS: &str = "status";
const LABEL_SORT: &str = "sort";
const LABEL_CTA: &str = "find";

/// Shared label width so chips and inputs align their values. The widest label
/// is the `favourites` range input, so every row's value stacks at that column.
/// `pub(crate)` so the Home view computes the caret column with the same offset.
pub(crate) const LABEL_WIDTH: usize = "favourites".len();

/// Left title of the results browse (the left pane of the master-detail).
pub const BROWSE_LIST_TITLE: &str = " RESULTS ";

/// Credit + risk line for the nzbasic route: the backing database is a
/// community-hosted free instance, so the copy names the source and warns it can
/// be unavailable. Only shown when the criteria resolve to nzbasic.
const CREDIT: &str = "data by nzbasic (batch beatmap downloader) · community-hosted";

/// Push the find-source FORM rows into the Home panel.
pub fn push_form_rows(
    items: &mut widgets::FormItems<HomeField>,
    find: &FindSource,
    focus: HomeField,
    editing: bool,
    tick: u64,
) {
    let route = find.resolved_route();

    push_input(items, HomeField::FindQuery, &find.query, focus, editing);
    items.push(widgets::spacer());

    items.push_focusable(
        HomeField::FindPreset,
        widgets::cycle_item(
            LABEL_PRESET,
            find.preset_labels(),
            find.preset_label(),
            focus == HomeField::FindPreset,
            LABEL_WIDTH,
        ),
    );
    items.push_focusable(
        HomeField::FindSpecial,
        widgets::cycle_item(
            LABEL_SPECIAL,
            find.special_labels(),
            find.special_label(),
            focus == HomeField::FindSpecial,
            LABEL_WIDTH,
        ),
    );
    items.push_focusable(
        HomeField::FindMode,
        widgets::cycle_item(
            LABEL_MODE,
            find.mode_labels(),
            find.mode_label(),
            focus == HomeField::FindMode,
            LABEL_WIDTH,
        ),
    );
    items.push_focusable(
        HomeField::FindStatus,
        widgets::cycle_item(
            LABEL_STATUS,
            find.status_labels(),
            find.status_label(),
            focus == HomeField::FindStatus,
            LABEL_WIDTH,
        ),
    );
    let sort_labels = find.sort_labels();
    items.push_focusable(
        HomeField::FindSort,
        widgets::cycle_item(
            LABEL_SORT,
            &sort_labels,
            find.sort_label(),
            focus == HomeField::FindSort,
            LABEL_WIDTH,
        ),
    );
    items.push(widgets::spacer());

    for (field, input) in [
        (HomeField::FindStars, &find.stars),
        (HomeField::FindAr, &find.ar),
        (HomeField::FindCs, &find.cs),
        (HomeField::FindOd, &find.od),
        (HomeField::FindHp, &find.hp),
        (HomeField::FindBpm, &find.bpm),
        (HomeField::FindLength, &find.length),
        (HomeField::FindKeys, &find.keys),
        (HomeField::FindFavourites, &find.favourites),
        (HomeField::FindRanked, &find.ranked),
        (HomeField::FindArtist, &find.artist),
        (HomeField::FindCreator, &find.creator),
        (HomeField::FindTitle, &find.title),
    ] {
        push_input(items, field, input, focus, editing);
    }
    items.push(widgets::spacer());

    push_input(items, HomeField::FindLimit, &find.limit, focus, editing);
    items.push(widgets::spacer());

    // Read-only resolved-backend indicator, directly above the CTA so the route
    // (or a routing conflict) is visible the moment the user reaches the button
    // — the mitigation for a sort preset silently switching backends.
    items.push(route_indicator(&route));

    // A run in flight mirrors the scan CTA: the static `find` label swaps for an
    // inline braille spinner and the button is inert until results land.
    let loading = matches!(find.status_msg, FindStatusMsg::Loading);
    let cta_label = if loading {
        format!("{} finding", spinner_str(tick).trim())
    } else {
        LABEL_CTA.to_string()
    };
    items.push_focusable(
        HomeField::FindRun,
        widgets::button_item(&cta_label, focus == HomeField::FindRun, !loading),
    );

    // `view N maps` reopens the results browse without re-fetching; inert until
    // fresh results still matching the inputs are loaded.
    let loaded = find.browse.rows.len();
    let view_current = loaded > 0 && find.results_current();
    let view_label = if view_current {
        widgets::view_maps_label(loaded)
    } else {
        "view maps".to_string()
    };
    items.push_focusable(
        HomeField::FindBrowse,
        widgets::button_item(&view_label, focus == HomeField::FindBrowse, view_current),
    );
    if let Some(row) = status_row(&find.status_msg) {
        items.push(row);
    }

    // osu-routed results carry a nekoha size backfill, so the button reads
    // `download (N) · ~X`; the nzbasic route (and un-probed sets) sums to 0, which
    // drops the suffix and leaves the plain `download (N)`.
    let (download_label, download_enabled) = widgets::download_button_label_with_size(
        find.browse.selected_count(),
        find.checked_known_bytes(),
    );
    items.push_focusable(
        HomeField::Download,
        widgets::button_item(
            &download_label,
            focus == HomeField::Download,
            download_enabled,
        ),
    );

    // The nzbasic credit/risk line only when the criteria actually resolve there.
    if matches!(route, FindRoute::Nzbasic) {
        items.push(widgets::spacer());
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  {CREDIT}"),
            Style::default().fg(text_faint()),
        ))));
    }
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

/// The read-only resolved-backend indicator row: `→ via osu! api` / `→ via
/// nzbasic` for a clean route, or `! <field> needs nzbasic · <field> needs osu!
/// api` in a warning tint for a conflict. Never focusable — a live view of
/// [`FindSource::resolved_route`].
fn route_indicator(route: &FindRoute) -> ListItem<'static> {
    let spans = match route {
        FindRoute::Osu => via_spans("osu! api"),
        FindRoute::Nzbasic => via_spans("nzbasic"),
        FindRoute::Conflict { nzbasic, osu } => vec![
            Span::styled("  ! ", Style::default().fg(warning())),
            Span::styled(
                format!("{nzbasic} needs nzbasic · {osu} needs osu! api"),
                Style::default().fg(warning()),
            ),
        ],
    };
    ListItem::new(Line::from(spans))
}

/// `→ via <backend>`: the arrow in `LINE`, the copy in `TEXT_DIM` (a recessive
/// read-only cue, never a chip).
fn via_spans(backend: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(" → ", Style::default().fg(line())),
        Span::styled(format!("via {backend}"), Style::default().fg(text_dim())),
    ]
}

/// The inline outcome line beneath the buttons: the nzbasic pre-download size
/// summary (`N sets · X GB` from the response's `SizeMap`), a warning for an
/// empty match, or the reason for a failure. The osu `Ready` case shows its
/// count as a ratio in the RESULTS pane title instead, so it adds no row here.
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
        FindStatusMsg::Empty => ("no results".to_string(), warning()),
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
