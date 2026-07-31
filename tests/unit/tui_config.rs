use super::auth_chip_item;
use super::widgets::render_list;
use crate::app::{AuthLoginState, ConfigTab};
use crate::config::Config;
use ratatui::{Terminal, backend::TestBackend, layout::Rect, widgets::ListItem};

fn tab_with(login_state: AuthLoginState, supporter: bool) -> ConfigTab {
    let mut tab = ConfigTab::new(&Config::default());
    tab.login_state = login_state;
    tab.supporter = supporter;
    tab
}

/// Renders a single chip `ListItem` into a wide one-row buffer and reads back
/// its plain text, so the assertion pins what actually reaches the screen
/// rather than the `ConfigTab` model alone.
fn render_row(item: ListItem<'static>) -> String {
    let width = 40;
    let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("test backend");
    terminal
        .draw(|frame| {
            let _ = render_list(
                frame,
                Rect::new(0, 0, width, 1),
                vec![item],
                None,
                false,
                &std::cell::Cell::new(0),
            );
        })
        .expect("frame renders");
    let buf = terminal.backend().buffer().clone();
    (0..width).map(|x| buf[(x, 0)].symbol()).collect()
}

#[test]
fn auth_chip_shows_supporter_badge_only_when_logged_in_and_confirmed() {
    let logged_out = render_row(auth_chip_item(&tab_with(AuthLoginState::LoggedOut, true)));
    assert!(
        !logged_out.contains("supporter"),
        "a logged-out account must never claim supporter, got: {logged_out:?}"
    );

    let logged_in_unconfirmed =
        render_row(auth_chip_item(&tab_with(AuthLoginState::LoggedIn, false)));
    assert!(
        !logged_in_unconfirmed.contains("supporter"),
        "an unconfirmed account must show no badge, got: {logged_in_unconfirmed:?}"
    );
    assert!(
        logged_in_unconfirmed.contains("logged in"),
        "the base logged-in state must still render, got: {logged_in_unconfirmed:?}"
    );

    let logged_in_supporter = render_row(auth_chip_item(&tab_with(AuthLoginState::LoggedIn, true)));
    assert!(
        logged_in_supporter.contains("supporter"),
        "a confirmed supporter must show the badge, got: {logged_in_supporter:?}"
    );
}
