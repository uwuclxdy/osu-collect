use crate::app::collection_state::{self, CollectionStateFile, STATE_ENV_PATH};
use std::fs;
use tempfile::tempdir;

#[test]
fn load_missing_file_returns_default() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("does_not_exist.toml");
    let state = collection_state::load(&path);
    assert!(state.collections.is_empty());
}

#[test]
fn save_and_load_round_trip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("collection_state.toml");

    let mut state = CollectionStateFile {
        schema_version: 1,
        collections: Default::default(),
    };
    state.update(42, vec![100, 200, 300], vec![100, 300], vec![]);
    state.update(99, vec![1, 2], vec![2], vec![]);

    collection_state::save(&state, &path);
    assert!(path.exists(), "state file must exist after save");

    let loaded = collection_state::load(&path);
    let rec42 = &loaded.collections[&42].last_seen_beatmapsets;
    assert_eq!(rec42.len(), 3);
    assert!(rec42.contains(&100));
    assert!(rec42.contains(&200));
    assert!(rec42.contains(&300));

    let installed42 = loaded.last_installed_at_scan(42);
    assert_eq!(installed42.len(), 2);
    assert!(installed42.contains(&100));
    assert!(installed42.contains(&300));

    let rec99 = &loaded.collections[&99].last_seen_beatmapsets;
    assert_eq!(rec99.len(), 2);
}

#[test]
fn save_is_atomic_no_tmp_left_on_success() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.toml");
    let tmp = path.with_extension("toml.tmp");

    let mut state = CollectionStateFile::default();
    state.update(1, vec![10, 20], vec![10], vec![]);
    collection_state::save(&state, &path);

    assert!(path.exists(), "final file must exist");
    assert!(!tmp.exists(), "tmp file must be renamed away");
}

#[test]
fn load_corrupt_file_returns_default() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("corrupt.toml");
    fs::write(&path, b"this is not valid toml ][[[").unwrap();

    let state = collection_state::load(&path);
    assert!(state.collections.is_empty(), "corrupt file → empty default");
}

#[test]
fn last_scan_accessors_unknown_collection_return_empty_slices() {
    let state = CollectionStateFile::default();
    assert!(state.last_seen_remote(999).is_empty());
    assert!(state.last_installed_at_scan(999).is_empty());
}

#[test]
fn update_overwrites_previous_record() {
    let mut state = CollectionStateFile::default();
    state.update(5, vec![1, 2, 3], vec![1], vec![]);
    state.update(5, vec![10, 20], vec![20], vec![]);

    let seen = &state.collections[&5].last_seen_beatmapsets;
    assert_eq!(seen.len(), 2);
    assert!(seen.contains(&10));
    assert!(seen.contains(&20));
    assert!(!seen.contains(&1));

    let installed = state.last_installed_at_scan(5);
    assert_eq!(installed, &[20]);
}

#[test]
fn state_path_respects_env_override() {
    let dir = tempdir().unwrap();
    let custom = dir.path().join("custom_state.toml");
    let _env = crate::test_env::TempEnvVar::set(STATE_ENV_PATH, custom.to_str().unwrap());
    let path = collection_state::state_path();
    assert_eq!(path.unwrap(), custom);
}
