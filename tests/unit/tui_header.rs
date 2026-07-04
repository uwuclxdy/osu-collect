use ratatui::{Terminal, backend::TestBackend, layout::Rect, style::Modifier};

use crate::app::UpdateIndicator;

use super::{RenderParams, render};

fn header_buffer_with_active(active: usize) -> ratatui::buffer::Buffer {
    header_buffer(active, false)
}

fn header_buffer(active: usize, downloading: bool) -> ratatui::buffer::Buffer {
    header_buffer_with_update(active, downloading, None)
}

fn header_buffer_with_update(
    active: usize,
    downloading: bool,
    update_phase: Option<UpdateIndicator>,
) -> ratatui::buffer::Buffer {
    let tabs: Vec<std::borrow::Cow<'static, str>> = ["home", "updates", "config"]
        .map(std::borrow::Cow::Borrowed)
        .into();
    let backend = TestBackend::new(80, 1);
    let mut terminal = Terminal::new(backend).expect("test backend should initialize");
    terminal
        .draw(|frame| {
            render(
                frame,
                RenderParams {
                    area: Rect::new(0, 0, 80, 1),
                    tabs: &tabs,
                    active,
                    tick: 0,
                    downloading,
                    brand_ramp: if downloading { 1.0 } else { 0.0 },
                    update_phase,
                    client: crate::osu_db::OsuClient::Stable,
                },
            );
        })
        .expect("header should render");
    terminal.backend().buffer().clone()
}

/// Concatenate the rendered cells into a single string for substring checks.
fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
    buf.content.iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn brand_renders_osu_bang_collect() {
    let buf = header_buffer_with_active(0);
    assert!(
        buffer_text(&buf).contains("osu!collect"),
        "header must render the osu!collect wordmark"
    );
}

#[test]
fn client_chip_renders_after_version() {
    let buf = header_buffer_with_active(0);
    let text = buffer_text(&buf);
    let chip = text
        .find("[ stable ]")
        .expect("header must render the active client chip");
    let version = text.find(" v").expect("header must render the version");
    assert!(chip > version, "the client chip sits right of the version");
}

#[test]
fn client_chip_label_breathes_with_tick() {
    // The label color must respond to the tick (the breathing glow). The `b` of
    // "stable" is unique in the header, so it isolates the label from the brand.
    let label_fg = |tick: u64| {
        let tabs: Vec<std::borrow::Cow<'static, str>> =
            ["home"].map(std::borrow::Cow::Borrowed).into();
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                render(
                    frame,
                    RenderParams {
                        area: Rect::new(0, 0, 40, 1),
                        tabs: &tabs,
                        active: 0,
                        tick,
                        downloading: false,
                        brand_ramp: 0.0,
                        update_phase: None,
                        client: crate::osu_db::OsuClient::Stable,
                    },
                );
            })
            .expect("render");
        let buf = terminal.backend().buffer().clone();
        buf.content
            .iter()
            .find(|c| c.symbol() == "b")
            .and_then(|c| c.style().fg)
    };
    // A quarter-period apart (period is 80 ticks) → different breath depth.
    let a = label_fg(0);
    let b = label_fg(20);
    assert!(a.is_some() && b.is_some(), "the label glyph must render");
    assert_ne!(a, b, "client label color must animate across ticks");
}

#[test]
fn brand_text_is_identical_idle_and_downloading() {
    // The animation only recolors the wordmark; the glyphs never change.
    let idle = buffer_text(&header_buffer(0, false));
    let busy = buffer_text(&header_buffer(0, true));
    assert!(idle.contains("osu!collect"));
    assert!(busy.contains("osu!collect"));
}

#[test]
fn update_available_shows_current_version_and_up_arrow() {
    let buf = header_buffer_with_update(0, false, Some(UpdateIndicator::Available));
    let text = buffer_text(&buf);
    assert!(
        text.contains(concat!("v", env!("CARGO_PKG_VERSION"))),
        "available cue keeps the current version"
    );
    assert!(text.contains('↑'), "available cue keeps the ↑ arrow");
}

#[test]
fn update_downloading_swaps_arrow_for_spinner() {
    let buf = header_buffer_with_update(0, false, Some(UpdateIndicator::Downloading));
    let text = buffer_text(&buf);
    // tick 0 → first braille spinner frame; the arrow is gone.
    assert!(
        text.contains('⠋'),
        "downloading swaps the arrow for a spinner"
    );
    assert!(!text.contains('↑'), "downloading drops the ↑ arrow");
}

#[test]
fn update_restart_pending_shows_reload_glyph() {
    let buf = header_buffer_with_update(0, false, Some(UpdateIndicator::RestartPending));
    let text = buffer_text(&buf);
    assert!(
        text.contains('↻'),
        "restart-pending shows the ↻ reload glyph"
    );
    assert!(!text.contains('↑'), "restart-pending drops the ↑ arrow");
}

#[test]
fn active_tab_has_underlined_modifier() {
    // active=0 → "home"; check that at least one cell of "home" carries UNDERLINED
    let buf = header_buffer_with_active(0);
    let has_underlined = buf.content.iter().any(|cell| {
        cell.symbol() == "h" && cell.style().add_modifier.contains(Modifier::UNDERLINED)
    });
    assert!(
        has_underlined,
        "active tab 'home' must carry UNDERLINED modifier on at least one cell"
    );
}

#[test]
fn inactive_tabs_do_not_have_underlined_modifier() {
    // active=0 → "home"; "updates" and "config" are inactive
    let buf = header_buffer_with_active(0);

    // Sample the first letter of each inactive tab title.
    // "updates" starts with 'u', "config" starts with 'c'.
    // Neither of these letters appears in "home", the brand, or the version on a 80-col render,
    // so checking the modifier on 'u' and 'c' cells is sufficient.
    let inactive_letters = ['u', 'c'];
    for letter in inactive_letters {
        let underlined_inactive = buf.content.iter().any(|cell| {
            cell.symbol() == letter.encode_utf8(&mut [0u8; 4])
                && cell.style().add_modifier.contains(Modifier::UNDERLINED)
        });
        assert!(
            !underlined_inactive,
            "inactive tab with first letter '{letter}' must not carry UNDERLINED modifier"
        );
    }
}
