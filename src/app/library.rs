use super::home::InputField;
use crate::config::Config;
use crate::osu_db::OsuClient;
use crate::utils::expand_tilde;

/// App-global "which osu! library are we working against": the selected client
/// (stable ↔ lazer) and its install-path input. Hoisted off per-tab state so
/// the header chip, the download pipeline, and the library scan share one
/// backing value; the Get Maps update source (the former Updates tab) reads it too.
///
/// Owns the osu!-path text field and its editing, so the field survives when
/// the panel that renders it moves. Scan/selection reset on a client switch
/// stays on the update source (see [`UpdateSource::reset_for_client_switch`]).
///
/// [`UpdateSource::reset_for_client_switch`]: super::UpdateSource::reset_for_client_switch
#[derive(Debug, Clone)]
pub struct LibraryState {
    pub client_type: OsuClient,
    pub osu_path: InputField,
}

impl LibraryState {
    pub fn new(client_type: OsuClient) -> Self {
        let default_path = Self::detect_default_path(client_type);
        Self::build(client_type, default_path.clone(), default_path)
    }

    /// Seed the client kind + path from the persisted `[recent]` config, falling
    /// back to auto-detection when either value is absent or blank. A
    /// saved-but-missing path is kept verbatim so the scan reports "no db"
    /// instead of silently reverting to the default location.
    pub fn from_config(config: &Config) -> Self {
        let client_type = config.recent.osu_client.unwrap_or_default();
        match config.recent.osu_path.as_deref() {
            Some(saved) if !saved.trim().is_empty() => Self::from_saved(client_type, saved),
            _ => Self::new(client_type),
        }
    }

    /// Seed from a persisted path, keeping the saved value verbatim (even if it
    /// no longer exists on disk) while still using the auto-detected default as
    /// the placeholder hint.
    fn from_saved(client_type: OsuClient, saved: &str) -> Self {
        let placeholder = Self::detect_default_path(client_type);
        Self::build(client_type, saved.to_string(), placeholder)
    }

    fn build(client_type: OsuClient, value: String, placeholder: String) -> Self {
        Self {
            client_type,
            osu_path: InputField::new("osu! path", value, placeholder),
        }
    }

    fn detect_default_path(client: OsuClient) -> String {
        use crate::osu_db::{BeatmapReader, LazerReader, StableReader};

        match client {
            OsuClient::Stable => StableReader::default_path(),
            OsuClient::Lazer => LazerReader::default_path(),
        }
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
    }

    /// Returns the osu! path with any leading `~` expanded to the home
    /// directory. Call this only when passing the path to the filesystem layer,
    /// never for rendering (the raw typed value is shown to the user).
    pub fn osu_path(&self) -> String {
        expand_tilde(&self.osu_path.value)
    }

    /// Whether the osu! path field still holds its auto-detected default (so the
    /// render can flag it as detected rather than hand-typed).
    pub fn is_path_auto_detected(&self) -> bool {
        self.osu_path.value == self.osu_path.placeholder
    }

    /// Switch the active osu! client (stable ↔ lazer). Re-detects the default
    /// install path while the field still holds a placeholder, then updates the
    /// placeholder hint. The prior client's scan data is cleared by the caller
    /// via [`UpdateSource::reset_for_client_switch`].
    ///
    /// [`UpdateSource::reset_for_client_switch`]: super::UpdateSource::reset_for_client_switch
    pub fn switch_client(&mut self) {
        self.client_type.toggle();
        let new_path = Self::detect_default_path(self.client_type);
        if self.osu_path.value.is_empty() || self.osu_path.value == self.osu_path.placeholder {
            self.osu_path.set_value(new_path.clone());
        }
        self.osu_path.placeholder = new_path;
    }

    // osu!-path text editing. The app gates these on the update source's path
    // field being the focused, editable input (`App::home_update_path_editing`).
    pub fn insert_char(&mut self, ch: char) {
        self.osu_path.insert_char(ch);
    }

    pub fn insert_str(&mut self, text: &str) {
        self.osu_path.insert_str(text);
    }

    pub fn backspace(&mut self) {
        self.osu_path.delete_before_caret();
    }

    pub fn delete_forward(&mut self) {
        self.osu_path.delete_at_caret();
    }

    pub fn backspace_word(&mut self) {
        self.osu_path.delete_word_before_caret();
    }

    pub fn caret_left(&mut self) {
        self.osu_path.caret_left();
    }

    pub fn caret_right(&mut self) {
        self.osu_path.caret_right();
    }

    pub fn caret_home(&mut self) {
        self.osu_path.caret_home();
    }

    pub fn caret_end(&mut self) {
        self.osu_path.caret_end();
    }
}

#[cfg(test)]
#[path = "../../tests/unit/library.rs"]
mod tests;
