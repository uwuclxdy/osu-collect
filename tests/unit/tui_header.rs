use ratatui::{Terminal, backend::TestBackend, layout::Rect, style::Modifier};

use super::{RenderParams, render};

fn header_buffer_with_active(active: usize) -> ratatui::buffer::Buffer {
    header_buffer(active, false)
}

fn header_buffer(active: usize, downloading: bool) -> ratatui::buffer::Buffer {
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
                    update_version: None,
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
fn brand_text_is_identical_idle_and_downloading() {
    // The animation only recolors the wordmark; the glyphs never change.
    let idle = buffer_text(&header_buffer(0, false));
    let busy = buffer_text(&header_buffer(0, true));
    assert!(idle.contains("osu!collect"));
    assert!(busy.contains("osu!collect"));
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
