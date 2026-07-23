//! Shared library error types (spec section 22.6; rule
//! `thiserror-for-libs-eyre-for-bins`).
//!
//! Every fallible constructor in this crate returns one of these `thiserror`
//! enums so failure modes stay in the function signature and callers can match
//! on each variant. No `unwrap()`/`expect()` appears in library code (rule
//! `no-unwrap-in-prod`).

use thiserror::Error;

/// Errors constructing a phantom-typed domain identifier ([`crate::Id`]).
///
/// Identifiers name spec section-2 artifacts (an issuance log, a batch of
/// consecutive entries), so an empty identifier can never refer to anything
/// and is rejected at the type boundary.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    /// The supplied identifier string was empty.
    #[error("domain identifier must not be empty")]
    Empty,
}

/// Errors constructing a [`crate::HashOutput`] from raw bytes.
///
/// A hash output models a SHA-256 node hash of the issuance log's Merkle tree
/// (spec section 2), which is exactly 32 bytes; any other length is a caller
/// bug surfaced as a typed error rather than a panic.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Error)]
pub enum HashOutputError {
    /// The supplied byte slice was not exactly the required length.
    #[error("hash output must be exactly {expected} bytes, got {actual}")]
    InvalidLength {
        /// The required length in bytes ([`crate::HashOutput::LEN`]).
        expected: usize,
        /// The length of the slice actually supplied.
        actual: usize,
    },
}
