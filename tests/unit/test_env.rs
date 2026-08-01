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

/// Several keys under ONE guard: each is live for the guard's lifetime and each
/// is restored independently on drop — a key that had a value gets it back, a
/// key that had none is removed. Two `set` guards cannot do this; the global
/// lock is not reentrant, so the second would deadlock.
#[test]
fn set_all_sets_every_key_and_restores_each_independently() {
    let had_value = "OSU_COLLECT_TEMPENV_TEST_MULTI_A";
    let was_absent = "OSU_COLLECT_TEMPENV_TEST_MULTI_B";
    // Safety: the guard serializes every set_var/remove_var globally.
    unsafe { std::env::set_var(had_value, "before") };
    unsafe { std::env::remove_var(was_absent) };
    {
        let _env = TempEnvVar::set_all([(had_value, "during-a"), (was_absent, "during-b")]);
        assert_eq!(std::env::var(had_value).unwrap(), "during-a");
        assert_eq!(std::env::var(was_absent).unwrap(), "during-b");
    }
    assert_eq!(std::env::var(had_value).unwrap(), "before");
    assert!(
        std::env::var(was_absent).is_err(),
        "a key with no previous value must be removed, not left at the override"
    );
    unsafe { std::env::remove_var(had_value) };
}

/// The same key overridden twice in one guard ends at the value it held before
/// the FIRST set: restores run in reverse of capture order.
#[test]
fn set_all_restores_a_repeated_key_to_its_pre_guard_value() {
    let key = "OSU_COLLECT_TEMPENV_TEST_MULTI_REPEAT";
    unsafe { std::env::set_var(key, "original") };
    {
        let _env = TempEnvVar::set_all([(key, "first"), (key, "second")]);
        assert_eq!(std::env::var(key).unwrap(), "second");
    }
    assert_eq!(std::env::var(key).unwrap(), "original");
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
