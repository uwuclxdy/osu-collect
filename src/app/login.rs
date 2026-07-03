use super::home::InputField;
use super::{first_field, last_field, next_field, prev_field};

/// Which step of the login flow the tab is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginPhase {
    /// Username + password entry.
    Credentials,
    /// osu! demanded device verification; collecting the emailed / TOTP code.
    NeedsVerification,
    /// A valid token is stored — show status + the logout action.
    LoggedIn,
}

/// Focusable rows on the login tab. The visible set depends on [`LoginPhase`]
/// (see [`LoginTab::fields`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginField {
    Username,
    Password,
    Code,
    /// Primary action chip: `log in` / `verify` / `log out` (or `cancel` while a
    /// request is in flight).
    Submit,
    /// Secondary action: re-send the verification code.
    Resend,
}

impl LoginField {
    pub fn is_text_input(self) -> bool {
        matches!(
            self,
            LoginField::Username | LoginField::Password | LoginField::Code
        )
    }

    /// Whether this field's value should render masked (`•`).
    pub fn is_secret(self) -> bool {
        self == LoginField::Password
    }
}

const CREDENTIALS_FIELDS: &[LoginField] = &[
    LoginField::Username,
    LoginField::Password,
    LoginField::Submit,
];
const VERIFICATION_FIELDS: &[LoginField] =
    &[LoginField::Code, LoginField::Submit, LoginField::Resend];
const LOGGED_IN_FIELDS: &[LoginField] = &[LoginField::Submit];

/// State for the dynamic, closeable login tab. Drives the osu!lazer password
/// (ROPC) login that authorizes the osu! official download mirror.
pub struct LoginTab {
    pub username: InputField,
    pub password: InputField,
    pub code: InputField,
    pub focus: LoginField,
    pub phase: LoginPhase,
    /// `true` only when the `LoggedIn` phase was reached via a fresh login this
    /// session ([`LoginTab::enter_logged_in`]). Stays `false` when the tab opens
    /// already-logged-in via the config "manage" chip ([`LoginTab::new`] with
    /// `logged_in`), so the "you can close this tab now" hint shows only after an
    /// actual sign-in.
    pub just_logged_in: bool,
    /// Persisted list scroll offset so the focused row isn't re-pinned to the
    /// panel's bottom edge every frame (see [`widgets::render_list`]).
    pub list_offset: std::cell::Cell<usize>,
}

impl LoginTab {
    /// Build a fresh login tab. When `logged_in` (a token is already stored) the
    /// tab opens on the logged-in view; otherwise on the credentials form.
    pub fn new(logged_in: bool) -> Self {
        let phase = if logged_in {
            LoginPhase::LoggedIn
        } else {
            LoginPhase::Credentials
        };
        let focus = if logged_in {
            LoginField::Submit
        } else {
            LoginField::Username
        };
        Self {
            username: InputField::new("username", "", "osu! username or email"),
            password: InputField::new("password", "", "osu! password"),
            code: InputField::new("code", "", "verification code"),
            focus,
            phase,
            just_logged_in: false,
            list_offset: std::cell::Cell::new(0),
        }
    }

    /// The focusable rows for the current phase, in navigation order.
    pub fn fields(&self) -> &'static [LoginField] {
        match self.phase {
            LoginPhase::Credentials => CREDENTIALS_FIELDS,
            LoginPhase::NeedsVerification => VERIFICATION_FIELDS,
            LoginPhase::LoggedIn => LOGGED_IN_FIELDS,
        }
    }

    pub fn next_field(&mut self) {
        self.focus = next_field(self.fields(), self.focus);
    }

    pub fn prev_field(&mut self) {
        self.focus = prev_field(self.fields(), self.focus);
    }

    pub fn first_field(&mut self) {
        self.focus = first_field(self.fields(), self.focus);
    }

    pub fn last_field(&mut self) {
        self.focus = last_field(self.fields(), self.focus);
    }

    /// Reveal the verification step and focus the code field. Called when osu!
    /// signals a pending device verification after the password grant.
    pub fn enter_verification(&mut self) {
        self.phase = LoginPhase::NeedsVerification;
        self.code.set_value(String::new());
        self.focus = LoginField::Code;
    }

    /// Move to the logged-in view and clear every entered secret from the UI.
    /// Marks `just_logged_in` so the view shows the "you can close this tab now"
    /// hint — the config "manage" path keeps it `false` via [`LoginTab::new`].
    pub fn enter_logged_in(&mut self) {
        self.phase = LoginPhase::LoggedIn;
        self.focus = LoginField::Submit;
        self.just_logged_in = true;
        self.username.set_value(String::new());
        self.password.set_value(String::new());
        self.code.set_value(String::new());
    }

    /// Return to the credentials form (login failed, cancelled, or logged out).
    /// The username is kept so a retry is one keypress; the password is wiped.
    pub fn reset_credentials(&mut self) {
        self.phase = LoginPhase::Credentials;
        self.just_logged_in = false;
        self.password.set_value(String::new());
        self.code.set_value(String::new());
        self.focus = LoginField::Username;
    }

    /// Clear only the password (called the moment it is handed to the login
    /// command, so the secret never lingers in the field).
    pub fn clear_password(&mut self) {
        self.password.set_value(String::new());
    }

    pub fn handle_char(&mut self, ch: char) {
        if let Some(field) = self.focused_input_mut() {
            field.insert_char(ch);
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        if let Some(field) = self.focused_input_mut() {
            field.insert_str(text);
        }
    }

    pub fn backspace(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.delete_before_caret();
        }
    }

    pub fn delete_forward(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.delete_at_caret();
        }
    }

    pub fn backspace_word(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.delete_word_before_caret();
        }
    }

    pub fn caret_left(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.caret_left();
        }
    }

    pub fn caret_right(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.caret_right();
        }
    }

    pub fn caret_home(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.caret_home();
        }
    }

    pub fn caret_end(&mut self) {
        if let Some(field) = self.focused_input_mut() {
            field.caret_end();
        }
    }

    /// The focused text input, or `None` on the action chips. Used by the
    /// renderer to place the caret.
    pub fn focused_input(&self) -> Option<&InputField> {
        match self.focus {
            LoginField::Username => Some(&self.username),
            LoginField::Password => Some(&self.password),
            LoginField::Code => Some(&self.code),
            _ => None,
        }
    }

    fn focused_input_mut(&mut self) -> Option<&mut InputField> {
        match self.focus {
            LoginField::Username => Some(&mut self.username),
            LoginField::Password => Some(&mut self.password),
            LoginField::Code => Some(&mut self.code),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/login.rs"]
mod tests;
