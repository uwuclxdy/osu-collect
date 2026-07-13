/// Rendering smoke tests using ratatui's TestBackend.
///
/// These verify that each view renders without panic and produces
/// non-empty output at standard terminal sizes.
use osu_collect::{app::App, config::Config, tui::draw};
use ratatui::{Terminal, backend::TestBackend};

fn make_app() -> App {
    App::new(Config::default())
}

fn render_to_buffer(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            draw(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn render_content(app: &App, width: u16, height: u16) -> String {
    render_to_buffer(app, width, height)
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

/// Terminal caret position after a draw. `(0, 0)` means no caret was set this
/// frame (a focused text field always parks the caret inside the panel, y > 0).
fn cursor_pos(app: &App, width: u16, height: u16) -> (u16, u16) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    // `draw` positions the caret via `Frame::set_cursor_position`; ratatui applies
    // it to the backend after the buffer flush. A frame that never sets it
    // leaves the cursor hidden — reported as `(0, 0)`.
    terminal.draw(|frame| draw(frame, app)).unwrap();
    let backend = terminal.backend();
    if backend.cursor_visible() {
        let pos = backend.cursor_position();
        (pos.x, pos.y)
    } else {
        (0, 0)
    }
}

#[test]
fn caret_advances_as_collection_field_is_typed() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Collection;
    app.editing = true; // edit mode: caret shows and keys type

    app.home.collection.set_value("");
    let empty = cursor_pos(&app, 120, 24);

    // Type through the key handler so the caret advances with each char.
    for ch in "abcde".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()));
    }
    let typed = cursor_pos(&app, 120, 24);

    assert_eq!(typed.1, empty.1, "caret stays on the same row");
    assert_eq!(
        typed.0,
        empty.0 + 5,
        "caret advances one column per typed char"
    );
}

#[test]
fn caret_follows_left_arrow_then_home_and_end() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Collection;
    app.editing = true; // edit mode: caret shows and keys type
    app.home.collection.set_value("");
    let origin = cursor_pos(&app, 120, 24);

    for ch in "abcde".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()));
    }

    // Two lefts park the caret three chars in.
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::empty()));
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::empty()));
    assert_eq!(
        cursor_pos(&app, 120, 24).0,
        origin.0 + 3,
        "two left arrows move the caret back two columns"
    );

    app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::empty()));
    assert_eq!(
        cursor_pos(&app, 120, 24).0,
        origin.0,
        "Home parks the caret at the value start"
    );

    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::empty()));
    assert_eq!(
        cursor_pos(&app, 120, 24).0,
        origin.0 + 5,
        "End parks the caret at the value end"
    );
}

#[test]
fn no_caret_on_toggle_field() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Video;
    assert_eq!(
        cursor_pos(&app, 120, 24),
        (0, 0),
        "no caret is shown when a non-text field is focused"
    );
}

#[test]
fn no_caret_while_help_overlay_open() {
    use osu_collect::app::HomeField;
    let mut app = make_app();
    app.home.focus = HomeField::Collection;
    app.help_open = true;
    assert_eq!(
        cursor_pos(&app, 120, 24),
        (0, 0),
        "the help overlay suppresses the text caret"
    );
}

// ── home view ────────────────────────────────────────────────────────────────

#[test]
fn home_renders_without_panic_standard() {
    let app = make_app();
    let content = render_content(&app, 120, 40);
    assert!(content.contains("osu!collect"));
}

#[test]
fn home_renders_collection_label() {
    let app = make_app();
    let content = render_content(&app, 120, 40);
    assert!(content.contains("collection"));
}

#[test]
fn home_renders_mirrors_section() {
    let app = make_app();
    let content = render_content(&app, 120, 40);
    assert!(content.contains("MIRRORS") || content.contains("mirrors"));
}

#[test]
fn home_cta_scrolls_into_view_on_short_terminal() {
    use osu_collect::app::{BrowseRow, HomeField};

    // 18 rows overflows the home form (~17 rows) but stays out of compact mode
    // (>= COMPACT_HEIGHT). The CTA is the last, unhighlighted row; before the
    // scroll/highlight split it was selected=None → offset 0 → off-screen.
    let mut app = make_app();
    // A picked subset makes the CTA read the unique "download (2)"; the bare
    // "download" label would otherwise collide with the "download directory"
    // field, so this pins the assertion to the button, not surrounding chrome.
    app.home.set_resolved_collection(1, vec![10, 20, 30]);
    app.home.collection_browse.set_rows(
        vec![
            BrowseRow { id: 10, meta: None },
            BrowseRow { id: 20, meta: None },
            BrowseRow { id: 30, meta: None },
        ],
        &std::collections::HashMap::new(),
    );
    app.home.collection_browse.set_all_selected(true);
    app.home.collection_browse.toggle_selected(); // drop one → a proper subset
    app.home.collection_browse_id = Some(1);
    app.home.focus = HomeField::Download;
    let content = render_content(&app, 120, 18);
    assert!(
        content.contains("download (2)"),
        "focused CTA must scroll into view on a short terminal: {content}"
    );
}

#[test]
fn collection_form_cta_reads_download_all() {
    // No picked subset → the collection CTA dispatches the whole collection and
    // reads `download all`, distinct from a source's bare disabled `download`.
    let app = make_app();
    let content = render_content(&app, 120, 40);
    assert!(
        content.contains("download all"),
        "collection form CTA reads 'download all': {content}"
    );
}

#[test]
fn search_view_maps_button_shows_when_results_loaded() {
    use osu_collect::app::{BrowseRow, GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.find.browse.set_rows(
        vec![
            BrowseRow { id: 1, meta: None },
            BrowseRow { id: 2, meta: None },
        ],
        &std::collections::HashMap::new(),
    );
    // Mirror the Ready handler, which snapshots the inputs the rows are for.
    app.home.find.mark_results_current();
    // The merged find form is long; focus the button so it scrolls into view.
    app.home.focus = HomeField::FindBrowse;
    let content = render_content(&app, 80, 26);
    assert!(
        content.contains("view 2 maps"),
        "find renders a `view N maps` button once results are loaded: {content}"
    );
}

#[test]
fn update_view_maps_button_is_disabled_until_a_scan_finds_updates() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    let content = render_content(&app, 80, 22);
    // Idle (no scan): the view button reads the bare `view maps`, sitting under
    // the scan CTA. It gains a count only after a scan finds updates.
    assert!(
        content.contains("scan for updates") && content.contains("view maps"),
        "update idle form shows the scan CTA + a disabled `view maps`: {content}"
    );
}

#[test]
fn collection_view_maps_button_shows_when_resolved() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Collection;
    app.home.set_resolved_collection(7, vec![10, 20, 30]);
    let content = render_content(&app, 80, 30);
    assert!(
        content.contains("view 3 maps"),
        "resolved collection renders a `view N maps` button: {content}"
    );
}

#[test]
fn collection_view_maps_button_renders_above_download_section() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Collection;
    app.home.set_resolved_collection(7, vec![10, 20, 30]);
    // `render_content` is row-major, so an earlier byte index == a higher row.
    // `view N maps` now groups with the collection field, above the shared
    // download section (`overwrite existing` lives at that section's tail).
    let content = render_content(&app, 80, 30);
    let view = content.find("view 3 maps").expect("view button present");
    let overwrite = content
        .find("overwrite existing")
        .expect("download section present");
    assert!(
        view < overwrite,
        "the collection `view N maps` button sits above the download section: {content}"
    );
}

#[test]
fn collection_browse_shows_focus_caret_and_uppercase_title() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use osu_collect::app::{GetMapsSource, HomeField};

    let mut app = make_app();
    app.home.source = GetMapsSource::Collection;
    app.home.set_resolved_collection(7, vec![10, 20, 30]);
    app.home.focus = HomeField::CollectionBrowse;
    // Descend into browse&pick: the list pane owns focus, so its cursor row draws
    // the caret. The browse claims the whole body, so this `❯` can only be the
    // list row's (the source form isn't rendered).
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert!(app.home.collection_browse.is_browsing());

    let content = render_content(&app, 120, 40);
    // Exactly one caret: the browse claims the whole body (no form rendered) and
    // only the cursor row is caret-marked. More than one would mean the per-row
    // `list_focused && cursor == Some(i)` gate regressed to caret-on-every-row.
    assert_eq!(
        content.matches('❯').count(),
        1,
        "exactly the cursor row shows the caret: {content}"
    );
    assert!(
        content.contains("COLLECTION"),
        "browse list panel title is uppercased: {content}"
    );
}

#[test]
fn search_cta_shows_inline_spinner_while_loading() {
    use osu_collect::app::{FindStatusMsg, GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.find.status_msg = FindStatusMsg::Loading;
    // Focus the CTA so it scrolls into view in the long merged find form.
    app.home.focus = HomeField::FindRun;
    let content = render_content(&app, 80, 24);
    // The CTA mirrors the scan CTA: an inline braille spinner replaces `find`
    // while a query is in flight (tick 0 → frame `⠋`), rather than a separate
    // status row below the button.
    assert!(
        content.contains("⠋ finding"),
        "find CTA shows an inline spinner while loading: {content}"
    );
}

#[test]
fn find_form_shows_resolved_backend_indicator() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    // Focus the CTA so the indicator (rendered directly above it) is in view.
    app.home.focus = HomeField::FindRun;
    let content = render_content(&app, 80, 24);
    // An untouched form routes osu, so the read-only indicator reads `via osu! api`.
    assert!(
        content.contains("via osu! api"),
        "the find form shows the resolved-backend indicator: {content}"
    );
}

#[test]
fn find_form_shows_conflict_in_indicator() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.focus = HomeField::FindRun;
    // A nzbasic-forcer (farm) plus an osu-forcer (free text) = a routing conflict;
    // the indicator names both offending fields instead of a route.
    app.home.find.cycle_special(true); // → farm
    app.home.find.query.set_value("tekno");
    let content = render_content(&app, 80, 24);
    assert!(
        content.contains("needs nzbasic") && content.contains("needs osu! api"),
        "a routing conflict renders inline in the indicator: {content}"
    );
}

#[test]
fn range_field_hint_reads_the_parsed_value() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    // A focused range field shows a live plain-english reading of its value; the
    // `[…]` highlight markers are stripped in the rendered cells.
    app.home.focus = HomeField::FindStars;

    app.home.find.stars.set_value("7+");
    let content = render_content(&app, 80, 40);
    assert!(
        content.contains("maps with 7 stars or higher"),
        "an inclusive lower bound reads plainly: {content}"
    );

    app.home.find.stars.set_value(">9");
    let content = render_content(&app, 80, 40);
    assert!(
        content.contains("maps above 9 stars"),
        "a strict lower bound reads as 'above': {content}"
    );

    app.home.find.stars.set_value("5..7");
    let content = render_content(&app, 80, 40);
    assert!(
        content.contains("maps between 5 and 7 stars"),
        "a two-sided range reads as 'between … and …': {content}"
    );

    // `-` is interchangeable with `..` as a range separator.
    app.home.find.stars.set_value("2-3");
    let content = render_content(&app, 80, 40);
    assert!(
        content.contains("maps between 2 and 3 stars"),
        "a dash range reads the same as a `..` range: {content}"
    );
}

#[test]
fn empty_range_field_shows_no_hint() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    // A focused-but-blank range field shows no tooltip at all (no example legend).
    app.home.focus = HomeField::FindStars;
    app.home.find.stars.set_value("");
    let content = render_content(&app, 80, 40);
    assert!(
        !content.contains("maps between")
            && !content.contains("or higher")
            && !content.contains("maps above"),
        "a blank range field renders no reading or legend: {content}"
    );
}

#[test]
fn range_field_hint_shows_parse_error() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.focus = HomeField::FindStars;
    app.home.find.stars.set_value("abc");
    let content = render_content(&app, 80, 40);
    assert!(
        content.contains("is not a number"),
        "an unparseable value surfaces its error in the hint: {content}"
    );
}

#[test]
fn help_overlay_shows_filter_syntax_on_home() {
    // The default active tab is Home (get maps), where the filter grammar applies.
    let mut app = make_app();
    app.help_open = true;
    let content = render_content(&app, 100, 44);
    assert!(
        content.contains("FILTER SYNTAX"),
        "the help overlay lists the filter grammar section: {content}"
    );
    assert!(
        content.contains("between 2 and 3"),
        "the filter grammar section names the range form: {content}"
    );
}

// ── update source view ───────────────────────────────────────────────────────

#[test]
fn update_source_shows_recheck_failed_control() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home.update.set_failed_beatmapset_count(2);
    // use a tall terminal so the summary_metrics row (last in the list) stays visible
    let content = render_content(&app, 120, 60);

    assert!(
        content.contains("known bad"),
        "summary metrics must surface the known-bad count"
    );
    assert!(
        content.contains('2'),
        "the known-bad beatmap count must be rendered"
    );
}

#[test]
fn update_browse_collection_row_follows_viewport_on_short_terminal() {
    use osu_collect::app::GetMapsSource;
    use osu_collect::app::update_source::CollectionEntry;

    // A long collections list with the cursor on the last entry: the `ListState`
    // scroll target must follow the cursor down so the bottom row is visible and
    // the top row has scrolled out of the window.
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    for i in 0..20u64 {
        app.home
            .update
            .selection
            .local_collections
            .push(CollectionEntry {
                name: format!("coll-{i:02}"),
                collection_id: Some(i),
                beatmap_count: 1,
                selected: false,
                removed_count: 0,
            });
    }
    app.home.update.descend();
    app.home.update.selection.collections_cursor = Some(19);

    let content = render_content(&app, 120, 18);
    assert!(
        content.contains("coll-19"),
        "the focused bottom row must be visible in the scrolled window: {content}"
    );
    assert!(
        !content.contains("coll-00"),
        "the window must have scrolled down past the top row: {content}"
    );
}

#[test]
fn update_source_shows_client_toggle() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    let content = render_content(&app, 120, 40);
    // The header client chip shows either "lazer" or "stable" on any surface.
    assert!(content.contains("lazer") || content.contains("stable"));
}

// ── config view ──────────────────────────────────────────────────────────────

#[test]
fn config_tab_shows_auth_chip() {
    let mut app = make_app();
    app.next_tab();
    app.next_tab(); // home → downloads → config
    let content = render_content(&app, 120, 40);
    assert!(
        content.contains("signed out")
            || content.contains("signed in")
            || content.contains("log in")
            || content.contains("login unavailable"),
        "auth chip must render a visible auth state: {content}"
    );
}

// ── error / message footer ───────────────────────────────────────────────────

#[test]
fn footer_shows_hint_line() {
    let app = make_app();
    let content = render_content(&app, 120, 24);
    // footer should contain the hint line keys
    assert!(content.contains("move") || content.contains("quit") || content.contains("↑↓"));
}

#[test]
fn home_footer_hides_toggle_hint_on_text_input_focus() {
    use osu_collect::app::HomeField;

    let mut app = make_app();
    app.home.focus = HomeField::Collection;
    let content = render_content(&app, 120, 24);
    assert!(
        !content.contains("↵ toggle"),
        "toggle hint must be hidden while a text field is focused"
    );
}

#[test]
fn home_footer_shows_enter_toggle_on_toggle_focus() {
    use osu_collect::app::HomeField;

    let mut app = make_app();
    app.home.focus = HomeField::AutoOverwrite;
    let content = render_content(&app, 120, 24);
    assert!(content.contains("↵ toggle"));
}

#[test]
fn home_footer_shows_enter_download_on_button_focus() {
    use osu_collect::app::HomeField;

    let mut app = make_app();
    app.home.focus = HomeField::Download;
    let content = render_content(&app, 120, 24);
    assert!(content.contains("↵ download"));
}

#[test]
fn update_source_footer_hides_recheck_without_failed_maps() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    let content = render_content(&app, 120, 24);
    assert!(!content.contains("recheck"));
}

#[test]
fn update_source_footer_shows_recheck_with_failed_maps() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    // scan_status defaults to Idle (a "ready" state) so can_recheck is true.
    app.home.update.set_failed_beatmapset_count(1);
    let content = render_content(&app, 120, 24);
    assert!(
        content.contains("recheck"),
        "footer must surface the r recheck hint once maps are known bad"
    );
}

#[test]
fn update_browse_footer_shows_scroll_and_select_hints() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home
        .update
        .set_collections(vec![osu_collect::osu_db::LocalCollection {
            name: "coll - 1234".to_string(),
            beatmap_checksums: Vec::new().into(),
        }]);
    app.home.update.descend();
    let content = render_content(&app, 120, 24);
    assert!(
        content.contains("scroll"),
        "browse footer must show scroll hint"
    );
    assert!(
        content.contains("↵ toggle"),
        "browse footer must show ↵ toggle hint"
    );
    assert!(
        content.contains("all") && content.contains("none"),
        "browse footer must show select-all / select-none hint"
    );
    assert!(
        content.contains("preview"),
        "browse footer must show the → preview hint"
    );
    assert!(content.contains('?'), "browse footer must show ? help hint");
}

#[test]
fn config_footer_omits_space_on_text_input() {
    use osu_collect::app::Tab;
    use osu_collect::app::{ConfigField, HomeField};

    let mut app = make_app();
    // Focus a non-text field so Right switches tabs rather than moving the caret.
    app.home.focus = HomeField::Video;
    // Three static tabs: home → downloads → config.
    for _ in 0..2 {
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Right,
            crossterm::event::KeyModifiers::empty(),
        ));
    }
    assert_eq!(app.active_tab(), Tab::Config);
    app.config.focus = ConfigField::MirrorCustomUrl(0);

    let content = render_content(&app, 120, 24);
    assert!(!content.contains("space change"));
    assert!(!content.contains("↵ confirm"));
}

// ── footer hint count & content per context ──────────────────────────────────

/// Returns the content of the last rendered row (footer area).
fn render_footer_row(app: &App, width: u16, height: u16) -> String {
    let buf = render_to_buffer(app, width, height);
    let last_row = (height - 1) as usize;
    buf.content()
        .iter()
        .skip(last_row * width as usize)
        .take(width as usize)
        .map(|c| c.symbol())
        .collect()
}

fn hint_count(footer: &str) -> usize {
    // hint groups are 3-space separated (no glyph); count the inter-group gaps
    // + 1. Trim the trailing panel padding first so it isn't counted.
    footer.trim_end().matches("   ").count() + 1
}

#[test]
fn home_footer_toggle_focus_ends_with_help_then_quit() {
    use osu_collect::app::HomeField;

    let mut app = make_app();
    app.home.focus = HomeField::AutoOverwrite;
    let footer = render_footer_row(&app, 200, 24);
    assert!(footer.contains("↑↓"), "must show move hint");
    assert!(footer.contains("↵ toggle"), "must show ↵ toggle");
    assert!(footer.contains("q quit"), "must show q quit");
    assert!(footer.contains('?'), "must show ? help");
    assert!(
        footer.contains("switch client"),
        "must show c switch client"
    );
    // cloudy-tui order: `? help` then the back/quit key trail the bar.
    assert!(
        footer.find("? help") < footer.find("q quit"),
        "help must precede quit: {footer:?}"
    );
    assert!(footer.contains("source"), "must show the source-jump hint");
    assert_eq!(
        hint_count(&footer),
        6,
        "toggle focus must show move, toggle, source-jump, switch-client, help, quit"
    );
}

#[test]
fn home_footer_button_focus_shows_enter_download() {
    use osu_collect::app::HomeField;

    let mut app = make_app();
    app.home.focus = HomeField::Download;
    let footer = render_footer_row(&app, 200, 24);
    assert!(footer.contains("↑↓"), "must show move hint");
    assert!(footer.contains("↵ download"), "must show ↵ download");
    assert!(footer.contains("q quit"), "must show q quit");
    assert!(footer.contains('?'), "must show ? help");
    assert!(
        footer.contains("switch client"),
        "must show c switch client"
    );
    assert!(footer.contains("source"), "must show the source-jump hint");
    assert_eq!(
        hint_count(&footer),
        6,
        "button focus must show move, download, source-jump, quit, help, switch-client"
    );
}

#[test]
fn home_footer_text_input_focus_has_four_hints_with_edit_and_quit() {
    use osu_collect::app::HomeField;

    let mut app = make_app();
    app.home.focus = HomeField::Collection;
    let footer = render_footer_row(&app, 200, 24);
    assert!(footer.contains("↑↓"), "must show move hint");
    assert!(
        footer.contains("↵ edit"),
        "selected text input must show ↵ edit"
    );
    assert!(footer.contains('q'), "must show q quit");
    assert!(footer.contains('?'), "must show ? help");
    assert!(
        footer.contains("switch client"),
        "must show c switch client"
    );
    assert!(footer.contains("source"), "must show the source-jump hint");
    assert_eq!(
        hint_count(&footer),
        6,
        "selected text input must show move, edit, source-jump, quit, help, switch-client"
    );
}

#[test]
fn update_source_form_footer_lists_source_jump() {
    use osu_collect::app::{GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home.focus = HomeField::UpdateScan;
    let footer = render_footer_row(&app, 200, 24);
    assert!(footer.contains("↑↓"), "must show move hint");
    assert!(footer.contains("↵ scan"), "scan CTA focus shows ↵ scan");
    assert!(
        footer.contains("1-3 switch source"),
        "update form must advertise the strip-digit jump like the other sources: {footer}"
    );
    assert!(footer.contains('?'), "must show ? help");
    assert!(
        footer.contains("switch client"),
        "must show c switch client"
    );
    // move, scan, source-jump, switch-client, help, quit
    assert_eq!(
        hint_count(&footer),
        6,
        "update form footer must show move, scan, source-jump, switch-client, help, quit: {footer}"
    );
}

#[test]
fn update_browse_footer_lists_the_browse_keys() {
    use osu_collect::app::GetMapsSource;
    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home
        .update
        .set_collections(vec![osu_collect::osu_db::LocalCollection {
            name: "coll - 1234".to_string(),
            beatmap_checksums: Vec::new().into(),
        }]);
    app.home.update.descend();
    let footer = render_footer_row(&app, 200, 24);
    assert!(
        footer.contains("switch client"),
        "must show c switch client"
    );
    assert!(
        footer.contains("s sort"),
        "browse footer must advertise the `s sort` cycle: {footer}"
    );
    // scroll, toggle, all/none, sort, preview, switch-client, help
    assert_eq!(
        hint_count(&footer),
        7,
        "browse footer must show scroll, toggle, all/none, sort, preview, switch-client, help"
    );
}

#[test]
fn config_footer_non_text_has_four_hints_with_help() {
    use osu_collect::app::ConfigField;
    use osu_collect::app::Tab;

    let mut app = make_app();
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::DownloadVideo;
    let footer = render_footer_row(&app, 200, 24);
    assert!(footer.contains("↵ toggle"), "must show ↵ toggle");
    assert!(footer.contains("q quit"), "must show q quit");
    assert!(footer.contains('?'), "must show ? help");
    assert!(
        footer.contains("switch client"),
        "must show c switch client"
    );
    assert_eq!(
        hint_count(&footer),
        5,
        "config non-text footer must show exactly 5 hints"
    );
}

#[test]
fn config_footer_text_input_shows_edit_not_toggle() {
    use osu_collect::app::ConfigField;
    use osu_collect::app::Tab;

    let mut app = make_app();
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::MirrorCustomUrl(0);
    let footer = render_footer_row(&app, 200, 24);
    assert!(
        footer.contains("↵ edit"),
        "selected text field must show ↵ edit"
    );
    assert!(footer.contains("q quit"), "config footer must show q quit");
    assert!(footer.contains('?'), "text field must show ? help");
    assert!(
        !footer.contains("↵ toggle"),
        "text field must not show ↵ toggle"
    );
    assert!(
        footer.contains("switch client"),
        "must show c switch client"
    );
    assert_eq!(
        hint_count(&footer),
        5,
        "config text field footer must show move, edit, quit, help, switch-client"
    );

    // While editing, the footer collapses to the exit affordance.
    app.editing = true;
    let footer = render_footer_row(&app, 200, 24);
    assert!(
        footer.contains("esc done"),
        "editing config text field must show esc done"
    );
}

#[test]
fn download_tab_footer_shows_help_hint() {
    use osu_collect::app::{CollectionPage, Tab};

    let mut app = make_app();
    let page = CollectionPage::new(1, "test".to_string(), 1);
    app.downloads.push(page);
    app.active_tab = Tab::Downloads;
    app.downloads_tab.preview_focused = true;
    let footer = render_footer_row(&app, 200, 24);
    assert!(footer.contains('?'), "download tab footer must show ? help");
    assert!(
        footer.contains("scroll"),
        "download tab must show scroll hint"
    );
}

// ── gauge label ──────────────────────────────────────────────────────────────

#[test]
fn gauge_label_shows_avg_when_verified() {
    use osu_collect::app::CollectionPage;

    let mut page = CollectionPage::new(1, "test".to_string(), 1);
    page.total_maps = 10;
    page.stats.downloaded = 3;
    page.stats.skipped = 2;
    page.stats.verify_total_count = 5;
    page.stats.verify_total_us = 5_000_000;

    let avg = page.avg_verify_us();
    assert_eq!(avg, Some(1_000_000));
}

#[test]
fn gauge_bottom_row_shows_tally_left_and_verified_right() {
    use osu_collect::app::{CollectionPage, Tab};
    use osu_collect::download::DownloadStage;

    let mut app = make_app();
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Downloading;
    page.total_maps = 10;
    page.download_target = 10;
    page.stats.downloaded = 3;
    page.stats.skipped = 2;
    page.stats.failed = 1;
    app.downloads.push(page);
    app.active_tab = Tab::Downloads;
    app.downloads_tab.preview_focused = true;

    // Wide enough that the Downloads preview pane fits tally + verified.
    let buf = render_to_buffer(&app, 140, 24);
    // Find the single row carrying both the tally and the verified count.
    let row = (0..24u16)
        .map(|y| {
            (0..140u16)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .find(|r| r.contains("verified"))
        .expect("a gauge bottom row with the verified count must render");

    assert!(
        row.contains("3 downloaded") && row.contains("1 failed"),
        "tally must share the gauge bottom row: {row:?}"
    );
    assert!(
        row.contains("5/10 verified"),
        "verified count must share the gauge bottom row: {row:?}"
    );
    // Tally is left-aligned, verified is right-aligned: the tally precedes it.
    let tally_at = row.find("downloaded").expect("tally present");
    let verified_at = row.find("verified").expect("verified present");
    assert!(
        tally_at < verified_at,
        "tally (left) must precede verified (right): {row:?}"
    );
}

#[test]
fn gauge_drops_verified_count_when_too_narrow_for_tally() {
    use osu_collect::app::{CollectionPage, Tab};
    use osu_collect::download::DownloadStage;

    let mut app = make_app();
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Downloading;
    page.total_maps = 10;
    page.download_target = 10;
    page.stats.downloaded = 3;
    page.stats.skipped = 2;
    page.stats.failed = 1;
    app.downloads.push(page);
    app.active_tab = Tab::Downloads;
    app.downloads_tab.preview_focused = true;

    // The preview pane (~58 cols at this width) fits the ~53-col tally but not
    // tally + " 5/10 verified ", so the verified count is dropped and the
    // tally keeps the shared gauge bottom row.
    let content = render_content(&app, 100, 24);
    assert!(
        content.contains("downloaded") && content.contains("1 failed"),
        "the tally must still render at a narrow width: {content}"
    );
    assert!(
        !content.contains("verified"),
        "the verified count must be dropped when it would collide with the tally: {content}"
    );
}

#[test]
fn gauge_label_none_when_no_verified() {
    use osu_collect::app::CollectionPage;

    let page = CollectionPage::new(1, "test".to_string(), 1);
    assert_eq!(page.avg_verify_us(), None);
}

#[test]
fn gauge_label_none_when_avg_rounds_to_zero() {
    use osu_collect::app::CollectionPage;

    let mut page = CollectionPage::new(1, "test".to_string(), 1);
    page.stats.verify_total_count = 5;
    page.stats.verify_total_us = 0;
    assert_eq!(page.avg_verify_us(), None);
}

// ── help overlay render ───────────────────────────────────────────────────────

#[test]
fn help_overlay_renders_keybindings_heading() {
    let mut app = make_app();
    app.help_open = true;
    let content = render_content(&app, 120, 40);
    assert!(
        content.contains("KEYBINDINGS") || content.contains("keybindings"),
        "help overlay must render a KEYBINDINGS heading"
    );
}

#[test]
fn help_overlay_contains_question_mark_entry() {
    let mut app = make_app();
    app.help_open = true;
    let content = render_content(&app, 120, 40);
    assert!(content.contains('?'), "help overlay must show ? key");
}

#[test]
fn help_overlay_hidden_when_closed() {
    let app = make_app();
    // help_open defaults to false
    let content = render_content(&app, 120, 40);
    assert!(
        !content.contains("KEYBINDINGS"),
        "KEYBINDINGS heading must not appear when help is closed"
    );
}

// ── login split ───────────────────────────────────────────────────────────────

#[test]
fn login_split_docks_login_on_the_right_of_config() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use osu_collect::app::{AuthLoginState, ConfigField};

    let mut app = make_app();
    // Pin logged-out so the panel opens on the credentials phase regardless of
    // any osu! token on the host.
    app.config.login_state = AuthLoginState::LoggedOut;
    app.next_tab();
    app.next_tab(); // home → downloads → config
    app.config.focus = ConfigField::AuthChip;
    app.handle_key(KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    });
    assert!(
        app.login_open(),
        "enter on the auth chip opens the login split"
    );

    let (w, h) = (120u16, 30u16);
    let buf = render_to_buffer(&app, w, h);
    let cells = buf.content();
    let mut config_x = None;
    let mut login_x = None;
    for y in 0..h {
        let row: String = (0..w)
            .map(|x| cells[(y * w + x) as usize].symbol())
            .collect();
        if let Some(i) = row.find("CONFIG") {
            config_x.get_or_insert(i as u16);
        }
        if let Some(i) = row.find("LOGIN") {
            login_x.get_or_insert(i as u16);
        }
    }
    let config_x = config_x.expect("config panel still renders on the left");
    let login_x = login_x.expect("login panel renders");
    assert!(config_x < w / 2, "config panel keeps the left half");
    assert!(login_x > w / 2, "login panel docks on the right half");
}

#[test]
fn login_split_info_lines_wrap_instead_of_clipping() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use osu_collect::app::{AuthLoginState, ConfigField};

    let mut app = make_app();
    app.config.login_state = AuthLoginState::LoggedOut;
    app.next_tab();
    app.next_tab(); // home → downloads → config
    app.config.focus = ConfigField::AuthChip;
    app.handle_key(KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    });
    assert!(app.login_open());

    // At 120 cols the login panel is ~48 wide, so the credentials note (62
    // chars) used to hard-clip at the border. Wrapped, its tail survives.
    let content = render_content(&app, 120, 24);
    assert!(
        content.contains("stored locally"),
        "the credentials note must wrap, not clip: {content}"
    );
}

// ── config item order ─────────────────────────────────────────────────────────

#[test]
fn config_tab_shows_mirrors_section_before_download() {
    let mut app = make_app();
    app.next_tab();
    app.next_tab(); // home → downloads → config
    let content = render_content(&app, 120, 60);
    // both sections should be present
    assert!(content.contains("download") || content.contains("DOWNLOAD"));
    assert!(content.contains("mirrors") || content.contains("MIRRORS"));
    // mirrors render before download, matching the home tab's section flow.
    // Section headers render UPPERCASE; the lowercase `downloads` tab title
    // and the display section's `jump to downloads on start` row never match
    // the uppercase anchor, so only a bare `DOWNLOAD` counts as the header.
    let mir_pos = content.find("MIRRORS");
    let dl_pos = content
        .match_indices("DOWNLOAD")
        .map(|(i, _)| i)
        .find(|&i| !content[i..].starts_with("DOWNLOADS"));
    let m = mir_pos.expect("mirrors section header must render at 120x60");
    let d = dl_pos.expect("download section header must render at 120x60");
    assert!(m < d, "mirrors section should render before download");
}

// ── enrichment loading cues ──────────────────────────────────────────────────

#[test]
fn collection_browse_opens_id_only_immediately_on_enter() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use osu_collect::app::{GetMapsSource, HomeField};

    let mut app = make_app();
    app.home.source = GetMapsSource::Collection;
    app.home.set_resolved_collection(7, vec![10, 20, 30]);
    // Unenriched (set, diff) pairs so the open still has titles to page.
    app.home.resolved_enrich_pairs = vec![(10, 100), (20, 200), (30, 300)];
    app.home.focus = HomeField::CollectionBrowse;
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert!(
        app.home.collection_browse.is_browsing(),
        "the browse descends immediately, id-only — no deferred wait on enrichment"
    );

    let content = render_content(&app, 120, 40);
    assert!(
        !content.contains("opening"),
        "the deferred `opening` spinner is gone: {content}"
    );
    assert!(
        content.contains("#10"),
        "the browse renders id-only rows until titles land: {content}"
    );
}

#[test]
fn update_view_button_trails_loading_titles_cue_while_enriching() {
    use osu_collect::app::update_source::{MissingBeatmapset, MissingStatus};
    use osu_collect::app::{EnrichSink, GetMapsSource, HomeField};

    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    // `set_missing_beatmaps` seeds the pager at scan-land; dispatch its first
    // page manually to simulate a fetch still in flight.
    app.home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 10,
            status: MissingStatus::NotInstalled,
            collection_id: 100,
            collection_name: "test - 100".to_string(),
            selected: true,
            previously_deleted: false,
            enrich_diff_id: Some(1000),
        }],
        &std::collections::HashMap::new(),
    );
    let _ = app.home.update.next_enrich_page();
    app.home.update.mark_enrichment_dispatched();
    assert!(app.home.update.is_enriching());

    app.home.focus = HomeField::UpdateBrowse;
    let content = render_content(&app, 100, 26);
    assert!(
        content.contains("view 1 mapset"),
        "the button stays pressable (labelled) mid-fetch: {content}"
    );
    assert!(
        content.contains("⠋ loading titles"),
        "the update view button trails a loading-titles cue while enrichment is in flight: {content}"
    );
}

#[test]
fn find_view_button_trails_loading_titles_cue_while_enriching() {
    use osu_collect::app::{BrowseRow, EnrichSink, GetMapsSource, HomeField};

    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.find.browse.set_rows(
        vec![
            BrowseRow { id: 1, meta: None },
            BrowseRow { id: 2, meta: None },
        ],
        &std::collections::HashMap::new(),
    );
    app.home.find.mark_results_current();
    app.home.find.browse.seed_enrichment(
        vec![(101, Some(1)), (102, Some(2))],
        &std::collections::HashMap::new(),
    );
    let _ = app.home.find.browse.next_enrich_page();
    app.home.find.browse.mark_enrichment_dispatched();
    assert!(app.home.find.browse.is_enriching());

    app.home.focus = HomeField::FindBrowse;
    let content = render_content(&app, 80, 26);
    assert!(
        content.contains("view 2 maps"),
        "the button stays pressable (labelled) mid-fetch: {content}"
    );
    assert!(
        content.contains("⠋ loading titles"),
        "the find view button trails a loading-titles cue while enrichment is in flight: {content}"
    );
}

#[test]
fn find_results_list_pane_appends_loading_titles_cue_to_the_ratio() {
    use osu_collect::app::{BrowseRow, EnrichSink, GetMapsSource};

    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.find.browse.set_rows(
        vec![BrowseRow { id: 42, meta: None }],
        &std::collections::HashMap::new(),
    );
    app.home
        .find
        .browse
        .seed_enrichment(vec![(101, Some(42))], &std::collections::HashMap::new());
    let _ = app.home.find.browse.next_enrich_page();
    app.home.find.browse.mark_enrichment_dispatched();
    app.home.find.browse.descend();
    assert!(app.home.find.browse.is_enriching());

    let content = render_content(&app, 120, 30);
    assert!(
        content.contains("0/1"),
        "the list pane keeps its selected/total ratio: {content}"
    );
    assert!(
        content.contains("⠋ loading titles"),
        "the results list pane appends a loading-titles cue while enrichment is in flight: {content}"
    );
}

#[test]
fn find_results_row_stays_bare_id_with_no_per_row_spinner_while_enriching() {
    use osu_collect::app::{BrowseRow, EnrichSink, GetMapsSource};

    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.find.browse.set_rows(
        vec![BrowseRow { id: 42, meta: None }],
        &std::collections::HashMap::new(),
    );
    app.home
        .find
        .browse
        .seed_enrichment(vec![(101, Some(42))], &std::collections::HashMap::new());
    let _ = app.home.find.browse.next_enrich_page();
    app.home.find.browse.mark_enrichment_dispatched();
    app.home.find.browse.descend();
    assert!(app.home.find.browse.is_enriching());

    let (width, height) = (120u16, 30u16);
    let buf = render_to_buffer(&app, width, height);
    let row = (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .find(|r| r.contains("#42"))
        .expect("the id-only row renders");
    assert!(
        !row.contains('⠋'),
        "the row itself carries no per-row spinner (only the panel title does): {row:?}"
    );
}

#[test]
fn update_preview_pane_appends_loading_titles_cue_without_dropping_counts() {
    use osu_collect::app::update_source::{MissingBeatmapset, MissingStatus};
    use osu_collect::app::{EnrichSink, GetMapsSource};
    use osu_collect::osu_db::LocalCollection;

    let mut app = make_app();
    app.home.source = GetMapsSource::Update;
    app.home.update.set_collections(vec![LocalCollection {
        name: "test - 100".to_string(),
        beatmap_checksums: Vec::new().into(),
    }]);
    app.home.update.set_missing_beatmaps(
        vec![MissingBeatmapset {
            id: 10,
            status: MissingStatus::NotInstalled,
            collection_id: 100,
            collection_name: "test - 100".to_string(),
            selected: true,
            previously_deleted: false,
            enrich_diff_id: Some(1000),
        }],
        &std::collections::HashMap::new(),
    );
    let _ = app.home.update.next_enrich_page();
    app.home.update.mark_enrichment_dispatched();
    app.home.update.descend();
    assert!(app.home.update.is_enriching());

    let content = render_content(&app, 120, 30);
    assert!(
        content.contains("1 new"),
        "the preview title-right meta keeps its new/removed counts: {content}"
    );
    assert!(
        content.contains("⠋ loading titles"),
        "the preview title appends a loading-titles cue while enrichment is in flight: {content}"
    );
}

#[test]
fn find_download_button_shows_approx_size_for_checked_osu_results() {
    use osu_collect::app::{BrowseRow, FindBackend, GetMapsSource, HomeField};
    let mut app = make_app();
    app.home.source = GetMapsSource::Find;
    app.home.find.browse.set_rows(
        vec![
            BrowseRow { id: 1, meta: None },
            BrowseRow { id: 2, meta: None },
        ],
        &std::collections::HashMap::new(),
    );
    // osu-routed results, both checked, with two landed nekoha probes.
    app.home.find.note_results_backend(FindBackend::Osu);
    app.home.find.browse.set_all_selected(true);
    app.home.find.record_size(1, Some(20 * 1024 * 1024));
    app.home.find.record_size(2, Some(30 * 1024 * 1024));
    // Focus the button so it scrolls into view in the long merged find form.
    app.home.focus = HomeField::Download;
    let content = render_content(&app, 80, 30);
    // The button reads `download (2) · ~50.0 MiB` (summed known sizes; `~` = approx).
    assert!(
        content.contains("download (2) · ~50.0 MiB"),
        "osu find button shows the summed approx size: {content}"
    );
}
