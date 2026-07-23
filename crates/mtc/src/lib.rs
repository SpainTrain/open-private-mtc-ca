//! Core domain types for the Merkle Tree Certificate (MTC) CA.
//!
//! This crate is a clean-room Rust implementation of the primitive concepts of
//! `draft-ietf-plants-merkle-tree-certs` as catalogued in the architecture
//! spec, section 2 ("Background: What is MTC"). It contains *types only*:
//!
//! - integer domain newtypes ([`Index`], [`TreeSize`], [`Epoch`]) per spec
//!   section 22.1,
//! - the fixed-size hash newtype ([`HashOutput`]) for SHA-256 tree hashes,
//! - phantom-typed string identifiers ([`Id`], [`LogId`], [`BatchId`]) per
//!   spec section 22.5,
//! - the shared `thiserror` error enums for constructing them (spec
//!   section 22.6; rule `thiserror-for-libs-eyre-for-bins`).
//!
//! Building on these types, the crate also provides Merkle tree operations
//! (the [`tree`] module, `tree-primitives` ticket) and the TLS-presentation
//! wire-format codec (the [`wire`] module, `mtc-serialization` ticket).
//! Checkpoint signing remains out of scope here and arrives with its own
//! ticket.
//!
//! # Lint posture (spec section 22.12)
//!
//! `unsafe_code` is forbidden workspace-wide; this crate additionally denies
//! `missing_docs` and (outside tests) `clippy::unwrap_used` /
//! `clippy::expect_used`, and warns on `clippy::pedantic` / `clippy::nursery`
//! / `clippy::cargo`.

#![deny(missing_docs)]
#![warn(clippy::pedantic, clippy::nursery, clippy::cargo)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
// The dev-dependency tree (trybuild) is outside this crate's control; version
// duplication there is the integrator's concern, not this library's API.
#![allow(clippy::multiple_crate_versions)]

pub mod error;
pub mod signing;
pub mod tree;
pub mod types;
pub mod wire;

pub use error::{HashOutputError, IdError};
pub use signing::{
    scheme_for, EcdsaP256, KeyRejected, SignError, Signature, SignatureAlgorithm, SignatureScheme,
    SigningKey, UnknownAlgorithm, UnsupportedAlgorithm, VerifyError, VerifyingKey,
};
pub use tree::{
    decompose_range, empty_root, hash_leaf, hash_node, Hasher, MerkleTree, Sha256Hasher, Subtree,
    LEAF_PREFIX, NODE_PREFIX, SHA256_EMPTY_ROOT,
};
pub use types::{BatchId, BatchTag, Epoch, HashOutput, Id, Index, LogId, LogTag, TreeSize};
pub use wire::{TlsParse, TlsReader, TlsSerialize, WireError, U24};
