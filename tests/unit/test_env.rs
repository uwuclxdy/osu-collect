use crate::test_env::TempEnvVar;

#[test]
fn sets_and_restores_previous_value() {
    let key = "OSU_COLLECT_TEMPENV_TEST_SETS";
    // Safety: the guard serializes every set_var/remove_var globally.
    unsafe { std::env::set_var(key, "before") };
    {
        let _env = TempEnvVar::set(key, "during");
        assert_eq!(std::env::var(key).unwrap(), "during");
    }
    assert_eq!(std::env::var(key).unwrap(), "before");
    unsafe { std::env::remove_var(key) };
}

#[test]
fn removes_when_no_previous_value() {
    let key = "OSU_COLLECT_TEMPENV_TEST_REMOVES";
    // Ensure key is absent before we start.
    unsafe { std::env::remove_var(key) };
    {
        let _env = TempEnvVar::set(key, "ephemeral");
        assert_eq!(std::env::var(key).unwrap(), "ephemeral");
    }
    assert!(
        std::env::var(key).is_err(),
        "key must be removed after the guard drops"
    );
}
