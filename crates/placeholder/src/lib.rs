//! Placeholder crate for the MTC-CA workspace.
//!
//! This crate carries no real logic. It exists so the Cargo workspace has at
//! least one member and its `cargo test` / `cargo test --doc` harnesses are
//! exercised from a fresh clone. It is expected to be deleted once the first
//! domain crate lands (see the `core-mtc-library` and `cloud-abstraction`
//! epics); nothing should come to depend on it.

/// Returns the crate's placeholder marker string.
///
/// Exists purely to give the workspace a passing unit test and doctest, so the
/// test harnesses are wired from day one.
///
/// ```
/// assert_eq!(mtc_placeholder::marker(), "mtc-placeholder");
/// ```
#[must_use]
pub fn marker() -> &'static str {
    "mtc-placeholder"
}

#[cfg(test)]
mod tests {
    use super::marker;

    #[test]
    fn marker_is_stable() {
        assert_eq!(marker(), "mtc-placeholder");
    }
}
