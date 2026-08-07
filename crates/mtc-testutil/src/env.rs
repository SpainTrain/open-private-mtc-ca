//! Temp-directory and process-environment helpers for tests.
//!
//! Spec §19.1: unit tests have no external dependencies, so a test that
//! touches the filesystem or the process environment must clean up after
//! itself deterministically rather than leaking state to the next test.

use std::env;
use std::ffi::{OsStr, OsString};

pub use tempfile::TempDir;

/// Creates a fresh, uniquely-named temp directory scoped to the MTC-CA test
/// suite, removed when the returned [`TempDir`] is dropped.
///
/// A thin wrapper over [`tempfile::Builder`] with a recognizable prefix, so a
/// leaked directory (e.g. a test panicking before its guard would normally
/// run) is easy to spot in `/tmp` during debugging.
///
/// # Errors
///
/// Propagates [`std::io::Error`] if the OS cannot create the directory (spec
/// §22.6 — never a panic, even in test-support code; see the crate docs for
/// why this crate's own source follows the production error-handling rules).
pub fn temp_dir() -> std::io::Result<TempDir> {
    tempfile::Builder::new().prefix("mtc-testutil-").tempdir()
}

/// Sets a process environment variable and restores its previous value (or
/// removes it, if it was previously unset) when dropped.
///
/// Process environment is global mutable state shared by the whole test
/// binary, so callers must scope the guard as narrowly as possible and use a
/// variable name no concurrently-running test touches (the standard caveat
/// for any `std::env::set_var`-based test helper — `cargo test` runs tests in
/// parallel threads within one process by default).
#[must_use = "the environment variable is restored when this guard drops; binding it to `_` restores it immediately"]
pub struct EnvVarGuard {
    key: OsString,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    /// Sets `key` to `value`, remembering the prior value (if any) so it can
    /// be restored on drop.
    pub fn set(key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        let key = key.as_ref().to_os_string();
        let previous = env::var_os(&key);
        env::set_var(&key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => env::set_var(&self.key, value),
            None => env::remove_var(&self.key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{temp_dir, EnvVarGuard};
    use std::env;

    #[test]
    fn temp_dir_creates_a_writable_unique_directory() {
        let a = temp_dir().unwrap();
        let b = temp_dir().unwrap();
        assert!(a.path().is_dir());
        assert_ne!(a.path(), b.path());
        std::fs::write(a.path().join("probe"), b"ok").unwrap();
        assert_eq!(std::fs::read(a.path().join("probe")).unwrap(), b"ok");
    }

    #[test]
    fn temp_dir_is_removed_on_drop() {
        let dir = temp_dir().unwrap();
        let path = dir.path().to_path_buf();
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn env_var_guard_restores_a_previously_set_value() {
        let key = "MTC_TESTUTIL_ENV_GUARD_PROBE_RESTORE";
        env::set_var(key, "original");
        {
            let guard = EnvVarGuard::set(key, "overridden");
            assert_eq!(env::var(key).unwrap(), "overridden");
            drop(guard);
        }
        assert_eq!(env::var(key).unwrap(), "original");
        env::remove_var(key);
    }

    #[test]
    fn env_var_guard_removes_a_previously_unset_variable() {
        let key = "MTC_TESTUTIL_ENV_GUARD_PROBE_REMOVE";
        env::remove_var(key);
        {
            let guard = EnvVarGuard::set(key, "temporary");
            assert_eq!(env::var(key).unwrap(), "temporary");
            drop(guard);
        }
        assert!(env::var(key).is_err());
    }
}
