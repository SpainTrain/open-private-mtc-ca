//! Merkle tree core for the issuance log (spec section 2, "Issuance log").
//!
//! This module implements the tree math the CA's append-only log is built on,
//! per the RFC 9162 §2.1.1 Merkle Tree Hash construction adopted by
//! `draft-ietf-plants-merkle-tree-certs-03` (see the submodule docs for the
//! cited sections):
//!
//! - [`digest`] — the [`Hasher`] trait (the spec's `HASH`), its SHA-256
//!   implementation [`Sha256Hasher`], and the domain-separated leaf / interior
//!   constructions ([`hash_leaf`], [`hash_node`], [`empty_root`]). This is the
//!   single audit point for the leaf/interior domain separation (spec sections
//!   19.2, 19.6).
//! - [`builder`] — [`MerkleTree`]: append, root, and subtree hashing.
//! - [`decomposition`] — [`decompose_range`] and [`Subtree`]: splitting an
//!   arbitrary entry range into aligned power-of-two subtrees.
//!
//! Inclusion/consistency proofs, tiles, range pruning, entry serialization, and
//! persistence are intentionally elsewhere (see the crate-level docs and the
//! submodule notes).

pub mod builder;
pub mod decomposition;
pub mod digest;

pub use builder::MerkleTree;
pub use decomposition::{decompose_range, Subtree, SubtreeError};
pub use digest::{
    empty_root, hash_leaf, hash_node, Hasher, Sha256Hasher, LEAF_PREFIX, NODE_PREFIX,
    SHA256_EMPTY_ROOT,
};
