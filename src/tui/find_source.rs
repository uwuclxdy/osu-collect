//! Render for the Get Maps `Find` source: one union criteria form (free-text
//! query + preset/special/mode/status/sort chips + per-diff range inputs +
//! artist/mapper/title texts + limit) that auto-routes to an osu! API search or
//! an nzbasic BBD filter. A read-only resolved-backend indicator sits directly
//! above the CTA so the route (`via osu! api` / `via nzbasic`, or a conflict) is
//! visible before the run fires. The form descends into the shared flat browse
//! ([`super::set_browse`]) once results arrive. The source strip is drawn by the
//! Home view; these rows are pushed into the same Home panel.

use crate::app::{
    EnrichSink, FindRoute, FindSource, FindStatusMsg, HomeField, InputField, RangeHint,
    describe_range,
};
use crate::utils::format_bytes;
use ratatui::{
    style::Stylize,
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
const CREDIT: &str = "data by nzbasic (batch beatmap downloader)";

/// Push the find-source FORM rows into the Home panel.
pub fn push_form_rows(
    items: &mut widgets::FormItems<HomeField>,
    find: &FindSource,
    focus: HomeField,
    editing: bool,
    tick: u64,
    primary: HomeField,
) {
    let route = find.resolved_route();

    push_input(
        items,
        HomeField::FindQuery,
        &find.query,
        focus,
        editing,
        find,
    );

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
    items.push(widgets::spacer());
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
        push_input(items, field, input, focus, editing, find);
    }
    items.push(widgets::spacer());

    push_input(
        items,
        HomeField::FindLimit,
        &find.limit,
        focus,
        editing,
        find,
    );
    items.push(widgets::spacer());

    // A run in flight mirrors the scan CTA: the static `find` label swaps for an
    // inline braille spinner and the button is inert until results land. The
    // resolved-backend indicator trails the CTA on the SAME row (`→ via osu! api`
    // / `via nzbasic`, or a conflict), so the route is visible the moment the
    // user reaches the button — the mitigation for a sort preset silently
    // switching backends.
    let loading = matches!(find.status_msg, FindStatusMsg::Loading);
    let cta_label = if loading {
        format!("{} finding", spinner_str(tick).trim())
    } else {
        LABEL_CTA.to_string()
    };
    items.push_focusable(
        HomeField::FindRun,
        widgets::button_item_with_trailing(
            &cta_label,
            focus == HomeField::FindRun,
            !loading,
            route_trailing_spans(&route),
            widgets::ButtonProminence::primary_if(HomeField::FindRun == primary),
        ),
    );

    // `view N maps` reopens the results browse without re-fetching; inert until
    // fresh results still matching the inputs are loaded.
    let loaded = find.browse.rows.len();
    let view_current = loaded > 0 && find.results_current();
    items.push_focusable(
        HomeField::FindBrowse,
        widgets::view_browse_button(
            loaded,
            focus == HomeField::FindBrowse,
            view_current,
            find.browse.is_enriching(),
            tick,
            widgets::ButtonProminence::primary_if(HomeField::FindBrowse == primary),
        ),
    );
    if let Some(row) = status_row(&find.status_msg) {
        items.push(row);
    }

    // The shared download button + run settings (mirrors / directory / threads /
    // overwrite / video) render AFTER this, in the Home view's download section —
    // one section borrowed across all three sources. The nzbasic credit/risk line
    // shows only when the criteria actually resolve there.
    if matches!(route, FindRoute::Nzbasic) {
        items.push(widgets::spacer());
        items.push(ListItem::new(Line::from(
            format!("  {CREDIT}").fg(text_faint()),
        )));
    }
}

fn push_input(
    items: &mut widgets::FormItems<HomeField>,
    field: HomeField,
    input: &InputField,
    focus: HomeField,
    editing: bool,
    find: &FindSource,
) {
    items.push_focusable(
        field,
        widgets::input_item(input, focus == field, editing, LABEL_WIDTH),
    );
    push_hint(items, field, input, focus, field_error(find, field));
}

/// The live parse error for a focused field whose grammar [`describe_range`]
/// can't read — `ranked` runs its own date grammar and `limit` is a plain cap.
/// The numeric range fields validate off their own value in [`range_hint_item`],
/// so they never route through here.
fn field_error(find: &FindSource, field: HomeField) -> Option<String> {
    match field {
        HomeField::FindRanked => find.ranked_error(),
        HomeField::FindLimit => find.limit_error(),
        _ => None,
    }
}

/// A `└ <hint>` tooltip below the row when it holds focus. A numeric range field
/// gets a LIVE reading of its value ([`range_hint_item`]) ONLY once something is
/// typed — an empty range field shows no tooltip. The rest get a static hint for
/// the non-obvious syntax (query q-DSL, ranked date range, limit cap), replaced
/// by the parse error while the value is invalid (`error`) so a field that can be
/// typed wrong always says so at the keystroke, not at the run. Rows that read
/// plainly (preset/special/mode/status/sort/artist/title) get none.
fn push_hint(
    items: &mut widgets::FormItems<HomeField>,
    field: HomeField,
    input: &InputField,
    focus: HomeField,
    error: Option<String>,
) {
    if focus != field {
        return;
    }
    if is_range_field(field) {
        if let Some(item) = range_hint_item(input) {
            items.push(item);
        }
    } else if let Some(reason) = error {
        items.push(error_hint_item(reason));
    } else if let Some(hint) = field_hint(field) {
        items.push(widgets::help_item_keyed(hint));
    }
}

/// A danger-tinted `└ <reason>` tooltip — the one render of a live parse error,
/// shared by the numeric ranges and the fields with their own grammar.
fn error_hint_item(reason: String) -> ListItem<'static> {
    ListItem::new(Line::from(vec!["  └ ".fg(line()), reason.fg(danger())]))
}

/// The nine numeric range fields that parse the operator grammar (`7+`, `5..7`,
/// `>9`). `ranked` is a date field on its own `..` grammar, so it is excluded.
fn is_range_field(field: HomeField) -> bool {
    use HomeField::*;
    matches!(
        field,
        FindStars
            | FindAr
            | FindCs
            | FindOd
            | FindHp
            | FindBpm
            | FindLength
            | FindKeys
            | FindFavourites
    )
}

/// Live reading of a numeric range field, shown only once a value is typed: a
/// plain-english interpretation when it parses (`maps with 7 stars or higher`) or
/// the parse error when it does not. `None` (no tooltip) while the field is blank.
/// Example numbers highlight; errors are danger-tinted.
fn range_hint_item(input: &InputField) -> Option<ListItem<'static>> {
    match describe_range(input.label, &input.value) {
        RangeHint::Empty => None,
        RangeHint::Valid(reading) => Some(widgets::help_item_keyed(&reading)),
        RangeHint::Invalid(reason) => Some(error_hint_item(reason)),
    }
}

/// Example values wrapped in `[…]` render highlighted (accent) via
/// [`widgets::help_item_keyed`]; the surrounding prose stays faint.
fn field_hint(field: HomeField) -> Option<&'static str> {
    use HomeField::*;
    Some(match field {
        FindQuery => "supports osu-native filter expressions like ar:10",
        FindRanked => "e.g. [2020..2024], [2020-06-01..], [..2024]",
        FindLimit => "caps diff rows (default 500)",
        _ => return None,
    })
}

/// The resolved-backend indicator spans, trailing the find CTA on its row:
/// `→ via osu! api` / `→ via nzbasic` for a clean route, or `! <field> needs
/// nzbasic · <field> needs osu! api` in a warning tint for a conflict. A live
/// view of [`FindSource::resolved_route`]; the leading gap separates it from the
/// button pill.
fn route_trailing_spans(route: &FindRoute) -> Vec<Span<'static>> {
    match route {
        FindRoute::Osu => via_trailing("osu! api"),
        FindRoute::Nzbasic => via_trailing("nzbasic"),
        FindRoute::Conflict { nzbasic, osu } => vec![
            "  ! ".fg(warning()),
            format!("{nzbasic} needs nzbasic · {osu} needs osu! api").fg(warning()),
        ],
    }
}

/// `→ via <backend>`: the arrow in `LINE`, the copy in `TEXT_DIM` (a recessive
/// read-only cue, never a chip).
fn via_trailing(backend: &str) -> Vec<Span<'static>> {
    vec![" → ".fg(line()), format!("via {backend}").fg(text_dim())]
}

/// The inline outcome line beneath the buttons: the nzbasic pre-download size
/// summary (`N mapsets · X GB` from the response's `SizeMap`), a warning for an
/// empty match, or the reason for a failure. The osu `Ready` case shows its
/// count as a ratio in the RESULTS pane title instead, so it adds no row here.
fn status_row(msg: &FindStatusMsg) -> Option<ListItem<'static>> {
    let (label, color) = match msg {
        FindStatusMsg::ReadyFilter { sets, total_bytes } => (
            format!(
                "{sets} mapset{} · {}",
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
        label.fg(color),
    ])))
}
