//! Merkle inclusion and consistency proofs: generation, verification, and their
//! TLS-presentation wire formats (`mtclib-inclusion-proofs`).
//!
//! An **inclusion proof** is the list of sibling hashes on the path from a leaf
//! up to the tree (or subtree) root — the object a relying party applies to a
//! signed checkpoint to prove a certificate entry is in the log (spec section 2
//! "Inclusion proof"; the read-path flow of spec section 12.1). A
//! **consistency proof** shows one checkpoint's tree is an append-only prefix of
//! a later checkpoint's tree (spec section 19.2 "Consistency proofs verify
//! between any two tree sizes").
//!
//! # Construction (cited)
//!
//! Both proofs follow the RFC 9162 constructions that
//! `draft-ietf-plants-merkle-tree-certs-03` adopts:
//!
//! - inclusion generation — RFC 9162 §2.1.3.1 (`PATH`);
//! - inclusion verification — RFC 9162 §2.1.3.2 (the `fn`/`sn` reconstruction);
//! - consistency generation — RFC 9162 §2.1.4.1 (`PROOF`/`SUBPROOF`);
//! - consistency verification — RFC 9162 §2.1.4.2.
//!
//! Every hash is computed through the domain-separated
//! [`hash_node`](crate::hash_node) / [`hash_leaf`](crate::hash_leaf) of
//! [`crate::tree::digest`] — there is **no** second hashing path — so leaf and
//! interior preimages stay separated (spec sections 19.2, 19.6) and a proof
//! reconstructs bit-for-bit the same root a [`MerkleTree`] would.
//!
//! # Verification never trusts the proof's length
//!
//! Verification validates the proof's shape against the `(index, tree_size)` (or
//! `(old_size, new_size)`) it carries **before** it hashes anything: an
//! out-of-range index, non-monotonic sizes, or a path that is too long or too
//! short is rejected as a [`ProofError`], never a panic (crypto-review: a
//! relying party must reject a malformed proof cleanly, not fault on it). This
//! is the crown-jewel property the crypto-reviewer audits.

mod consistency;
mod inclusion;

pub use consistency::ConsistencyProof;
pub use inclusion::InclusionProof;

use thiserror::Error;

/// Why a proof could not be generated or failed verification.
///
/// A library error enum (spec section 22.6; rule
/// `thiserror-for-libs-eyre-for-bins`): every failure mode is a variant a
/// relying party can match on, and none is a panic. Wire-decoding faults are
/// *not* here — those are [`WireError`](crate::WireError), surfaced by the
/// bounded parser; `ProofError` is the semantic layer on top.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProofError {
    /// The leaf index is not `< tree_size` (an empty tree makes every index
    /// out of range). Raised by inclusion generation and verification.
    #[error("leaf index {index} is out of range for tree size {tree_size}")]
    IndexOutOfRange {
        /// The offending leaf index.
        index: u64,
        /// The tree size it was checked against.
        tree_size: u64,
    },

    /// A consistency proof was asked to relate a larger old tree to a smaller
    /// new tree (`old_size > new_size`) — an append-only log never shrinks
    /// (spec section 19.2).
    #[error("non-monotonic tree sizes: old {old_size} > new {new_size}")]
    NonMonotonicSizes {
        /// The claimed earlier (old) tree size.
        old_size: u64,
        /// The claimed later (new) tree size.
        new_size: u64,
    },

    /// Generation requested a size or index the source tree does not yet cover.
    #[error("tree holds {available} entries, but the proof needs {requested}")]
    TreeTooSmall {
        /// The size or index the caller requested.
        requested: u64,
        /// The number of entries actually in the tree.
        available: u64,
    },

    /// The proof's path length does not match the length the `(index, size)`
    /// pair implies. Detected **before** any hashing, so an overlong or short
    /// path is rejected without work (crypto-review: validate length first).
    #[error("malformed proof: expected {expected} path element(s), found {actual}")]
    MalformedPath {
        /// The path length implied by the proof's declared sizes/index.
        expected: usize,
        /// The path length actually present.
        actual: usize,
    },

    /// The path is well-formed but the root it reconstructs does not equal the
    /// expected (checkpoint) root — the proof is simply invalid.
    #[error("proof does not reconstruct the expected root")]
    RootMismatch,
}

/// The largest power of two strictly less than `n`, for `n >= 2` (RFC 9162
/// §2.1.1: "let `k` be the largest power of two smaller than `n`").
///
/// Shared by inclusion and consistency generation/verification so all three
/// derive their split points identically. Mirrors `tree::builder::split_point`
/// but over `u64` (proofs reason about tree *sizes*, not in-memory slice
/// lengths).
const fn largest_power_of_two_below(n: u64) -> u64 {
    debug_assert!(n >= 2, "largest_power_of_two_below requires n >= 2");
    // `(n - 1).ilog2()` is `floor(log2(n - 1))`; `1 << that` is the largest
    // power of two `<= n - 1`, i.e. strictly less than `n`. E.g. 8 -> 4, 5 -> 4.
    1u64 << (n - 1).ilog2()
}

#[cfg(test)]
mod tests {
    use super::largest_power_of_two_below;

    #[test]
    fn largest_power_of_two_below_matches_definition() {
        assert_eq!(largest_power_of_two_below(2), 1);
        assert_eq!(largest_power_of_two_below(3), 2);
        assert_eq!(largest_power_of_two_below(4), 2);
        assert_eq!(largest_power_of_two_below(5), 4);
        assert_eq!(largest_power_of_two_below(7), 4);
        assert_eq!(largest_power_of_two_below(8), 4);
        assert_eq!(largest_power_of_two_below(9), 8);
    }
}
