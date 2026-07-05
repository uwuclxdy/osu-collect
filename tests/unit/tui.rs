use crate::{
    app::{App, CollectionPage, ConfigField, Tab},
    config::Config,
    download::{DownloadEvent, DownloadStage, DownloadSummary},
};
use ratatui::{Terminal, backend::TestBackend, style::Color};

fn render_buffer(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");
    terminal
        .draw(|frame| {
            super::draw(frame, app);
        })
        .expect("app should render");
    terminal.backend().buffer().clone()
}

fn render_app(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");
    terminal
        .draw(|frame| {
            super::draw(frame, app);
        })
        .expect("app should render");

    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

/// Positions of cells matching `symbol` drawn in `color`. Used to locate the
/// bracketed bouncing-block bar's filled chunk (`█` in the bar color) and frame
/// (`[`/`]` in the recessed line color).
fn cells_matching(buf: &ratatui::buffer::Buffer, symbol: &str, color: Color) -> Vec<(u16, u16)> {
    buf.content
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.symbol() == symbol && cell.style().fg == Some(color))
        .map(|(i, _)| {
            let x = (i as u16) % buf.area.width;
            let y = (i as u16) / buf.area.width;
            (x, y)
        })
        .collect()
}

#[test]
fn home_render_shows_sections_and_footer() {
    use crate::app::HomeField;

    let mut app = App::new(Config::default());
    // focus a toggle option so the footer hint exposes the toggle shortcut
    app.home.focus = HomeField::AutoOverwrite;

    let output = render_app(&app, 80, 24);

    assert!(output.contains("home"));
    assert!(
        !output.contains("[ home ]"),
        "active tab must not use brackets"
    );
    assert!(output.contains("COLLECTION"));
    assert!(output.contains("MIRRORS"));
    assert!(output.contains("DOWNLOAD"));
    // footer renders `enter` as the glyph `↵`, never the word
    assert!(output.contains("↵ toggle"));
    assert!(!output.contains("enter toggle"), "enter must render as ↵");
}

#[test]
fn home_mirrors_summary_shows_enabled_count() {
    use crate::app::HomeField;

    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Mirrors;

    let output = render_app(&app, 80, 24);

    // Default config enables 7 built-in mirrors; the collapsed row reports it
    // instead of listing every mirror toggle.
    assert!(
        output.contains("mirrors") && output.contains("7 enabled"),
        "mirrors summary must show the enabled count: {output}"
    );
}

#[test]
fn home_directory_tooltip_shows_resolved_path_only_when_focused() {
    use crate::app::HomeField;

    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Directory;
    app.home.directory.set_value("~/osu-collect-tooltip-test");

    // Before a collection resolves, the tooltip shows the base dir plus a
    // per-collection folder placeholder.
    let output = render_app(&app, 80, 24);
    assert!(
        output.contains("downloads to")
            && output.contains("osu-collect-tooltip-test")
            && output.contains("<collection>"),
        "unresolved tooltip must show the base path with a per-collection placeholder: {output}"
    );

    // Once resolved, the placeholder is replaced by the real per-collection folder.
    app.home.resolved_folder_name = Some("MyCollection-123".to_string());
    let output = render_app(&app, 80, 24);
    assert!(
        output.contains("MyCollection-123") && !output.contains("<collection>"),
        "resolved tooltip must show the actual per-collection folder: {output}"
    );

    // The tooltip is focus-scoped: it disappears once another field has focus.
    app.home.focus = HomeField::Threads;
    let output = render_app(&app, 80, 24);
    assert!(
        !output.contains("downloads to"),
        "directory tooltip must only render while the directory field is focused"
    );
}

#[test]
fn config_render_scrolls_to_focused_logging_field() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::LoggingDirectory;

    // height=20 keeps the render in normal mode (content area > 12)
    // so section headers are visible and the scroll-to-focus assertion holds.
    let output = render_app(&app, 40, 20);

    assert!(output.contains("LOGGING"));
    assert!(output.contains("logs directory"));
}

#[test]
fn config_render_shows_download_help() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::DownloadThreads;

    let output = render_app(&app, 80, 20);

    assert!(output.contains("DOWNLOAD"));
    assert!(output.contains("verify .osz integrity"));
    assert!(
        output.contains("off") && output.contains("basic") && output.contains("strict"),
        "all three archive validation labels must render: {output}"
    );
}

#[test]
fn config_render_shows_auto_defer_label() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    // Focus the row so the scrollable panel brings it (and its secs input) into view.
    app.config.focus = ConfigField::DownloadAutoSkipRateLimited;

    let output = render_app(&app, 80, 20);

    assert!(
        output.contains("auto-defer rate limited"),
        "auto-defer toggle label must render: {output}"
    );
    assert!(
        output.contains("defer after (secs)"),
        "the delay input must be worded as a defer: {output}"
    );
}

#[test]
fn config_render_shows_strict_help_only_when_strict_selected() {
    use crate::download::ArchiveValidation;

    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::DownloadArchiveValidation;

    app.config.archive_validation = ArchiveValidation::Magic;
    let basic = render_app(&app, 100, 24);
    assert!(
        !basic.contains("eocd"),
        "the strict-only help line must be hidden when basic is selected"
    );

    app.config.archive_validation = ArchiveValidation::Eocd;
    let strict = render_app(&app, 100, 24);
    assert!(
        strict.contains("eocd"),
        "the strict help line must appear when strict is selected: {strict}"
    );
}

#[test]
fn config_render_shows_auth_chip() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::AuthChip;

    let output = render_app(&app, 80, 24);

    assert!(
        output.contains("signed out")
            || output.contains("signed in")
            || output.contains("login unavailable"),
        "auth chip must render auth state: {output}"
    );
    assert!(!output.contains("client id:"));
    assert!(!output.contains("client secret:"));
}

#[test]
fn download_render_shows_status_metrics_and_results() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 2);
    page.stage = DownloadStage::Completed;
    page.total_maps = 10;
    page.download_target = 10;
    page.stats.downloaded = 8;
    page.stats.skipped = 2;
    page.summary = Some(DownloadSummary {
        downloaded: 8,
        skipped: 2,
        failed: 0,
        unverified: 0,
    });
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 90, 24);

    assert!(output.contains("completed"));
    // The OVERVIEW `settings: N threads` row and the `done:` tally are gone
    // post-completion; the RESULTS block carries the counts as lowercase
    // `label value` metrics.
    assert!(output.contains("downloaded 8"));
    assert!(output.contains("skipped 2"));
}

#[test]
fn confirm_retry_modal_renders_focused_button_block() {
    // Confirm modals carry right-aligned buttons; the focused one is a neutral
    // inverse block (fg=BG, bg=TEXT), unfocused are bare TEXT_DIM.
    // focus=1 is `retry`.
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");
    terminal
        .draw(|frame| {
            let area = frame.area();
            super::modal::render_confirm_retry_modal(frame, area, 12, 1);
        })
        .expect("modal should render");
    let buf = terminal.backend().buffer().clone();
    let text: String = buf.content.iter().map(|cell| cell.symbol()).collect();
    assert!(
        text.contains("cancel") && text.contains("retry"),
        "both buttons must render, got: {text}"
    );

    // The focused `retry` button renders as an inverse block with bg = TEXT.
    let text_color = Color::Rgb(205, 214, 244);
    let has_inverse = buf
        .content
        .iter()
        .any(|cell| cell.style().bg == Some(text_color));
    assert!(
        has_inverse,
        "focused button must render as an inverse TEXT-bg block"
    );
}

#[test]
fn confirm_retry_modal_is_content_sized_not_terminal_sized() {
    // A modal shrinks to its content and never grows past 60% of the terminal.
    // On a very wide terminal it must stay narrow, not balloon.
    let width = 200u16;
    let backend = TestBackend::new(width, 24);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");
    terminal
        .draw(|frame| {
            let area = frame.area();
            super::modal::render_confirm_retry_modal(frame, area, 12, 1);
        })
        .expect("modal should render");
    let buf = terminal.backend().buffer().clone();

    // Measure the modal by the horizontal span of its border glyphs (only the
    // modal is drawn here, so these are unambiguous).
    let is_border = |s: &str| matches!(s, "╭" | "╮" | "╰" | "╯" | "│" | "─");
    let buf_width = buf.area.width as usize;
    let (mut min_x, mut max_x) = (usize::MAX, 0usize);
    for (i, cell) in buf.content.iter().enumerate() {
        if is_border(cell.symbol()) {
            let x = i % buf_width;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
        }
    }
    assert_ne!(min_x, usize::MAX, "modal border must render");
    let modal_w = max_x - min_x + 1;
    assert!(
        modal_w <= (width as usize * 60 / 100),
        "modal must not exceed 60% of terminal width, got {modal_w}"
    );
    assert!(
        modal_w < 40,
        "modal must be content-sized (~25 cols), not terminal-sized, got {modal_w}"
    );
}

#[test]
fn modal_buttons_do_not_shift_on_focus_change() {
    fn render(focus: usize) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).expect("test backend should initialize");
        terminal
            .draw(|frame| {
                let area = frame.area();
                super::modal::render_confirm_retry_modal(frame, area, 12, focus);
            })
            .expect("modal should render");
        terminal.backend().buffer().clone()
    }
    let cancel_focused = render(0);
    let retry_focused = render(1);

    // Glyph layout is byte-for-byte identical regardless of which button is
    // focused — every button always carries its insets, so only the highlight
    // moves and no button shifts position.
    let glyphs = |b: &ratatui::buffer::Buffer| -> String {
        b.content.iter().map(|c| c.symbol().to_string()).collect()
    };
    assert_eq!(
        glyphs(&cancel_focused),
        glyphs(&retry_focused),
        "button glyph positions must not change when focus changes"
    );

    // The inverse highlight (bg = TEXT) exists in both states but in different
    // cells (it follows the focused button).
    let text_bg = Color::Rgb(205, 214, 244);
    let bg_cells = |b: &ratatui::buffer::Buffer| -> Vec<usize> {
        b.content
            .iter()
            .enumerate()
            .filter(|(_, c)| c.style().bg == Some(text_bg))
            .map(|(i, _)| i)
            .collect()
    };
    let cancel_hl = bg_cells(&cancel_focused);
    let retry_hl = bg_cells(&retry_focused);
    assert!(
        !cancel_hl.is_empty() && !retry_hl.is_empty(),
        "each focus state highlights one button"
    );
    assert_ne!(cancel_hl, retry_hl, "the highlight moves with focus");
}

#[test]
fn active_tab_has_accent_color_no_brackets_and_plain_bg() {
    let app = App::new(Config::default());
    let buf = render_buffer(&app, 80, 24);
    let accent = Color::Rgb(67, 171, 229);
    let bg = Color::Rgb(30, 30, 46);

    // active tab text ("home") must use accent (blue)
    let has_accent_h = buf
        .content
        .iter()
        .any(|cell| cell.symbol() == "h" && cell.style().fg == Some(accent));
    assert!(has_accent_h, "active tab 'home' must render with accent fg");

    // The only brackets allowed in the header are the client chip
    // (`[ stable ]` / `[ lazer ]`); tab titles must never be bracket-wrapped
    // (the retired `[home]` style).
    let header_row: String = buf
        .content
        .iter()
        .take(80)
        .map(|cell| cell.symbol())
        .collect();
    let chip = format!("[ {} ]", app.library.client_type.label());
    let without_chip = header_row.replacen(&chip, "", 1);
    assert!(
        !without_chip.contains('[') && !without_chip.contains(']'),
        "only the client chip may use brackets in the header row"
    );

    // header area (row 0) must use plain BG, not BG_RAISED
    let raised_bg = Color::Rgb(24, 24, 37);
    let header_has_raised = buf
        .content
        .iter()
        .take(80)
        .any(|cell| cell.style().bg == Some(raised_bg) && cell.style().bg != Some(bg));
    assert!(!header_has_raised, "header row must not use BG_RAISED fill");
}

#[test]
fn section_titles_use_text_dim() {
    // section_header eyebrow rows render in TEXT_DIM (no bold, no orange accent).
    let text_dim = Color::Rgb(166, 173, 200);

    // home tab: COLLECTION section header
    let app = App::new(Config::default());
    let buf = render_buffer(&app, 120, 30);
    let has_dim_c = buf
        .content
        .iter()
        .any(|cell| cell.symbol() == "C" && cell.style().fg == Some(text_dim));
    assert!(has_dim_c, "COLLECTION section header must be text_dim");

    // config tab: MIRRORS section header
    let mut app2 = App::new(Config::default());
    app2.active_tab = Tab::Config;
    let buf2 = render_buffer(&app2, 120, 30);
    let has_dim_m = buf2
        .content
        .iter()
        .any(|cell| cell.symbol() == "M" && cell.style().fg == Some(text_dim));
    assert!(
        has_dim_m,
        "MIRRORS section header must be text_dim in config tab"
    );
}

#[test]
fn spinner_advances_with_tick_count() {
    use crate::app::messages::AppMessage;

    let mut app = App::new(Config::default());
    app.home.message = Some(AppMessage {
        text: "loading...".to_string(),
    });

    let buf0 = render_buffer(&app, 80, 24);
    // grab cells in the footer row (last row at y=23)
    let spinner0: String = buf0
        .content
        .iter()
        .skip(80 * 23)
        .take(5)
        .map(|c| c.symbol())
        .collect();

    app.tick_count = 1;
    let buf1 = render_buffer(&app, 80, 24);
    let spinner1: String = buf1
        .content
        .iter()
        .skip(80 * 23)
        .take(5)
        .map(|c| c.symbol())
        .collect();

    // the spinner character in the footer must differ between tick 0 and tick 1
    assert_ne!(spinner0, spinner1, "spinner must advance with tick_count");
}

#[test]
fn letter_key_on_config_toggle_row_is_inert() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    // focus is a mirror toggle by default — not the auth chip.
    let key = KeyEvent {
        code: KeyCode::Char('l'),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };
    let cmd = app.handle_key(key);
    assert!(
        cmd.is_none(),
        "a letter on a config toggle row issues no command"
    );
    assert!(app.login.is_none(), "a letter must not open the login tab");
}

#[test]
fn auth_chip_is_reachable_via_focus_cycle() {
    use crate::app::ConfigField;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;

    let down = KeyEvent {
        code: KeyCode::Down,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };

    // cycle through all fields and check that AuthChip is reachable
    // (bound is comfortably above the field count; the loop breaks on reach)
    let mut found_chip = false;
    for _ in 0..50 {
        if app.config.focus == ConfigField::AuthChip {
            found_chip = true;
            break;
        }
        app.handle_key(down);
    }
    assert!(
        found_chip,
        "AuthChip field must be reachable via down-arrow navigation"
    );
}

#[test]
fn auth_chip_renders_when_focused() {
    use crate::app::ConfigField;

    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::AuthChip;

    let output = render_app(&app, 120, 30);
    assert!(
        output.contains("signed out")
            || output.contains("signed in")
            || output.contains("logging in")
            || output.contains("login unavailable"),
        "AuthChip must render a visible auth state label"
    );
}

#[test]
fn active_view_renders_progress_bar_when_downloading() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 1);
    page.stage = DownloadStage::Downloading;
    page.total_maps = 5;
    page.download_target = 5;
    page.update_active_status(
        42,
        crate::download::BeatmapStage::Downloading,
        "Downloading #42 from mirror",
        false,
        None,
    );
    page.update_active_progress(42, 5_000_000, 10_000_000);
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 100, 24);

    assert!(output.contains("█"), "filled bar segment must render");
    assert!(output.contains("░"), "empty bar segment must render");
    assert!(
        output.contains("50%"),
        "percent label must reflect progress"
    );
}

#[test]
fn active_view_requires_percentage_for_discovered_download_size() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 1);
    page.stage = DownloadStage::Downloading;
    page.total_maps = 5;
    page.download_target = 5;
    page.update_active_status(
        42,
        crate::download::BeatmapStage::Downloading,
        "Downloading #42 from mirror",
        false,
        None,
    );
    page.update_active_progress(42, 1_500_000, 10_000_000);
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 100, 24);

    assert!(output.contains("█"), "filled bar segment must render");
    assert!(
        output.contains("15%"),
        "percent label must reflect probed size"
    );
    assert!(!output.contains("  --"), "progress must render as percent");
}

#[test]
fn active_view_renders_bouncing_bar_when_total_is_unknown() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 1);
    page.stage = DownloadStage::Downloading;
    page.total_maps = 5;
    page.download_target = 5;
    page.update_active_status(
        42,
        crate::download::BeatmapStage::Downloading,
        "Downloading #42 from mirror",
        false,
        None,
    );
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 100, 24);

    assert!(output.contains("████"), "bouncing segment must render");
    assert!(output.contains("..."), "unknown progress label must render");
    assert!(
        !output.contains("  --"),
        "unknown progress must not render --"
    );
}

#[test]
fn active_panel_drops_finished_and_idle_rows() {
    use crate::download::BeatmapStage;

    fn count_id_prefixes(output: &str) -> usize {
        output.matches("  #").count()
    }

    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked".into(), 3);
    page.stage = DownloadStage::Downloading;
    page.total_maps = 10;
    page.download_target = 10;
    for id in [10u32, 11, 12] {
        page.update_active_status(
            id,
            BeatmapStage::Downloading,
            &format!("Downloading #{id} from mirror"),
            false,
            None,
        );
    }
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let baseline = render_app(&app, 120, 30);
    assert_eq!(
        count_id_prefixes(&baseline),
        3,
        "three active downloads render"
    );

    // Completing the middle slot removes its row outright — a finished download
    // is no longer downloading, so it must not linger with a "done" message. Text
    // is debounced, so wait past the window before rendering.
    app.downloads[0].update_active_status(11, BeatmapStage::Success, "done", false, None);
    std::thread::sleep(std::time::Duration::from_millis(75));
    let after_complete = render_app(&app, 120, 30);
    assert_eq!(
        count_id_prefixes(&after_complete),
        2,
        "a finished download's row is removed from the active list"
    );
    assert!(
        !after_complete.contains("done"),
        "the finished row's terminal message must not linger"
    );
    assert!(after_complete.contains("#10") && after_complete.contains("#12"));
    assert!(!after_complete.contains("#11"), "finished id must be gone");

    // A fresh download claims the freed slot and a row reappears.
    app.downloads[0].update_active_status(
        99,
        BeatmapStage::Downloading,
        "Downloading #99 from mirror",
        false,
        None,
    );
    let after_refill = render_app(&app, 120, 30);
    assert_eq!(count_id_prefixes(&after_refill), 3);
    assert!(
        after_refill.contains("#99"),
        "new beatmapset takes the freed slot"
    );

    // Clearing every slot leaves no active rows at all.
    app.downloads[0].clear_active_downloads();
    let after_clear = render_app(&app, 120, 30);
    assert_eq!(count_id_prefixes(&after_clear), 0, "no active rows remain");
}

#[test]
fn long_message_does_not_drop_the_progress_bar() {
    use crate::download::BeatmapStage;

    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked".into(), 1);
    page.stage = DownloadStage::Downloading;
    page.total_maps = 1;
    page.download_target = 1;
    page.update_active_status(
        42,
        BeatmapStage::Downloading,
        "retrying nerinyan-extra-long-mirror-name after Connection timed out (attempt 3/3)",
        false,
        None,
    );
    page.update_active_progress(42, 7_000_000, 10_000_000);
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 60, 24);
    assert!(
        output.contains("█") && output.contains("░"),
        "bar must remain visible even when the message would otherwise overflow"
    );
    assert!(
        output.contains("70%"),
        "percent label must still render after truncation"
    );
}

#[test]
fn active_view_shows_bar_for_active_download_regardless_of_message() {
    // The bar is keyed on the beatmap's `BeatmapStage`, not on the message string, so
    // sub-state transitions (retrying, mirror failed, verifying) keep the bar visible
    // and don't flicker on every status update.
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 1);
    page.stage = DownloadStage::Downloading;
    page.total_maps = 5;
    page.download_target = 5;
    page.update_active_status(
        42,
        crate::download::BeatmapStage::Downloading,
        "retrying nerinyan after timeout (attempt 2/3)",
        false,
        None,
    );
    page.update_active_progress(42, 3_000_000, 6_000_000);
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 100, 24);

    assert!(output.contains("retrying"), "transient status must render");
    assert!(
        output.contains("█") && output.contains("░"),
        "bar must remain visible across in-flight sub-states"
    );
    assert!(
        output.contains("50%"),
        "percent label must reflect latest progress"
    );
}

#[test]
fn rechecking_stage_shows_verification_progress() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Rechecking;
    page.total_maps = 10;
    page.download_target = 10;
    page.stats.skipped = 3;
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 100, 24);

    assert!(
        output.contains("3/10 verified"),
        "gauge must show running verification count during rechecking"
    );
}

#[test]
fn rechecking_stage_replaces_top_title_with_recheck_progress() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Rechecking;
    page.total_maps = 10;
    page.download_target = 10;
    page.stats.skipped = 3;
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 100, 24);

    assert!(
        output.contains("rechecking 3/10"),
        "gauge top title must show recheck progress during rechecking"
    );
    // The downloaded/queued/skipped/failed tally rides the gauge's bottom title
    // row (left-aligned, opposite the right-aligned verified count). During
    // rechecking the gauge keeps its top recheck title and this bottom tally.
    assert!(
        output.contains("queued"),
        "gauge tally must render the queued count during rechecking"
    );
}

#[test]
fn rechecking_stage_threads_panel_shows_verification_status() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Rechecking;
    page.total_maps = 10;
    page.download_target = 10;
    page.stats.skipped = 2;
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 100, 24);

    assert!(
        output.contains("verifying existing archives"),
        "threads panel must show verification status during rechecking"
    );
    assert!(
        !output.contains("no active threads"),
        "rechecking must replace the idle-threads placeholder"
    );
}

#[test]
fn resolving_stage_renders_indeterminate_gauge_and_status() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Resolving;
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 100, 24);

    assert!(
        output.contains("resolving collection"),
        "gauge title must announce the resolving stage"
    );
    assert!(
        output.contains("fetching collection metadata"),
        "threads panel must replace the idle placeholder while resolving"
    );
    assert!(
        !output.contains("0 downloaded  0 queued"),
        "downloaded/queued label must not appear while resolving"
    );
    assert!(
        !output.contains("0/1 verified"),
        "verified label must not appear while resolving"
    );
}

#[test]
fn resolving_stage_with_progress_shows_count_in_title() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Resolving;
    page.resolve_progress = Some((2, 5));
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 100, 24);

    assert!(
        output.contains("resolving 2/5 collections"),
        "gauge title must reflect resolve progress"
    );
}

#[test]
fn resolving_stage_indeterminate_bar_is_bracketed_block() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Resolving;
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let buf = render_buffer(&app, 100, 24);
    let frame_color = super::line();
    let bar_color = super::info();

    // The canonical bouncing-block bar draws a `[ … ]` frame in the recessed
    // line color and a short `█` chunk in the bar (info) color inside it —
    // the same widget the per-row mini-bar renders.
    let open = cells_matching(&buf, "[", frame_color);
    let close = cells_matching(&buf, "]", frame_color);
    let block = cells_matching(&buf, "█", bar_color);

    let (open_x, open_y) = *open.first().expect("bar must render a `[` frame cell");
    assert!(
        close.iter().any(|&(_, y)| y == open_y),
        "bar must render a `]` frame cell on the same row as `[`"
    );
    // GAUGE_H_MARGIN insets the bar by one column, so the `[` sits at x=1.
    assert_eq!(open_x, 1, "bracket frame must start at the bar's left edge");
    assert!(
        !block.is_empty(),
        "bar must render a filled `█` chunk inside the frame"
    );
    let close_x = close
        .iter()
        .filter(|&&(_, y)| y == open_y)
        .map(|&(x, _)| x)
        .max()
        .expect("`]` on the bar row");
    assert!(
        block
            .iter()
            .all(|&(x, y)| y == open_y && x > open_x && x < close_x),
        "the filled chunk must travel inside the `[ … ]` frame"
    );
}

#[test]
fn resolving_stage_indeterminate_block_stays_inside_frame() {
    // The bouncing block is time-driven (one global clock); across renders the
    // chunk stays inside the frame and the frame stays put — no claim of
    // determinate progress.
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Resolving;
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let buf = render_buffer(&app, 100, 24);
    let frame_color = super::line();
    let bar_color = super::info();

    let open = cells_matching(&buf, "[", frame_color);
    let close = cells_matching(&buf, "]", frame_color);
    let block = cells_matching(&buf, "█", bar_color);

    assert_eq!(open.len(), 1, "exactly one `[` frame cell");
    assert_eq!(close.len(), 1, "exactly one `]` frame cell");
    let span = block.len();
    assert!(
        (1..=4).contains(&span),
        "the moving chunk is a short fixed-width block, got {span} cells"
    );
}

#[test]
fn resolving_stage_progress_bar_renders_single_row() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Resolving;
    page.resolve_progress = Some((2, 5));
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let buf = render_buffer(&app, 100, 24);
    let info = Color::Rgb(116, 199, 236);

    let rows_with_info_fill: std::collections::BTreeSet<u16> = buf
        .content
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.symbol() == "█" && cell.style().fg == Some(info))
        .map(|(i, _)| (i as u16) / buf.area.width)
        .collect();

    assert_eq!(
        rows_with_info_fill.len(),
        1,
        "resolving bar must be exactly 1 row thick, got rows {rows_with_info_fill:?}"
    );
}

#[test]
fn rechecking_stage_uses_warning_color_on_gauge() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 4);
    page.stage = DownloadStage::Rechecking;
    page.total_maps = 10;
    page.download_target = 10;
    page.stats.skipped = 5;
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let buf = render_buffer(&app, 100, 24);
    let warning = Color::Rgb(249, 226, 175);

    let has_warning_fill = buf
        .content
        .iter()
        .any(|cell| cell.symbol() == "█" && cell.style().fg == Some(warning));
    assert!(
        has_warning_fill,
        "gauge fill must use warning color during rechecking"
    );
}

#[test]
fn active_progress_is_per_beatmapset() {
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 2);
    page.update_active_status(
        100,
        crate::download::BeatmapStage::Downloading,
        "Downloading #100 from mirror",
        false,
        None,
    );
    page.update_active_progress(100, 1_000_000, 4_000_000);
    page.update_active_status(
        101,
        crate::download::BeatmapStage::Downloading,
        "Fetching #101 from mirror",
        false,
        None,
    );

    let line_100 = page
        .active_lines()
        .find(|l| l.beatmapset_id == 100)
        .expect("100 still active");
    assert_eq!(line_100.downloaded, 1_000_000);

    let line_101 = page
        .active_lines()
        .find(|l| l.beatmapset_id == 101)
        .expect("101 inserted");
    assert!(line_101.displayed_message().contains("Fetching"));
    assert_eq!(line_101.progress_ratio(), None);
}

#[test]
fn precheck_pending_status_does_not_consume_active_slot() {
    use crate::download::BeatmapStage;

    let mut page = CollectionPage::new(1, "ranked".into(), 2);
    // a flood of precheck "file changed" notifications must not pile up in the active panel
    for id in 0u32..50 {
        page.update_active_status(
            id,
            BeatmapStage::Pending,
            "file changed during precheck; re-downloading",
            false,
            None,
        );
    }
    assert_eq!(
        page.active_lines().count(),
        0,
        "precheck Pending events must not allocate active slots"
    );

    // a real download then claims a slot
    page.update_active_status(
        7,
        BeatmapStage::Downloading,
        "Downloading #7 from mirror",
        false,
        None,
    );
    assert_eq!(page.active_lines().count(), 1);
}

#[test]
fn active_slot_count_is_capped_at_thread_count() {
    use crate::download::BeatmapStage;

    let mut page = CollectionPage::new(1, "ranked".into(), 2);
    for id in [10u32, 11, 12, 13] {
        page.update_active_status(
            id,
            BeatmapStage::Downloading,
            &format!("Downloading #{id}"),
            false,
            None,
        );
    }
    assert_eq!(
        page.active_lines().count(),
        2,
        "active slots must not exceed concurrent thread count"
    );

    // when one terminates the freed slot can be reused
    page.update_active_status(10, BeatmapStage::Success, "done", false, None);
    page.update_active_status(
        12,
        BeatmapStage::Downloading,
        "Downloading #12",
        false,
        None,
    );
    let ids: std::collections::BTreeSet<u32> =
        page.active_lines().map(|l| l.beatmapset_id).collect();
    assert_eq!(ids, [11, 12].into_iter().collect());
}

#[test]
fn freed_slot_position_is_reused_for_stability() {
    use crate::download::BeatmapStage;

    let mut page = CollectionPage::new(1, "ranked".into(), 3);
    for id in [20u32, 21, 22] {
        page.update_active_status(
            id,
            BeatmapStage::Downloading,
            &format!("Downloading #{id}"),
            false,
            None,
        );
    }
    let position_of = |page: &CollectionPage, target: u32| -> Option<usize> {
        page.active_downloads.iter().position(|slot| {
            slot.as_ref()
                .is_some_and(|line| line.beatmapset_id == target)
        })
    };
    let pos_22 = position_of(&page, 22).expect("22 placed");

    // the middle slot frees; a new download must take that exact slot so the bottom row
    // doesn't shift visually.
    page.update_active_status(21, BeatmapStage::Success, "done", false, None);
    page.update_active_status(
        99,
        BeatmapStage::Downloading,
        "Downloading #99",
        false,
        None,
    );

    assert_eq!(position_of(&page, 99), Some(1));
    assert_eq!(
        position_of(&page, 22),
        Some(pos_22),
        "untouched neighbours must keep their slot index"
    );
}

#[test]
fn progress_alone_must_not_allocate_an_empty_slot() {
    use crate::download::BeatmapStage;

    let mut page = CollectionPage::new(1, "ranked".into(), 2);
    // a progress event without a preceding status event must not create a blank-message
    // slot — the lib always emits Contacting/Downloading first, and creating a line with
    // empty `displayed_message` is exactly the flicker source we're avoiding
    page.update_active_progress(42, 1_024, 4_096);
    assert_eq!(page.active_lines().count(), 0);

    // once the status event lands the slot allocates with a real message; subsequent
    // progress updates land on the same slot.
    page.update_active_status(
        42,
        BeatmapStage::Downloading,
        "contacting nerinyan",
        false,
        None,
    );
    page.update_active_progress(42, 1_024, 4_096);
    let line = page
        .active_lines()
        .find(|l| l.beatmapset_id == 42)
        .expect("slot held");
    assert!(!line.displayed_message().is_empty());
    assert_eq!(line.downloaded, 1_024);
}

#[test]
fn bar_visible_during_downloading_before_bytes_flow() {
    use crate::download::BeatmapStage;

    use super::accent;

    let mut page = CollectionPage::new(1, "ranked".into(), 1);
    page.update_active_status(
        7,
        BeatmapStage::Downloading,
        "contacting nerinyan",
        false,
        None,
    );
    let line = page.active_lines().next().expect("slot allocated");
    assert_eq!(
        line.bar_color(),
        accent(),
        "active downloads without a total should show an indeterminate bar in accent color"
    );

    page.update_active_progress(7, 4_096, 8_192);
    let line = page.active_lines().next().expect("slot allocated");
    assert_eq!(
        line.bar_color(),
        accent(),
        "bar must remain in accent color once real progress data is available"
    );
}

#[test]
fn first_status_lands_immediately_then_text_is_debounced() {
    use crate::download::BeatmapStage;

    let mut page = CollectionPage::new(1, "x".into(), 1);
    page.update_active_status(
        200,
        BeatmapStage::Downloading,
        "Downloading #200 ...",
        false,
        None,
    );
    let initial = page.active_downloads[0]
        .as_ref()
        .expect("slot must be allocated")
        .displayed_message();
    assert!(
        initial.starts_with("Downloading"),
        "first write must land instantly so the slot isn't blank, got {initial:?}"
    );

    // second write within the debounce window stays queued. after the window expires,
    // displayed flips to the latest pending value on next read.
    page.update_active_status(
        200,
        BeatmapStage::Downloading,
        "Rate limited on X, ...",
        true,
        None,
    );
    std::thread::sleep(std::time::Duration::from_millis(75));
    let line = page.active_downloads[0]
        .as_ref()
        .expect("slot must be allocated");
    let visible = line.displayed_message();
    assert!(
        visible.starts_with("Rate limited"),
        "queued status must surface after the debounce window, got {visible:?}"
    );
    assert!(line.displayed_rate_limited());

    page.update_active_status(200, BeatmapStage::Downloading, "", false, None);
    std::thread::sleep(std::time::Duration::from_millis(75));
    let fallback = page.active_downloads[0]
        .as_ref()
        .expect("slot must be allocated")
        .displayed_message();
    assert_eq!(fallback, "downloading #200");
}

#[test]
fn rapid_status_transitions_coalesce_to_latest() {
    use crate::download::BeatmapStage;

    let mut page = CollectionPage::new(1, "x".into(), 1);
    page.update_active_status(
        400,
        BeatmapStage::Downloading,
        "downloading from nerinyan",
        false,
        None,
    );
    page.update_active_status(
        400,
        BeatmapStage::Downloading,
        "checking nerinyan",
        false,
        None,
    );
    page.update_active_status(
        400,
        BeatmapStage::Downloading,
        "rate limited on nerinyan, waiting 5s",
        true,
        None,
    );
    std::thread::sleep(std::time::Duration::from_millis(75));

    let line = page.active_downloads[0]
        .as_ref()
        .expect("slot must be allocated");
    let visible = line.displayed_message();
    assert!(
        visible.starts_with("rate limited"),
        "intermediate texts must coalesce; only the final state shows after the window, got {visible:?}"
    );
    assert!(line.displayed_rate_limited());
}

#[test]
fn footer_offers_defer_and_drop_when_a_row_is_parked() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked".into(), 1);
    page.stage = DownloadStage::Downloading;
    // First write lands instantly, so the row reads as parked without a debounce.
    page.update_active_status(
        42,
        crate::download::BeatmapStage::Downloading,
        "rate limited on mirror",
        true,
        Some(std::time::Instant::now() + std::time::Duration::from_secs(30)),
    );
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 120, 24);
    assert!(
        output.contains("s defer"),
        "footer must offer defer when a row is parked inline: {output}"
    );
    assert!(
        output.contains("S drop"),
        "footer must offer drop when a row is parked inline: {output}"
    );
}

#[test]
fn footer_offers_drop_only_when_only_deferred() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked".into(), 1);
    page.stage = DownloadStage::Downloading;
    // A queue-deferred map with nothing parked inline: only `S drop` can act.
    page.mark_deferred(42);
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 120, 24);
    assert!(
        output.contains("S drop"),
        "footer must offer drop for a queue-deferred map: {output}"
    );
    assert!(
        !output.contains("s defer"),
        "footer must not offer defer when nothing is parked inline: {output}"
    );
}

#[test]
fn terminal_stage_clears_active_downloads() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 1);
    page.update_active_status(
        100,
        crate::download::BeatmapStage::Downloading,
        "Downloading #100",
        false,
        None,
    );
    app.downloads.push(page);

    app.handle_download_event(DownloadEvent::Failed {
        id: 1,
        message: "boom".into(),
    });
    assert_eq!(
        app.downloads[0].active_lines().count(),
        0,
        "active_downloads must be cleared on Failed"
    );

    app.downloads[0].update_active_status(
        101,
        crate::download::BeatmapStage::Downloading,
        "Downloading #101",
        false,
        None,
    );
    app.handle_download_event(DownloadEvent::StageChanged {
        id: 1,
        stage: DownloadStage::Completed,
    });
    assert_eq!(
        app.downloads[0].active_lines().count(),
        0,
        "active_downloads must be cleared on StageChanged Completed"
    );
}

#[test]
fn info_toast_renders_with_info_colored_bar() {
    let mut app = App::new(Config::default());
    app.toast_info("ready");
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");
    terminal
        .draw(|frame| {
            super::draw(frame, &app);
        })
        .expect("app should render");

    let has_info_bar = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .any(|cell| cell.symbol() == "┃" && cell.style().fg == Some(Color::Rgb(116, 199, 236)));

    assert!(has_info_bar, "info toast must render a ┃ bar in INFO color");
}

#[test]
fn threads_stepper_renders_recommended_chip_when_value_differs() {
    use crate::app::HomeField;

    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Threads;
    // Set to something other than the recommended (default_threads).
    let recommended = app.home.default_threads;
    let non_default = if recommended > 1 { recommended - 1 } else { 2 };
    app.home.threads.value = non_default.to_string();

    let output = render_app(&app, 80, 30);

    assert!(
        output.contains("recommended"),
        "recommended chip must appear when thread count differs from CPU count: {output}"
    );
}

#[test]
fn threads_stepper_omits_recommended_chip_when_at_default() {
    use crate::app::HomeField;

    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Threads;
    // Set value to exactly the recommended amount.
    let recommended = app.home.default_threads;
    app.home.threads.value = recommended.to_string();

    let output = render_app(&app, 80, 30);

    assert!(
        !output.contains("recommended"),
        "recommended chip must be omitted when value equals CPU count"
    );
}

#[test]
fn home_hint_shows_plus_minus_when_threads_focused() {
    use crate::app::HomeField;

    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Threads;

    let output = render_app(&app, 80, 24);

    assert!(
        output.contains("+/-") || output.contains("+"),
        "footer must show +/- hint when threads stepper is focused: {output}"
    );
}

#[test]
fn config_archive_validation_help_shows_when_focused() {
    use crate::download::ArchiveValidation;

    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::DownloadArchiveValidation;
    app.config.archive_validation = ArchiveValidation::Magic;

    let output = render_app(&app, 100, 30);

    assert!(
        output.contains("verifies archive headers"),
        "the current-state help must appear when the field is focused: {output}"
    );
}

#[test]
fn config_archive_validation_help_hidden_when_not_focused() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::DownloadThreads;

    let output = render_app(&app, 100, 30);

    assert!(
        !output.contains("verifies archive headers"),
        "archive validation help must not appear when field is not focused: {output}"
    );
    assert!(
        !output.contains("eocd"),
        "strict help must not appear when field is not focused: {output}"
    );
}

#[test]
fn config_archive_validation_strict_help_when_strict_selected_and_focused() {
    use crate::download::ArchiveValidation;

    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::DownloadArchiveValidation;
    app.config.archive_validation = ArchiveValidation::Eocd;

    let output = render_app(&app, 100, 30);

    assert!(
        output.contains("eocd"),
        "the strict-state help must appear when strict is selected and focused: {output}"
    );
    assert!(
        !output.contains("verifies archive headers"),
        "only the current-state help must render, not the basic one: {output}"
    );
}

#[test]
fn config_custom_mirror_help_shows_when_focused() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::MirrorCustomUrl(0);

    let output = render_app(&app, 100, 30);

    assert!(
        output.contains("must contain {id}"),
        "custom mirror help must appear when field is focused: {output}"
    );
}

#[test]
fn config_custom_mirror_help_hidden_when_not_focused() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    app.config.focus = ConfigField::DownloadThreads;

    let output = render_app(&app, 100, 30);

    assert!(
        !output.contains("must contain {id}"),
        "custom mirror help must not appear when field is not focused: {output}"
    );
}

#[test]
fn updates_osu_path_help_shows_when_focused() {
    use crate::app::{GetMapsSource, HomeField};
    use crate::osu_db::OsuClient;

    let mut config = Config::default();
    config.recent.osu_client = Some(OsuClient::Stable);
    let mut app = App::new(config);
    app.home.source = GetMapsSource::Update;
    app.home.focus = HomeField::UpdateOsuPath;

    let output = render_app(&app, 100, 30);

    assert!(
        output.contains("must contain osu!.db"),
        "osu! path help must appear when field is focused: {output}"
    );
}

#[test]
fn updates_osu_path_help_names_active_client_db() {
    use crate::app::{GetMapsSource, HomeField};
    use crate::osu_db::OsuClient;

    let mut config = Config::default();
    config.recent.osu_client = Some(OsuClient::Lazer);
    let mut app = App::new(config);
    app.home.source = GetMapsSource::Update;
    app.home.focus = HomeField::UpdateOsuPath;

    let output = render_app(&app, 100, 30);

    assert!(
        output.contains("must contain client.realm"),
        "lazer must name client.realm in the path help: {output}"
    );
    assert!(
        !output.contains("osu!.db"),
        "lazer must not mention osu!.db: {output}"
    );
}

#[test]
fn updates_osu_path_help_hidden_when_not_focused() {
    use crate::app::{GetMapsSource, HomeField};

    let mut app = App::new(Config::default());
    app.home.source = GetMapsSource::Update;
    app.home.focus = HomeField::UpdateScan;

    let output = render_app(&app, 100, 30);

    assert!(
        !output.contains("must contain osu!.db"),
        "osu! path help must not appear when field is not focused: {output}"
    );
}

// --- compact mode smoke tests ---
// These verify that rendering at terminal height < 14 does not panic and
// that the most-critical information is still visible.

#[test]
fn compact_home_renders_without_panic() {
    let app = App::new(Config::default());
    // total height 10 → content area 8 → compact path
    let output = render_app(&app, 60, 10);
    assert!(!output.is_empty(), "compact home must produce output");
    // section header dividers must be absent (decorative chrome stripped)
    assert!(
        !output.contains("── collection") && !output.contains("── mirrors"),
        "section header dividers must be hidden in compact home"
    );
}

#[test]
fn compact_home_shows_url_field() {
    use crate::app::HomeField;

    let mut app = App::new(Config::default());
    app.home.focus = HomeField::Collection;
    let output = render_app(&app, 60, 10);
    // The collection input label is always rendered
    assert!(
        output.contains("collection"),
        "collection field must appear in compact home: {output}"
    );
}

#[test]
fn compact_download_renders_without_panic() {
    let mut app = App::new(Config::default());
    let mut page = CollectionPage::new(1, "ranked maps".to_string(), 2);
    page.stage = DownloadStage::Downloading;
    page.total_maps = 10;
    page.download_target = 10;
    app.downloads.push(page);
    app.active_tab = Tab::Download(0);

    let output = render_app(&app, 60, 10);
    assert!(!output.is_empty(), "compact download must produce output");
    // per-row breakdown is hidden, but active/failed counts must appear
    assert!(
        output.contains("active:"),
        "compact download must show active count: {output}"
    );
    assert!(
        output.contains("failed:"),
        "compact download must show failed count: {output}"
    );
}

#[test]
fn compact_config_renders_without_panic() {
    let mut app = App::new(Config::default());
    app.active_tab = Tab::Config;
    let output = render_app(&app, 60, 10);
    assert!(!output.is_empty(), "compact config must produce output");
    // section headers hidden
    assert!(
        !output.contains("DISPLAY"),
        "section headers must be hidden in compact config"
    );
    // auth chip still visible
    assert!(
        output.contains("signed out")
            || output.contains("signed in")
            || output.contains("login unavailable"),
        "auth chip must still appear in compact config: {output}"
    );
}

#[test]
fn compact_updates_renders_without_panic() {
    use crate::app::GetMapsSource;
    let mut app = App::new(Config::default());
    app.home.source = GetMapsSource::Update;
    let output = render_app(&app, 60, 10);
    assert!(
        !output.is_empty(),
        "compact update source must produce output"
    );
    // The scan CTA is the anchor of the compact update form.
    assert!(
        output.contains("scan"),
        "compact update source must show the scan CTA: {output}"
    );
}

#[test]
fn header_renders_brand_tabs_and_version_regions() {
    let app = App::new(Config::default());
    let output = render_app(&app, 120, 24);

    assert!(
        output.contains("osu!collect"),
        "brand must render in header"
    );
    assert!(output.contains("home"), "tabs must render in header");
    let version = concat!("v", env!("CARGO_PKG_VERSION"));
    assert!(
        output.contains(version),
        "version must render in header: expected {version:?} in output"
    );
}
