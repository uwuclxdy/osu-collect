use crate::app::{
    UpdatesField, UpdatesTab,
    updates::{BeatmapDisplayItem, ScanStatus},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
};

use super::{UpdatesView, components};

pub fn render(frame: &mut Frame, area: Rect, view: UpdatesView) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(3),
    ])
    .split(area);

    render_tooltip(frame, chunks[0]);
    render_form(frame, chunks[1], view.form);
    components::render_console(
        frame,
        chunks[2],
        components::ConsoleMessage {
            message: view.form.message.as_ref(),
            quit_prompt: false,
            default_text: " Space: toggle client/selections | Enter: download | a/d: select/deselect all",
        },
    );
}

fn render_tooltip(frame: &mut Frame, area: Rect) {
    let text = " Configure download directory and mirrors in the Home tab before downloading!";
    let paragraph = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
    frame.render_widget(paragraph, area);
}

fn render_form(frame: &mut Frame, area: Rect, form: &UpdatesTab) {
    let items = build_items(form, area.height);

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Updates ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .highlight_symbol("");
    frame.render_widget(list, area);
}

fn build_items(form: &UpdatesTab, area_height: u16) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();

    items.push(client_toggle(form));
    items.push(osu_path_item(form));

    items.push(collections_header(form));

    if form.selection.in_collection_list {
        // area_height includes borders (2 lines), and we have 3 header lines above
        let collection_list_header_offset = 3u16;
        let collection_list_footer_offset = 3u16; // beatmaps header + summary + some padding
        let available_height = area_height
            .saturating_sub(2) // borders
            .saturating_sub(collection_list_header_offset)
            .saturating_sub(collection_list_footer_offset) as usize;

        let selected_idx = form.selection.collections_state.selected().unwrap_or(0);
        let total_items = form.selection.local_collections.len();

        // Calculate scroll offset to keep selection visible
        let scroll_offset = calculate_scroll_offset(selected_idx, total_items, available_height);

        for (i, collection) in form
            .selection
            .local_collections
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(available_height)
        {
            let is_scroll_pos = i == selected_idx;
            items.push(collection_item(collection, is_scroll_pos));
        }
    } else if !form.selection.local_collections.is_empty() {
        let selected = form.selected_collection_count();
        let total = form.selection.local_collections.len();
        items.push(ListItem::new(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!("{selected}/{total} collections selected"),
                Style::default().fg(Color::DarkGray),
            ),
        ])));
    }

    items.push(beatmaps_header(form));

    if form.selection.in_beatmap_list {
        let lines_above_beatmap_list = 5u16;
        let available_height = area_height
            .saturating_sub(2) // borders
            .saturating_sub(lines_above_beatmap_list) as usize;

        let selected_idx = form.selection.beatmaps_state.selected().unwrap_or(0);

        // Use display items from form (includes collection headers)
        let total_items = form.selection.display_items.len();

        // Calculate scroll offset to keep selection visible
        let scroll_offset = calculate_scroll_offset(selected_idx, total_items, available_height);

        for (i, item) in form
            .selection
            .display_items
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(available_height)
        {
            let is_scroll_pos = i == selected_idx;
            items.push(display_item_to_list_item(item, is_scroll_pos, form));
        }
    } else if form.total_missing_count() > 0 {
        let selected = form.selected_beatmap_count();
        let total = form.total_missing_count();
        items.push(ListItem::new(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!("{selected}/{total} beatmaps selected"),
                Style::default().fg(Color::DarkGray),
            ),
        ])));
    } else {
        let is_loading = matches!(
            form.scan.scan_status,
            ScanStatus::ReadingDatabase | ScanStatus::FetchingCollection
        );
        if is_loading {
            items.push(ListItem::new(Line::from(vec![
                Span::raw("    "),
                Span::styled("Loading...", Style::default().fg(Color::DarkGray)),
            ])));
        }
    }

    items
}

fn calculate_scroll_offset(
    selected_idx: usize,
    total_items: usize,
    visible_height: usize,
) -> usize {
    if visible_height == 0 || total_items == 0 {
        return 0;
    }

    // Keep selected item in middle-ish area when possible
    let half_visible = visible_height / 2;

    if selected_idx < half_visible {
        // Near start, no scrolling needed
        0
    } else if selected_idx >= total_items.saturating_sub(half_visible) {
        // Near end, scroll to show last items
        total_items.saturating_sub(visible_height)
    } else {
        // In middle, center on selection
        selected_idx.saturating_sub(half_visible)
    }
}

fn display_item_to_list_item(
    item: &BeatmapDisplayItem,
    is_scroll_pos: bool,
    form: &UpdatesTab,
) -> ListItem<'static> {
    match item {
        BeatmapDisplayItem::CollectionHeader { collection_id } => {
            let name = form
                .selection
                .cached_missing_sets
                .iter()
                .find(|b| b.collection_id == *collection_id)
                .map(|b| b.collection_name.clone())
                .unwrap_or_default();

            let visible_cache_indices: Vec<usize> = form
                .selection
                .visible_missing
                .iter()
                .copied()
                .filter(|&cache_idx| {
                    form.selection
                        .cached_missing_sets
                        .get(cache_idx)
                        .map(|beatmap| beatmap.collection_id == *collection_id)
                        .unwrap_or(false)
                })
                .collect();

            let all_selected = !visible_cache_indices.is_empty()
                && visible_cache_indices.iter().all(|&cache_idx| {
                    form.selection
                        .cached_missing_sets
                        .get(cache_idx)
                        .map(|beatmap| beatmap.selected)
                        .unwrap_or(false)
                });
            let marker = if all_selected { "[x]" } else { "[ ]" };

            let style = Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD);
            let spans = vec![
                Span::styled(
                    if is_scroll_pos { "  > " } else { "    " },
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(marker, style),
                Span::raw(" "),
                Span::styled(format!("#{collection_id} - {name}"), style),
            ];
            ListItem::new(Line::from(spans))
        }
        BeatmapDisplayItem::Beatmap { cache_index } => {
            if let Some(beatmap) = form.selection.cached_missing_sets.get(*cache_index) {
                beatmap_item(beatmap, is_scroll_pos)
            } else {
                ListItem::new(Line::from(""))
            }
        }
    }
}

fn client_toggle(form: &UpdatesTab) -> ListItem<'static> {
    let focused = form.selection.focus == UpdatesField::ClientType
        && !form.selection.in_collection_list
        && !form.selection.in_beatmap_list;

    let lazer_style = if form.path.client_type == crate::osu_db::OsuClient::Lazer {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let stable_style = if form.path.client_type == crate::osu_db::OsuClient::Stable {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let spans = vec![
        Span::styled(
            if focused { "> " } else { "  " },
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("Client: ", Style::default()),
        Span::raw("["),
        Span::styled(
            if form.path.client_type == crate::osu_db::OsuClient::Lazer {
                "●"
            } else {
                "○"
            },
            lazer_style,
        ),
        Span::styled(" Lazer", lazer_style),
        Span::raw(" "),
        Span::styled(
            if form.path.client_type == crate::osu_db::OsuClient::Stable {
                "●"
            } else {
                "○"
            },
            stable_style,
        ),
        Span::styled(" Stable", stable_style),
        Span::raw("]"),
    ];

    let style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    ListItem::new(Line::from(spans)).style(style)
}

fn osu_path_item(form: &UpdatesTab) -> ListItem<'static> {
    let focused = form.selection.focus == UpdatesField::OsuPath
        && !form.selection.in_collection_list
        && !form.selection.in_beatmap_list;
    let field = &form.path.osu_path;

    let value = if field.value.is_empty() {
        Span::styled(
            field.placeholder.clone(),
            Style::default(),
        )
    } else if form.is_path_auto_detected() {
        // Auto-detected path: show in dark gray like placeholder
        Span::styled(field.value.clone(), Style::default().fg(Color::DarkGray))
    } else {
        // Manually entered: show in normal color
        Span::raw(field.value.clone())
    };

    let spans = vec![
        Span::styled(
            if focused { "> " } else { "  " },
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(
            format!("{}: ", field.label),
            Style::default(),
        ),
        value,
    ];

    let style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    ListItem::new(Line::from(spans)).style(style)
}

fn collections_header(form: &UpdatesTab) -> ListItem<'static> {
    let focused =
        form.selection.focus == UpdatesField::Collections && !form.selection.in_beatmap_list;
    let in_list = form.selection.in_collection_list;

    let style = if focused || in_list {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let prefix = if focused && !in_list { "> " } else { "  " };
    let suffix = if in_list {
        "(Space: toggle, Enter/Esc: exit)"
    } else {
        "(Space: expand)"
    };

    let spans = vec![
        Span::styled(prefix, Style::default().fg(Color::Cyan)),
        Span::styled("Collections: ", style),
        Span::styled(
            suffix.to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    ListItem::new(Line::from(spans)).style(style)
}

fn collection_item(
    collection: &crate::app::updates::CollectionEntry,
    is_scroll_pos: bool,
) -> ListItem<'static> {
    let marker = if collection.selected { "[x]" } else { "[ ]" };

    let style = if is_scroll_pos {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let id_str = collection
        .collection_id
        .map(|id| format!("#{id} - "))
        .unwrap_or_default();

    let spans = vec![
        Span::styled(
            if is_scroll_pos { "  > " } else { "    " },
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(marker, style),
        Span::raw(" "),
        Span::styled(id_str, Style::default().fg(Color::Magenta)),
        Span::raw(collection.name.clone()),
        Span::styled(
            format!(" ({} maps)", collection.beatmap_count),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    ListItem::new(Line::from(spans)).style(style)
}

fn beatmaps_header(form: &UpdatesTab) -> ListItem<'static> {
    let focused =
        form.selection.focus == UpdatesField::BeatmapList && !form.selection.in_collection_list;
    let in_list = form.selection.in_beatmap_list;
    let is_fetching = matches!(
        form.scan.scan_status,
        ScanStatus::ReadingDatabase | ScanStatus::FetchingCollection
    );

    let style = if focused || in_list {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let prefix = if focused && !in_list { "> " } else { "  " };
    let suffix: Option<&str> = if is_fetching {
        None
    } else if in_list {
        Some("(Space: toggle, a: all, d: none, Enter/Esc: exit)")
    } else {
        Some("(Space: expand)")
    };

    let mut spans = vec![
        Span::styled(prefix, Style::default().fg(Color::Cyan)),
        Span::styled("Missing Beatmaps: ".to_string(), style),
    ];

    if let Some(text) = suffix {
        spans.push(
            Span::styled(
                text,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
        );
    }

    ListItem::new(Line::from(spans)).style(style)
}

fn beatmap_item(
    beatmap: &crate::app::updates::MissingBeatmapset,
    is_scroll_pos: bool,
) -> ListItem<'static> {
    let marker = if beatmap.selected { "[x]" } else { "[ ]" };

    let style = if is_scroll_pos {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let status_text = match beatmap.status {
        crate::app::updates::MissingStatus::NotInstalled => "(Not installed)",
    };

    let spans = vec![
        Span::styled(
            if is_scroll_pos { "  > " } else { "    " },
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(marker, style),
        Span::styled(
            format!(" #{}", beatmap.id),
            Style::default().fg(Color::Magenta),
        ),
        Span::raw(" "),
        Span::styled(
            status_text.to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    ListItem::new(Line::from(spans)).style(style)
}
