//! Process-global env-var override guard for tests.
//!
//! Exactly one `TempEnvVar` is live at a time: it holds a global lock for its
//! entire lifetime, so tests that manipulate the same variable are serialized
//! without needing `#[serial_test::serial]` — including across integration-test
//! binaries, where `serial_test` can't reach. A test needing several overrides
//! at once takes them in ONE guard via [`TempEnvVar::set_all`]; two guards on
//! one thread deadlock, since the lock is not reentrant.

use std::{env, sync::MutexGuard};

static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Guard that sets one or more env vars for its lifetime and restores each
/// previous value (or removes the key) on drop. The global lock is held for the
/// entire lifetime, so two `TempEnvVar`s can never coexist.
///
/// Use [`TempEnvVar::set`] or [`TempEnvVar::set_all`]; never construct this
/// directly.
#[must_use = "the env override is restored on drop; bind to a let binding to keep it alive"]
pub struct TempEnvVar {
    /// Captured in set order and restored in reverse, so a key overridden twice
    /// in one guard ends at the value it held before the first set.
    restore: Vec<(&'static str, Option<String>)>,
    _guard: MutexGuard<'static, ()>,
}

impl TempEnvVar {
    /// Set `key=value` and return a guard that restores the previous value on drop.
    ///
    /// # Panics
    /// Panics if the global env lock is poisoned.
    pub fn set(key: &'static str, value: &str) -> Self {
        Self::set_all([(key, value)])
    }

    /// Set every `(key, value)` pair under a single guard, restoring each on
    /// drop. One guard rather than several because the global lock is not
    /// reentrant: a test that needs two overrides live at once cannot take two
    /// [`set`](Self::set) guards, it deadlocks.
    ///
    /// # Panics
    /// Panics if the global env lock is poisoned.
    pub fn set_all<'a, I>(vars: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, &'a str)>,
    {
        let guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let mut restore = Vec::new();
        for (key, value) in vars {
            restore.push((key, env::var(key).ok()));
            // SAFETY: the global lock is held for the guard's entire lifetime, so no
            // other thread can read or write this key concurrently.
            unsafe { env::set_var(key, value) };
        }
        Self {
            restore,
            _guard: guard,
        }
    }
}

impl Drop for TempEnvVar {
    fn drop(&mut self) {
        // SAFETY: `_guard` is a field of this struct, and fields are dropped
        // only after this body returns, so the global env lock is still held for
        // every call below — no other `TempEnvVar` can read or write these keys
        // concurrently. Same invariant `set_all` relies on.
        for (key, prev) in self.restore.iter().rev() {
            match prev {
                Some(prev) => unsafe { env::set_var(key, prev) },
                None => unsafe { env::remove_var(key) },
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/test_env.rs"]
mod tests;
