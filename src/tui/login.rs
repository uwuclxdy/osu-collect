use crate::app::{AuthLoginState, LoginField, LoginPhase, LoginTab};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets;
use super::{success, text, text_dim, warning};

const PANEL_TITLE: &str = " LOGIN ";

const SECTION_CREDENTIALS: &str = "credentials";
const SECTION_VERIFICATION: &str = "verification";
const SECTION_ACCOUNT: &str = "account";

const NOTE_PASSWORD: &str = "logs in to osu.ppy.sh over https, access token is stored locally";
const NOTE_VERIFICATION: &str = "osu! emailed a code to verify this device.";

const CAUTION: &[&str] = &[
    "signs in with osu!lazer's first-party client (unofficial).",
    "requests to osu! are throttled automatically to stay within its limits.",
    "grey area: still best as a last-resort mirror, used sparingly.",
];

pub fn render(
    frame: &mut Frame,
    area: Rect,
    login: &LoginTab,
    login_state: &AuthLoginState,
    editing: bool,
) {
    let items = build_login_items(login, login_state, editing);

    let cursor_col = editing
        .then(|| {
            login
                .focused_input()
                .map(|field| widgets::input_cursor_col(field, 0))
        })
        .flatten();

    let (items, focused_index) = items.into_parts();
    widgets::render_scrollable_panel(
        frame,
        area,
        PANEL_TITLE,
        items,
        focused_index,
        // Text rows tint on focus; the action chips style themselves.
        login.focus.is_text_input(),
        cursor_col,
        true,
        true,
        &login.list_offset,
    );
}

fn build_login_items(
    login: &LoginTab,
    login_state: &AuthLoginState,
    editing: bool,
) -> widgets::FormItems<LoginField> {
    let focus = login.focus;
    let in_flight = matches!(login_state, AuthLoginState::InProgress(_));
    let mut items = widgets::FormItems::new(focus);

    match login.phase {
        LoginPhase::Credentials => {
            items.push(widgets::section_header(
                SECTION_CREDENTIALS,
                focus.is_text_input(),
            ));
            items.push_focusable(
                LoginField::Username,
                widgets::input_item(&login.username, focus == LoginField::Username, editing, 0),
            );
            items.push_focusable(
                LoginField::Password,
                widgets::password_input_item(
                    &login.password,
                    focus == LoginField::Password,
                    editing,
                    0,
                ),
            );
            items.push(widgets::spacer());
            items.push_focusable(
                LoginField::Submit,
                widgets::button_item(
                    submit_label(login.phase, in_flight),
                    focus == LoginField::Submit,
                    true,
                ),
            );
            items.push(widgets::spacer());
            items.push(note_line(NOTE_PASSWORD));
        }
        LoginPhase::NeedsVerification => {
            items.push(widgets::section_header(
                SECTION_VERIFICATION,
                focus == LoginField::Code,
            ));
            items.push(note_line(NOTE_VERIFICATION));
            items.push_focusable(
                LoginField::Code,
                widgets::input_item(&login.code, focus == LoginField::Code, editing, 0),
            );
            items.push(widgets::spacer());
            items.push_focusable(
                LoginField::Submit,
                widgets::button_item(
                    submit_label(login.phase, in_flight),
                    focus == LoginField::Submit,
                    true,
                ),
            );
            items.push_focusable(
                LoginField::Resend,
                widgets::button_item("resend code", focus == LoginField::Resend, !in_flight),
            );
        }
        LoginPhase::LoggedIn => {
            items.push(status_line());
            items.push(note_line("the osu! official mirror is now available."));
            // Only after a fresh sign-in this session — not when the tab was
            // opened already-logged-in via the config "manage" chip.
            if login.just_logged_in {
                items.push(note_line("you can close this tab now (esc or q)."));
            }
            items.push(widgets::spacer());
            items.push(widgets::section_header(SECTION_ACCOUNT, false));
            for line in CAUTION {
                items.push(caution_line(line));
            }
            items.push(widgets::spacer());
            items.push_focusable(
                LoginField::Submit,
                widgets::button_item("log out", focus == LoginField::Submit, true),
            );
        }
    }

    items
}

/// Primary action-chip label for the current phase and request state.
fn submit_label(phase: LoginPhase, in_flight: bool) -> &'static str {
    if in_flight {
        return "cancel";
    }
    match phase {
        LoginPhase::Credentials => "log in",
        LoginPhase::NeedsVerification => "verify",
        LoginPhase::LoggedIn => "log out",
    }
}

/// `● logged in` status line (SUCCESS dot).
fn status_line() -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled("● ", Style::default().fg(success())),
        Span::styled("logged in", Style::default().fg(text())),
    ]))
}

/// A dim informational line, indented to the panel content gutter.
fn note_line(text: &'static str) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(text, Style::default().fg(text_dim())),
    ]))
}

/// A caution line in WARNING color.
fn caution_line(text: &'static str) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(text, Style::default().fg(warning())),
    ]))
}
