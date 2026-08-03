//! Render for the Get Maps `Find` source: one union criteria form (free-text
//! query + preset/special/mode/status/sort chips + per-diff range inputs +
//! artist/mapper/title texts + limit) that auto-routes to an osu! API search or
//! an nzbasic BBD filter. A read-only resolved-backend indicator sits directly
//! above the CTA so the route (`via osu! api` / `via nzbasic`, or a conflict) is
//! visible before the run fires. The form descends into the shared flat browse
//! ([`super::set_browse`]) once results arrive. The source strip is drawn by the
//! Home view; these rows are pushed into the same Home panel.

use crate::app::{
    EnrichSink, FindBackend, FindRoute, FindSource, FindStatusMsg, HomeField, InputField,
    RangeHint, describe_range,
};
use crate::utils::format_bytes;
use ratatui::{
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets;
use super::{accent, danger, focused_label, line, spinner_str, text_dim, text_faint, warning};

const LABEL_PRESET: &str = "preset";
const LABEL_SPECIAL: &str = "special";
const LABEL_MODE: &str = "mode";
/// osu!'s own word for the rank-status facet.
const LABEL_STATUS: &str = "categories";
const LABEL_SORT: &str = "sort";
const LABEL_ADVANCED: &str = "advanced filters";
const LABEL_CTA: &str = "find";
/// Rank and played are the two osu!supporter-gated facets. Genre, language,
/// extra, and explicit work without supporter (live-probed 2026-08-03) and
/// render unconditionally. The two gated labels never render unless
/// `App.config.supporter` is a confirmed `true`.
const LABEL_EXPLICIT: &str = "explicit";
const LABEL_GENRE: &str = "genre";
const LABEL_LANGUAGE: &str = "language";
const LABEL_EXTRA: &str = "extra";
const LABEL_RANK: &str = "rank";
const LABEL_PLAYED: &str = "played";

/// Eyebrow headers grouping the chips, so the form reads as three decisions
/// (which macro / what to match / how to order) instead of one wall of rows.
/// The query box, the `advanced filters` disclosure and its range inputs, and
/// the CTAs sit outside every section.
const SECTION_PRESET: &str = "preset";
const SECTION_FILTERS: &str = "filters";
const SECTION_RESULTS: &str = "results";
/// Sentinel for a field under no eyebrow; never equals a rendered header label.
const SECTION_NONE: &str = "";

/// Shared label width so chips and inputs align their values. The widest labels
/// are `favourites` and `categories` (10 each), so every row's value stacks at
/// that column.
/// `pub(crate)` so the Home view computes the caret column with the same offset.
pub(crate) const LABEL_WIDTH: usize = "favourites".len();

/// Left title of the results browse (the left pane of the master-detail).
pub const BROWSE_LIST_TITLE: &str = " RESULTS ";

/// Credit + risk line for the nzbasic route: the backing database is a
/// community-hosted free instance, so the copy names the source and warns it can
/// be unavailable. Only shown when the criteria resolve to nzbasic.
const CREDIT: &str = "data by nzbasic (batch beatmap downloader)";

/// Push the find-source FORM rows into the Home panel.
///
/// `width` is the panel's content width, which the query box spans; `chrome`
/// (off below `COMPACT_HEIGHT`) drops the section eyebrows, as it does for the
/// shared download section.
#[allow(clippy::too_many_arguments)]
pub fn push_form_rows(
    items: &mut widgets::FormItems<HomeField>,
    find: &FindSource,
    focus: HomeField,
    editing: bool,
    tick: u64,
    primary: HomeField,
    width: u16,
    chrome: bool,
    supporter: bool,
) {
    let route = find.resolved_route();
    let active_section = find_section(focus);
    let section = |items: &mut widgets::FormItems<HomeField>, label: &str| {
        if chrome {
            items.push(widgets::section_header(label, active_section == label));
        }
    };

    // The query is the form's headline control, so it renders as a search box
    // spanning the panel rather than as one more labelled row.
    items.push_focusable(
        HomeField::FindQuery,
        widgets::search_box_item(&find.query, focus == HomeField::FindQuery, editing, width),
    );
    push_hint(
        items,
        HomeField::FindQuery,
        &find.query,
        focus,
        field_error(find, HomeField::FindQuery),
    );
    items.push(widgets::spacer());

    section(items, SECTION_PRESET);
    items.push_focusable(
        HomeField::FindPreset,
        widgets::cycle_item(
            LABEL_PRESET,
            find.preset_labels(),
            find.preset_label(),
            focus == HomeField::FindPreset,
            LABEL_WIDTH,
            width,
        ),
    );
    items.push(widgets::spacer());

    // Mode before categories, matching the osu! website's own facet order.
    section(items, SECTION_FILTERS);
    items.push_focusable(
        HomeField::FindMode,
        widgets::cycle_item(
            LABEL_MODE,
            find.mode_labels(),
            find.mode_label(),
            focus == HomeField::FindMode,
            LABEL_WIDTH,
            width,
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
            width,
        ),
    );
    // `explicit` sits between categories and special, matching the osu! website's
    // own filter block.
    items.push_focusable(
        HomeField::FindExplicit,
        widgets::cycle_item(
            LABEL_EXPLICIT,
            find.explicit_labels(),
            find.explicit_label(),
            focus == HomeField::FindExplicit,
            LABEL_WIDTH,
            width,
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
            width,
        ),
    );
    items.push(widgets::spacer());

    // Sort and limit both shape the result SET rather than which maps match, so
    // they group away from the criteria chips.
    section(items, SECTION_RESULTS);
    let sort_labels = find.sort_labels();
    items.push_focusable(
        HomeField::FindSort,
        widgets::cycle_item(
            LABEL_SORT,
            &sort_labels,
            find.sort_label(),
            focus == HomeField::FindSort,
            LABEL_WIDTH,
            width,
        ),
    );
    push_input(
        items,
        HomeField::FindLimit,
        &find.limit,
        focus,
        editing,
        find,
    );
    items.push(widgets::spacer());

    // The disclosure gates the 13 per-attribute range inputs. Collapsed by
    // default so the primary form (query + chips + find) fits on one screen;
    // `space`/`enter` toggle it (see `is_disclosure` in the key handler).
    let advanced_focused = focus == HomeField::FindAdvanced;
    let show_advanced = find.show_advanced_filters() || focus.is_advanced();
    items.push_focusable(
        HomeField::FindAdvanced,
        advanced_filters_item(show_advanced, advanced_focused),
    );
    if show_advanced {
        push_advanced_facets(items, find, focus, editing, width, supporter);
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
    }
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

/// The five facets that open the advanced section, ahead of the per-attribute
/// ranges: `genre  language  extra  rank  played`. `extra` and `rank` are
/// MULTI-select — several chips can be on at once, so each renders its own pick
/// marks ([`widgets::multi_chip_item`]) rather than a single accented value,
/// and descends into a chip cursor on `↵`.
///
/// Genre, language, and extra render unconditionally (live-probed 2026-08-03);
/// rank and played only render when `supporter` is true.
fn push_advanced_facets(
    items: &mut widgets::FormItems<HomeField>,
    find: &FindSource,
    focus: HomeField,
    editing: bool,
    width: u16,
    supporter: bool,
) {
    // Ungated: genre, language, extra.
    for (field, label, labels, selected) in [
        (
            HomeField::FindGenre,
            LABEL_GENRE,
            find.genre_labels(),
            find.genre_label(),
        ),
        (
            HomeField::FindLanguage,
            LABEL_LANGUAGE,
            find.language_labels(),
            find.language_label(),
        ),
    ] {
        items.push_focusable(
            field,
            widgets::cycle_item(label, labels, selected, focus == field, LABEL_WIDTH, width),
        );
    }
    items.push_focusable(
        HomeField::FindExtra,
        widgets::multi_chip_item(
            LABEL_EXTRA,
            find.extra_labels(),
            |idx| find.extra.contains(idx),
            find.extra.cursor(),
            focus == HomeField::FindExtra,
            editing && focus == HomeField::FindExtra,
            LABEL_WIDTH,
            width,
        ),
    );
    if focus == HomeField::FindExtra {
        push_chip_hint(items, focus, editing);
    }

    // Supporter-gated: rank, played.
    if supporter {
        items.push_focusable(
            HomeField::FindRank,
            widgets::multi_chip_item(
                LABEL_RANK,
                find.rank_labels(),
                |idx| find.rank.contains(idx),
                find.rank.cursor(),
                focus == HomeField::FindRank,
                editing && focus == HomeField::FindRank,
                LABEL_WIDTH,
                width,
            ),
        );
        if focus == HomeField::FindRank {
            push_chip_hint(items, focus, editing);
        }
        items.push_focusable(
            HomeField::FindPlayed,
            widgets::cycle_item(
                LABEL_PLAYED,
                find.played_labels(),
                find.played_label(),
                focus == HomeField::FindPlayed,
                LABEL_WIDTH,
                width,
            ),
        );
    }
}

/// The one `└ …` tooltip the multi-select pair carries, anchored under
/// whichever row holds focus. A row whose second stage is advertised nowhere on
/// screen is a stage nobody finds, so it states its own grammar as well as the
/// footer does; the key spellings match the footer's, since both land in the
/// same frame.
fn push_chip_hint(items: &mut widgets::FormItems<HomeField>, focus: HomeField, editing: bool) {
    if !focus.is_find_multi_chip() {
        return;
    }
    items.push(widgets::help_item_keyed(if editing {
        "[←→] move · [space] toggle · [esc] done"
    } else {
        "[↵] edit (multiselect)"
    }));
}

/// The eyebrow a focused find field sits under, driving its underline cue. The
/// query box, the `advanced filters` disclosure and its range inputs, and the
/// CTAs render outside every section, so focusing them lights no header.
fn find_section(field: HomeField) -> &'static str {
    use HomeField::*;
    match field {
        FindPreset => SECTION_PRESET,
        FindMode | FindStatus | FindExplicit | FindSpecial => SECTION_FILTERS,
        FindSort | FindLimit => SECTION_RESULTS,
        _ => SECTION_NONE,
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
        FindRoute::Osu => via_trailing(FindBackend::Osu.label()),
        FindRoute::Nzbasic => via_trailing(FindBackend::Nzbasic.label()),
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

/// The `advanced filters` disclosure row: `▶` collapsed (TEXT_DIM) / `▼`
/// expanded (ACCENT), followed by the label. A standalone form row — the
/// expanded inputs below it column-align at [`LABEL_WIDTH`] on their own.
fn advanced_filters_item(open: bool, focused: bool) -> ListItem<'static> {
    let (glyph, glyph_color) = if open {
        (widgets::EXPANDED, accent())
    } else {
        (widgets::COLLAPSED, text_dim())
    };
    ListItem::new(Line::from(vec![
        widgets::focus_span(focused),
        Span::styled(format!("{glyph} "), Style::default().fg(glyph_color)),
        Span::styled(LABEL_ADVANCED.to_string(), focused_label(focused)),
    ]))
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
