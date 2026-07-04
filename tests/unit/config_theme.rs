use crate::config::{Config, ThemeMode, load_config_from, save_config};
use std::fs;

fn write_toml(dir: &std::path::Path, contents: &str) -> std::path::PathBuf {
    let path = dir.join("config.toml");
    fs::write(&path, contents).unwrap();
    path
}

#[test]
fn theme_field_roundtrips_through_save_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut config = Config::default();
    config.display.theme = Some(ThemeMode::Compatible);

    // Use the env-var override to point save_config at our temp dir
    unsafe { std::env::set_var("OSU_COLLECT_CONFIG", path.to_str().unwrap()) };
    save_config(&config).expect("save must succeed");
    let loaded = load_config_from(&path).expect("load must succeed");
    unsafe { std::env::remove_var("OSU_COLLECT_CONFIG") };

    assert_eq!(
        loaded.display.theme,
        Some(ThemeMode::Compatible),
        "theme variant must survive save+load"
    );
}

#[test]
fn theme_defaults_to_none_when_absent_from_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_toml(dir.path(), "[mirror]\nnerinyan = true\n");
    let loaded = load_config_from(&path).expect("load must succeed");
    assert_eq!(
        loaded.display.theme, None,
        "missing display.theme must stay None so startup detection can decide"
    );
}

#[test]
fn legacy_no_video_migrates_and_unknown_keys_are_stripped_on_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_toml(
        dir.path(),
        "garbage_key = 1\n[download]\nno_video = true\nbogus_field = \"x\"\n",
    );

    let loaded = load_config_from(&path).expect("load must succeed");

    // no_video = true inverts to video = false.
    assert!(
        !loaded.download.video,
        "legacy no_video = true must load as video = false"
    );

    // Startup cleanup re-serialized the clean Config back to disk, so neither
    // the migrated-away key nor the unknown keys remain.
    let on_disk = fs::read_to_string(&path).unwrap();
    assert!(
        !on_disk.contains("no_video"),
        "no_video must be gone from the file after load:\n{on_disk}"
    );
    assert!(
        !on_disk.contains("garbage_key"),
        "unknown top-level key must be stripped after load:\n{on_disk}"
    );
    assert!(
        !on_disk.contains("bogus_field"),
        "unknown download key must be stripped after load:\n{on_disk}"
    );
    assert!(
        on_disk.contains("video"),
        "the migrated video key must be present after load:\n{on_disk}"
    );
}

#[test]
fn update_defaults_to_auto_on() {
    let config: crate::config::Config = toml::from_str("").unwrap();
    assert!(config.update.auto_update);
}

#[test]
fn all_theme_variants_serialize_and_deserialize() {
    let cases = [
        (ThemeMode::Full, "full"),
        (ThemeMode::Compatible, "compatible"),
    ];
    for (variant, toml_str) in cases {
        let toml = format!("[display]\ntheme = \"{toml_str}\"\n");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, &toml).unwrap();
        let loaded = load_config_from(&path).expect("load must succeed");
        assert_eq!(
            loaded.display.theme,
            Some(variant),
            "{toml_str} must deserialize to correct variant"
        );
    }
}

#[test]
fn ordered_builtins_defaults_to_canonical_order() {
    use crate::mirrors::MirrorKind;
    let mirror = Config::default().mirror;
    assert_eq!(mirror.ordered_builtins(), MirrorKind::BUILTINS.to_vec());
}

#[test]
fn ordered_builtins_puts_saved_hosts_first_then_appends_missing() {
    use crate::mirrors::MirrorKind;
    let mut config = Config::default();
    config.mirror.order = vec![
        MirrorKind::Catboy.host().into(),
        MirrorKind::Nekoha.host().into(),
    ];
    let ordered = config.mirror.ordered_builtins();
    assert_eq!(ordered[0], MirrorKind::Catboy);
    assert_eq!(ordered[1], MirrorKind::Nekoha);
    // Every built-in still appears exactly once — the field never hides or
    // duplicates a mirror.
    assert_eq!(ordered.len(), MirrorKind::BUILTINS.len());
    for kind in MirrorKind::BUILTINS {
        assert_eq!(
            ordered.iter().filter(|k| *k == kind).count(),
            1,
            "{kind:?} appears exactly once"
        );
    }
}

#[test]
fn ordered_builtins_drops_unknown_hosts_and_dedups() {
    use crate::mirrors::MirrorKind;
    let mut config = Config::default();
    config.mirror.order = vec![
        "ghost.example".into(),
        MirrorKind::Sayobot.host().into(),
        MirrorKind::Sayobot.host().into(),
    ];
    let ordered = config.mirror.ordered_builtins();
    assert_eq!(ordered[0], MirrorKind::Sayobot);
    assert_eq!(ordered.len(), MirrorKind::BUILTINS.len());
}

#[test]
fn mirror_order_deserializes_from_toml() {
    use crate::mirrors::MirrorKind;
    let dir = tempfile::tempdir().unwrap();
    let toml = format!(
        "[mirror]\norder = [\"{}\", \"{}\"]\n",
        MirrorKind::Catboy.host(),
        MirrorKind::OsuDirect.host()
    );
    let path = write_toml(dir.path(), &toml);
    let loaded = load_config_from(&path).expect("load must succeed");
    let ordered = loaded.mirror.ordered_builtins();
    assert_eq!(ordered[0], MirrorKind::Catboy);
    assert_eq!(ordered[1], MirrorKind::OsuDirect);
}
