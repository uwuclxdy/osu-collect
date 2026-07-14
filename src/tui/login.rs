use crate::app::{AuthLoginState, LoginField, LoginPhase, LoginTab};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::ListItem,
};

use super::widgets;
use super::{success, text, text_dim, warning};

const PANEL_TITLE: &str = " LOGIN ";

const SECTION_CREDENTIALS: &str = "credentials";
const SECTION_VERIFICATION: &str = "verification";
const SECTION_ACCOUNT: &str = "account";

const NOTE_PASSWORD: &[&str] = &[
    "logs in to osu.ppy.sh over https",
    "access token is stored locally",
];
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
    // Informational lines word-wrap to the panel's inner text width so the
    // narrow config-split panel never hard-clips them: inner is width - 2
    // borders - 2 padding, and note lines indent 2 more.
    let text_width = area.width.saturating_sub(6) as usize;
    let items = build_login_items(login, login_state, editing, text_width);

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
        None,
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
    text_width: usize,
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
                    widgets::ButtonProminence::Primary,
                ),
            );
            items.push(widgets::spacer());
            for line in NOTE_PASSWORD {
                push_wrapped(&mut items, line, text_dim(), text_width);
            }
        }
        LoginPhase::NeedsVerification => {
            items.push(widgets::section_header(
                SECTION_VERIFICATION,
                focus == LoginField::Code,
            ));
            push_wrapped(&mut items, NOTE_VERIFICATION, text_dim(), text_width);
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
                    widgets::ButtonProminence::Primary,
                ),
            );
            items.push_focusable(
                LoginField::Resend,
                widgets::button_item(
                    "resend code",
                    focus == LoginField::Resend,
                    !in_flight,
                    widgets::ButtonProminence::Primary,
                ),
            );
        }
        LoginPhase::LoggedIn => {
            items.push(status_line());
            push_wrapped(
                &mut items,
                "the osu! official mirror is now available.",
                text_dim(),
                text_width,
            );
            // Only after a fresh sign-in this session — not when the panel was
            // opened already-logged-in via the config "manage" chip.
            if login.just_logged_in {
                let mut spans = vec![Span::raw("  ")];
                spans.extend(widgets::keyed_spans(
                    "[esc] or [q] to close",
                    Style::default().fg(super::accent()).bold(),
                    Style::default().fg(text_dim()),
                ));
                items.push(ListItem::new(Line::from(spans)));
            }
            items.push(widgets::spacer());
            items.push(widgets::section_header(SECTION_ACCOUNT, false));
            for line in CAUTION {
                push_wrapped(&mut items, line, warning(), text_width);
            }
            items.push(widgets::spacer());
            items.push_focusable(
                LoginField::Submit,
                widgets::button_item(
                    "log out",
                    focus == LoginField::Submit,
                    true,
                    widgets::ButtonProminence::Primary,
                ),
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
        "● ".fg(success()),
        "logged in".fg(text()),
    ]))
}

/// Pushes an informational line word-wrapped to `width`, one indented `ListItem`
/// per wrapped line, in `color`. Keeps the narrow config-split panel from
/// hard-clipping the login copy at its right border.
fn push_wrapped(
    items: &mut widgets::FormItems<LoginField>,
    text: &str,
    color: Color,
    width: usize,
) {
    for line in wrap(text, width) {
        items.push(ListItem::new(Line::from(vec![
            Span::raw("  "),
            line.fg(color),
        ])));
    }
}

/// Greedy word-wrap to `width` columns. The login copy is ASCII, so char count
/// equals display width. Always returns at least one line; a word longer than
/// `width` is left to overflow its own line rather than split mid-word.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
