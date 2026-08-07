//! Per-operation command implementations.
//!
//! Only [`status`] is wired to the admin API today; [`not_implemented`] is
//! what every other leaf in the spec §17.3 tree resolves to via
//! [`crate::run`] (ticket mtc-no9's Out of Scope: "Individual operation
//! implementations (per-operation tickets)").

pub mod status;

use crate::error::CliError;

/// Resolves an unimplemented leaf to a distinct, non-zero-exit error.
///
/// Ticket mtc-no9 AC: "unimplemented leaves exit non-zero with a clear 'not
/// yet implemented' message". `command` is the leaf's full path, e.g.
/// `"batch list"`, for a message that names exactly what wasn't run.
#[must_use]
pub const fn not_implemented(command: &'static str) -> CliError {
    CliError::NotImplemented(command)
}
