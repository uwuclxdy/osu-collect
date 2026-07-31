//! Process-global env-var override guard for tests.
//!
//! Each `TempEnvVar` holds a global lock for its entire lifetime, so only one
//! env-var override is live at a time. Tests that manipulate the same variable
//! are serialized without needing `#[serial_test::serial]` — including across
//! integration-test binaries, where `serial_test` can't reach.

use std::{env, sync::MutexGuard};

static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Guard that sets an env var for its lifetime and restores the previous value
/// (or removes the key) on drop. The global lock is held for the entire
/// lifetime, so two `TempEnvVar`s can never coexist.
///
/// Use [`TempEnvVar::set`]; never construct this directly.
#[must_use = "the env override is restored on drop; bind to a let binding to keep it alive"]
pub struct TempEnvVar {
    key: &'static str,
    prev: Option<String>,
    _guard: MutexGuard<'static, ()>,
}

impl TempEnvVar {
    /// Set `key=value` and return a guard that restores the previous value on drop.
    ///
    /// # Panics
    /// Panics if the global env lock is poisoned.
    pub fn set(key: &'static str, value: &str) -> Self {
        let guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let prev = env::var(key).ok();
        // SAFETY: the global lock is held for the guard's entire lifetime, so no
        // other thread can read or write this key concurrently.
        unsafe { env::set_var(key, value) };
        Self {
            key,
            prev,
            _guard: guard,
        }
    }
}

impl Drop for TempEnvVar {
    fn drop(&mut self) {
        match &self.prev {
            Some(prev) => unsafe { env::set_var(self.key, prev) },
            None => unsafe { env::remove_var(self.key) },
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/test_env.rs"]
mod tests;
