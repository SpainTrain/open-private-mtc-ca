//! The MTC tree hash function and its domain-separated leaf/interior-node
//! constructions.
//!
//! # Construction (cited)
//!
//! The issuance log is an append-only Merkle tree (spec section 2, "Issuance
//! log"). Its node hashes follow the RFC 9162 §2.1.1 Merkle Tree Hash (MTH)
//! construction, which `draft-ietf-plants-merkle-tree-certs-03` adopts and
//! which the `tlog-tiles` serving format (spec section 28) is built on:
//!
//! - **empty tree**: `MTH({}) = HASH()` — the hash of the empty input
//!   (RFC 9162 §2.1.1);
//! - **leaf** (one entry): `MTH({entry}) = HASH(0x00 || entry)`
//!   (draft-03 §7.2 "Let `entry_hash` be … `HASH(0x00 || entry)`");
//! - **interior node** (two children `l`, `r`):
//!   `HASH(0x01 || l || r)` (draft-03 §4.3.2 inclusion-proof evaluation
//!   "Set `r` to `HASH(0x01 || p || r)`").
//!
//! `HASH` is SHA-256 for v1 (spec section 4 "Algorithm (v1) — ECDSA P-256"
//! signs checkpoints, but the tree hash itself is SHA-256; draft-03 §5.1
//! "SHA-256 … is RECOMMENDED"). The function is abstracted behind the
//! [`Hasher`] trait so tree operations are generic `<H: Hasher>` and
//! monomorphize to the concrete hash on the hot path (spec section 22.7,
//! "Tree updater hash function" row).
//!
//! # Domain separation (second-preimage resistance)
//!
//! The `0x00` / `0x01` prefixes are **not decoration**: they domain-separate
//! leaf preimages from interior-node preimages. Every leaf hash is computed
//! over an input whose first byte is `0x00`; every interior-node hash over an
//! input whose first byte is `0x01`. Because the two preimage languages are
//! disjoint in their first byte, no interior node hash can also be a valid leaf
//! hash (absent a SHA-256 collision). That closes the classic Merkle-tree
//! second-preimage attack in which an attacker presents an interior node as if
//! it were a leaf — the attack RFC 6962 introduced these prefixes to prevent.
//! The construction is defined **once** here, in [`hash_leaf`] / [`hash_node`],
//! and is not overridable by a [`Hasher`] implementation (implementations
//! supply only the raw [`Hasher::digest`]); this module is the single audit
//! point for the domain-separation invariant (property-tested below and per
//! spec sections 19.2 and 19.6).

use crate::types::HashOutput;

/// Domain-separation prefix for **leaf** hashes: `HASH(0x00 || entry)`.
///
/// Per draft-ietf-plants-merkle-tree-certs-03 §7.2 / RFC 9162 §2.1.1. Exposed
/// so conformance vectors and the differential oracle can assert the exact
/// byte.
pub const LEAF_PREFIX: u8 = 0x00;

/// Domain-separation prefix for **interior-node** hashes:
/// `HASH(0x01 || left || right)`.
///
/// Per draft-ietf-plants-merkle-tree-certs-03 §4.3.2 / RFC 9162 §2.1.1. Exposed
/// so conformance vectors and the differential oracle can assert the exact
/// byte. Must differ from [`LEAF_PREFIX`] — that difference is the domain
/// separation.
pub const NODE_PREFIX: u8 = 0x01;

/// The fixed-output cryptographic hash function underlying the Merkle tree —
/// the spec's `HASH` (draft-ietf-plants-merkle-tree-certs-03 §5.1).
///
/// This is the tree hash function, **not** [`core::hash::Hasher`] (which backs
/// `#[derive(Hash)]`). Implementations supply only [`digest`](Hasher::digest),
/// the raw hash of a sequence of byte slices; the domain-separated leaf and
/// interior-node constructions ([`hash_leaf`], [`hash_node`], [`empty_root`])
/// are defined once on top of it and are not part of the trait, so an
/// implementation cannot accidentally weaken the domain separation.
///
/// It is a trait (rather than a hard-wired `Sha256`) for algorithm agility and
/// because tree operations are generic over it for static dispatch on the hot
/// path (spec section 22.7). [`Sha256Hasher`] is the v1 implementation and the
/// default type parameter of [`MerkleTree`](crate::MerkleTree).
pub trait Hasher {
    /// Hashes the in-order concatenation of `parts` and returns the digest.
    ///
    /// Taking a slice of slices lets callers hash a domain prefix followed by
    /// payload bytes without allocating an intermediate buffer on the hot path
    /// (spec section 22.7). `digest(&[])` is the hash of the empty input.
    #[must_use]
    fn digest(parts: &[&[u8]]) -> HashOutput;
}

/// SHA-256 implementation of [`Hasher`] (`RustCrypto` `sha2`; spec section 28).
///
/// A zero-sized marker type: it carries no state, so `MerkleTree<Sha256Hasher>`
/// stores no hasher instance and every call monomorphizes to a direct SHA-256
/// invocation (spec section 22.7).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Sha256Hasher;

impl Hasher for Sha256Hasher {
    fn digest(parts: &[&[u8]]) -> HashOutput {
        use sha2::{Digest as _, Sha256};

        let mut hasher = Sha256::new();
        for part in parts {
            hasher.update(part);
        }
        HashOutput(hasher.finalize().into())
    }
}

/// The root hash of the **empty** tree: `HASH()` (RFC 9162 §2.1.1).
///
/// This is the checkpoint root of an issuance log that has committed no
/// entries. For [`Sha256Hasher`] it is the well-known SHA-256 of the empty
/// input; see [`SHA256_EMPTY_ROOT`], which locks the value.
#[must_use]
pub fn empty_root<H: Hasher>() -> HashOutput {
    H::digest(&[])
}

/// The **leaf** hash of a single log entry: `HASH(0x00 || entry)`
/// (draft-03 §7.2 / RFC 9162 §2.1.1).
///
/// `entry` is the already-encoded log-entry bytes (this crate's tree layer is
/// byte-oriented; entry encoding lands with `mtc-serialization`). The
/// [`LEAF_PREFIX`] byte is what domain-separates this from [`hash_node`].
#[must_use]
pub fn hash_leaf<H: Hasher>(entry: &[u8]) -> HashOutput {
    H::digest(&[&[LEAF_PREFIX], entry])
}

/// The **interior-node** hash of two child hashes:
/// `HASH(0x01 || left || right)` (draft-03 §4.3.2 / RFC 9162 §2.1.1).
///
/// The [`NODE_PREFIX`] byte is what domain-separates this from [`hash_leaf`],
/// giving second-preimage resistance (see the module docs).
#[must_use]
pub fn hash_node<H: Hasher>(left: &HashOutput, right: &HashOutput) -> HashOutput {
    H::digest(&[&[NODE_PREFIX], left.as_bytes(), right.as_bytes()])
}

/// The locked value of [`empty_root::<Sha256Hasher>`](empty_root): SHA-256 of
/// the empty input,
/// `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
///
/// Provided as a constant so consumers (e.g. an empty-log checkpoint) need not
/// recompute it, and locked against the computed value by a unit test
/// (spec section 19.6, "Empty tree root hash is well-defined and constant").
pub const SHA256_EMPTY_ROOT: HashOutput = HashOutput([
    0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24,
    0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55,
]);

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        empty_root, hash_leaf, hash_node, Hasher, Sha256Hasher, LEAF_PREFIX, NODE_PREFIX,
        SHA256_EMPTY_ROOT,
    };
    use crate::types::HashOutput;

    #[test]
    fn domain_prefixes_are_distinct() {
        // The whole domain-separation guarantee reduces to this inequality.
        assert_ne!(LEAF_PREFIX, NODE_PREFIX);
        assert_eq!(LEAF_PREFIX, 0x00);
        assert_eq!(NODE_PREFIX, 0x01);
    }

    #[test]
    fn empty_root_is_locked_constant() {
        // Spec section 19.6: empty tree root hash is well-defined and constant.
        let computed = empty_root::<Sha256Hasher>();
        assert_eq!(computed, SHA256_EMPTY_ROOT);
        // Known-answer: SHA-256 of the empty string.
        assert_eq!(
            format!("{computed:?}"),
            "HashOutput(e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855)",
        );
    }

    #[test]
    fn leaf_hash_matches_manual_construction() {
        // HASH(0x00 || entry) computed independently of hash_leaf.
        let entry = b"log entry bytes";
        let mut manual = Vec::with_capacity(1 + entry.len());
        manual.push(LEAF_PREFIX);
        manual.extend_from_slice(entry);
        let expected = Sha256Hasher::digest(&[&manual]);
        assert_eq!(hash_leaf::<Sha256Hasher>(entry), expected);
    }

    #[test]
    fn node_hash_matches_manual_construction() {
        // HASH(0x01 || left || right) computed independently of hash_node.
        let left = HashOutput([0x11; 32]);
        let right = HashOutput([0x22; 32]);
        let mut manual = Vec::with_capacity(1 + 64);
        manual.push(NODE_PREFIX);
        manual.extend_from_slice(left.as_bytes());
        manual.extend_from_slice(right.as_bytes());
        let expected = Sha256Hasher::digest(&[&manual]);
        assert_eq!(hash_node::<Sha256Hasher>(&left, &right), expected);
    }

    #[test]
    fn node_hash_is_not_symmetric() {
        // Child order is part of the preimage: swapping children changes the
        // hash (a right sibling must not verify as a left sibling).
        let a = HashOutput([0xaa; 32]);
        let b = HashOutput([0xbb; 32]);
        assert_ne!(
            hash_node::<Sha256Hasher>(&a, &b),
            hash_node::<Sha256Hasher>(&b, &a),
        );
    }

    proptest! {
        // Spec sections 19.2 / 19.6: leaf hashes never collide with interior
        // node hashes. This is the second-preimage-class invariant the
        // crypto-reviewer audits; it can only fail on a real SHA-256 collision.
        #[test]
        fn leaf_and_node_hashes_never_collide(
            entry in proptest::collection::vec(any::<u8>(), 0..256),
            left in any::<[u8; 32]>(),
            right in any::<[u8; 32]>(),
        ) {
            let leaf = hash_leaf::<Sha256Hasher>(&entry);
            let node = hash_node::<Sha256Hasher>(&HashOutput(left), &HashOutput(right));
            prop_assert_ne!(leaf, node);
            // The empty-tree root is likewise distinct from any leaf or node.
            prop_assert_ne!(leaf, empty_root::<Sha256Hasher>());
            prop_assert_ne!(node, empty_root::<Sha256Hasher>());
        }

        // Even when the bytes *after* the prefix coincide, the prefix keeps a
        // leaf distinct from a node: HASH(0x00 || X) != HASH(0x01 || X) for a
        // 64-byte X read either as an entry or as two child hashes.
        #[test]
        fn shared_payload_stays_separated(l in any::<[u8; 32]>(), r in any::<[u8; 32]>()) {
            let mut payload = Vec::with_capacity(64);
            payload.extend_from_slice(&l);
            payload.extend_from_slice(&r);
            let leaf = hash_leaf::<Sha256Hasher>(&payload);
            let node = hash_node::<Sha256Hasher>(&HashOutput(l), &HashOutput(r));
            prop_assert_ne!(leaf, node);
        }

        // digest over split parts equals digest over the concatenation: the
        // multi-part API is a pure streaming hash, not a per-part hash.
        #[test]
        fn digest_is_concatenation(a in proptest::collection::vec(any::<u8>(), 0..64),
                                   b in proptest::collection::vec(any::<u8>(), 0..64)) {
            let mut whole = a.clone();
            whole.extend_from_slice(&b);
            prop_assert_eq!(
                Sha256Hasher::digest(&[&a, &b]),
                Sha256Hasher::digest(&[&whole]),
            );
        }
    }
}
