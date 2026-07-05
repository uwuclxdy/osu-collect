use crate::app::LibraryState;
use crate::config::Config;
use crate::osu_db::OsuClient;

#[test]
fn from_config_seeds_saved_path_and_client_verbatim() {
    let mut config = Config::default();
    config.recent.osu_client = Some(OsuClient::Stable);
    config.recent.osu_path = Some("/custom/osu".to_string());

    let lib = LibraryState::from_config(&config);

    assert_eq!(lib.client_type, OsuClient::Stable);
    assert_eq!(
        lib.osu_path.value, "/custom/osu",
        "a saved path is kept verbatim, even if it no longer exists"
    );
}

#[test]
fn from_config_falls_back_to_detection_when_blank() {
    // A blank saved path is ignored: the value falls back to the detected
    // default (its own placeholder), never an empty hand-typed value.
    let mut config = Config::default();
    config.recent.osu_path = Some("   ".to_string());

    let lib = LibraryState::from_config(&config);

    assert_eq!(lib.osu_path.value, lib.osu_path.placeholder);
}

#[test]
fn switch_client_toggles_kind_and_keeps_auto_detected_path_tracking() {
    let mut lib = LibraryState::new(OsuClient::Stable);
    assert!(
        lib.is_path_auto_detected(),
        "a freshly built state holds its detected default"
    );

    lib.switch_client();

    assert_eq!(lib.client_type, OsuClient::Lazer, "client kind flips");
    assert!(
        lib.is_path_auto_detected(),
        "an auto-detected path re-detects to the new client's default"
    );
}

#[test]
fn switch_client_preserves_a_hand_typed_path() {
    let mut lib = LibraryState::new(OsuClient::Stable);
    lib.osu_path.set_value("/hand/typed/osu".to_string());

    lib.switch_client();

    assert_eq!(lib.client_type, OsuClient::Lazer);
    assert_eq!(
        lib.osu_path.value, "/hand/typed/osu",
        "a hand-typed path is not overwritten by the client switch"
    );
    assert!(!lib.is_path_auto_detected());
}
