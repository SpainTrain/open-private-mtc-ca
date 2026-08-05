//! Inclusion proofs: a leaf's audit path to the tree root (RFC 9162 §2.1.3,
//! adopted by `draft-ietf-plants-merkle-tree-certs-03`; spec sections 2, 12.1).

use std::io::{self, Write};

use super::{largest_power_of_two_below, ProofError};
use crate::tree::{hash_node, Hasher, MerkleTree};
use crate::types::{HashOutput, Index, TreeSize};
use crate::wire::{write_bytes, write_vector_u16, TlsParse, TlsReader, TlsSerialize, WireError};

/// A Merkle **inclusion proof**: the sibling hashes from a leaf up to the root
/// of a tree of a given size (spec section 2, "Inclusion proof").
///
/// The proof is self-describing — it carries the `leaf_index` it proves and the
/// `tree_size` (hence root) it proves inclusion in — so [`verify`](Self::verify)
/// can validate the path length against `(index, size)` on its own, and a
/// relying party can cross-check both against the certificate and the signed
/// checkpoint (spec section 12.1 steps 5–6).
///
/// `audit_path` is ordered **leaf-to-root**: `audit_path[0]` is the sibling at
/// the leaf's own level, the last element the sibling just below the root, per
/// RFC 9162 §2.1.3.1.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InclusionProof {
    tree_size: TreeSize,
    leaf_index: Index,
    audit_path: Vec<HashOutput>,
}

impl InclusionProof {
    /// Generates the inclusion proof for `index` in `tree` (proving inclusion in
    /// the tree's current root, `tree.len()`), per RFC 9162 §2.1.3.1.
    ///
    /// The sibling hashes are read from the tree with
    /// [`MerkleTree::subtree_hash`], i.e. the same domain-separated Merkle Tree
    /// Hash the root is built from — no separate hashing path.
    ///
    /// # Errors
    ///
    /// [`ProofError::IndexOutOfRange`] if `index` is not `< tree.len()` (which
    /// includes every index of an empty tree).
    pub fn generate<H: Hasher>(tree: &MerkleTree<H>, index: Index) -> Result<Self, ProofError> {
        let tree_size = tree.len().0;
        if index.0 >= tree_size {
            return Err(ProofError::IndexOutOfRange {
                index: index.0,
                tree_size,
            });
        }
        let mut audit_path = Vec::new();
        collect_siblings(tree, 0, tree_size, index.0, &mut audit_path)?;
        Ok(Self {
            tree_size: TreeSize(tree_size),
            leaf_index: index,
            audit_path,
        })
    }

    /// Constructs a proof from already-known parts (e.g. one decoded from the
    /// wire or assembled from fetched tiles). Performs no validation; call
    /// [`verify`](Self::verify) to check it.
    #[must_use]
    pub const fn from_parts(
        tree_size: TreeSize,
        leaf_index: Index,
        audit_path: Vec<HashOutput>,
    ) -> Self {
        Self {
            tree_size,
            leaf_index,
            audit_path,
        }
    }

    /// The tree size (hence the root) this proof proves inclusion in.
    #[must_use]
    pub const fn tree_size(&self) -> TreeSize {
        self.tree_size
    }

    /// The leaf index this proof proves.
    #[must_use]
    pub const fn leaf_index(&self) -> Index {
        self.leaf_index
    }

    /// The audit path (sibling hashes), leaf-to-root order.
    #[must_use]
    pub fn audit_path(&self) -> &[HashOutput] {
        &self.audit_path
    }

    /// Verifies the proof: reconstructs the root from `leaf_hash` and the audit
    /// path and checks it equals `expected_root` (RFC 9162 §2.1.3.2).
    ///
    /// `H` is the tree's hash function (SHA-256 for v1); `leaf_hash` is the
    /// leaf's `HASH(0x00 || entry)` value ([`hash_leaf`](crate::hash_leaf)), and
    /// `expected_root` is the root a signed checkpoint commits to.
    ///
    /// Before hashing, the path length is checked against the length implied by
    /// `(leaf_index, tree_size)`; an out-of-range index or a too-long/too-short
    /// path is rejected without any hashing.
    ///
    /// # Errors
    ///
    /// - [`ProofError::IndexOutOfRange`] if `leaf_index >= tree_size`.
    /// - [`ProofError::MalformedPath`] if the audit path is not exactly the
    ///   length `(leaf_index, tree_size)` requires.
    /// - [`ProofError::RootMismatch`] if the reconstructed root differs from
    ///   `expected_root`.
    pub fn verify<H: Hasher>(
        &self,
        leaf_hash: &HashOutput,
        expected_root: &HashOutput,
    ) -> Result<(), ProofError> {
        let index = self.leaf_index.0;
        let size = self.tree_size.0;
        if index >= size {
            return Err(ProofError::IndexOutOfRange {
                index,
                tree_size: size,
            });
        }

        // Length validation happens BEFORE any hashing (crypto-review): the path
        // for (index, size) has exactly this many siblings; anything else is
        // rejected up front, so an overlong path can never drive extra hashing.
        let expected = inclusion_path_len(index, size);
        if self.audit_path.len() != expected {
            return Err(ProofError::MalformedPath {
                expected,
                actual: self.audit_path.len(),
            });
        }

        // RFC 9162 §2.1.3.2 reconstruction. `fnn` (the RFC's `fn`) and `sn`
        // track the node's position and the rightmost node's position at each
        // level; their bits decide whether each sibling is a left or right
        // child.
        let mut fnn = index;
        let mut sn = size - 1;
        let mut acc = *leaf_hash;
        for sibling in &self.audit_path {
            if sn == 0 {
                // More siblings than the tree has levels: reject (also caught by
                // the length check above, kept per the RFC's own step 4 guard).
                return Err(ProofError::MalformedPath {
                    expected,
                    actual: self.audit_path.len(),
                });
            }
            if fnn & 1 == 1 || fnn == sn {
                acc = hash_node::<H>(sibling, &acc);
                while fnn != 0 && fnn & 1 == 0 {
                    fnn >>= 1;
                    sn >>= 1;
                }
            } else {
                acc = hash_node::<H>(&acc, sibling);
            }
            fnn >>= 1;
            sn >>= 1;
        }

        if sn != 0 {
            // Path too short to reach the root.
            return Err(ProofError::MalformedPath {
                expected,
                actual: self.audit_path.len(),
            });
        }
        if acc == *expected_root {
            Ok(())
        } else {
            Err(ProofError::RootMismatch)
        }
    }
}

/// The number of siblings an inclusion proof for `index` in a tree of `size`
/// entries contains — the depth of the leaf — computed without hashing.
///
/// Precondition: `index < size` and `size >= 1`. Mirrors the split recursion of
/// [`collect_siblings`] so the length check and generation agree exactly.
const fn inclusion_path_len(index: u64, size: u64) -> usize {
    let mut len = 0usize;
    let (mut start, mut end) = (0u64, size);
    while end - start > 1 {
        let k = start + largest_power_of_two_below(end - start);
        if index < k {
            end = k;
        } else {
            start = k;
        }
        len += 1;
    }
    len
}

/// Appends the leaf-to-root sibling hashes for `index` within the subtree
/// covering leaves `[start, end)` to `out` (RFC 9162 §2.1.3.1 `PATH`).
///
/// Recursion depth is bounded by the tree height (`<= 64`). All ranges passed to
/// [`MerkleTree::subtree_hash`] are non-empty and within `[0, tree.len())` by
/// construction, so it always returns `Some`; a `None` (unreachable) is surfaced
/// as [`ProofError::IndexOutOfRange`] rather than unwrapped.
fn collect_siblings<H: Hasher>(
    tree: &MerkleTree<H>,
    start: u64,
    end: u64,
    index: u64,
    out: &mut Vec<HashOutput>,
) -> Result<(), ProofError> {
    if end - start == 1 {
        return Ok(());
    }
    let k = start + largest_power_of_two_below(end - start);
    let (sib_start, sib_end);
    if index < k {
        collect_siblings(tree, start, k, index, out)?;
        (sib_start, sib_end) = (k, end);
    } else {
        collect_siblings(tree, k, end, index, out)?;
        (sib_start, sib_end) = (start, k);
    }
    let sibling =
        tree.subtree_hash(Index(sib_start), Index(sib_end))
            .ok_or(ProofError::IndexOutOfRange {
                index,
                tree_size: end,
            })?;
    out.push(sibling);
    Ok(())
}

impl TlsSerialize for InclusionProof {
    /// Wire form (clean-room, RFC 9162 §2.1.3 `InclusionProofV2` shape in the
    /// TLS presentation language):
    ///
    /// ```text
    /// struct {
    ///     uint64 tree_size;
    ///     uint64 leaf_index;
    ///     NodeHash inclusion_path<0..2^16-1>;  // NodeHash = opaque[32]
    /// } InclusionProof;
    /// ```
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_bytes(writer, &self.tree_size.0.to_be_bytes())?;
        write_bytes(writer, &self.leaf_index.0.to_be_bytes())?;
        let arrays: Vec<[u8; 32]> = self.audit_path.iter().map(|h| h.0).collect();
        write_vector_u16(writer, &arrays)
    }
}

impl TlsParse for InclusionProof {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        // u64s are read through the bounded reader's fixed-size primitive; the
        // wire framework has no native uint64 yet, so we compose one here rather
        // than reach past the reader's bounds checks.
        let tree_size = u64::from_be_bytes(reader.read_array::<8>()?);
        let leaf_index = u64::from_be_bytes(reader.read_array::<8>()?);
        // Each NodeHash is a fixed opaque[32]; the framework enforces the u16
        // body ceiling and that the body is a whole number of 32-byte hashes.
        // No positive lower bound is imposed here: an empty path is valid (a
        // single-leaf tree), so the semantic length is checked in `verify`.
        let arrays: Vec<[u8; 32]> = reader.read_vector_u16::<[u8; 32]>()?;
        let audit_path = arrays.into_iter().map(HashOutput).collect();
        Ok(Self {
            tree_size: TreeSize(tree_size),
            leaf_index: Index(leaf_index),
            audit_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{inclusion_path_len, InclusionProof};
    use crate::tree::{hash_leaf, Sha256Hasher};
    use crate::types::{HashOutput, Index, TreeSize};
    use crate::wire::{TlsParse, TlsSerialize};
    use crate::{assert_roundtrip, MerkleTree, ProofError};

    type Tree = MerkleTree<Sha256Hasher>;

    fn tree_of(n: u64) -> Tree {
        let mut tree = Tree::new();
        for i in 0..n {
            tree.append(format!("entry-{i}").as_bytes());
        }
        tree
    }

    fn leaf_hash_of(i: u64) -> HashOutput {
        hash_leaf::<Sha256Hasher>(format!("entry-{i}").as_bytes())
    }

    #[test]
    fn empty_tree_has_no_provable_leaf() {
        let tree = Tree::new();
        assert_eq!(
            InclusionProof::generate(&tree, Index(0)).unwrap_err(),
            ProofError::IndexOutOfRange {
                index: 0,
                tree_size: 0,
            },
        );
    }

    #[test]
    fn single_leaf_proof_is_empty_and_verifies() {
        let tree = tree_of(1);
        let proof = InclusionProof::generate(&tree, Index(0)).unwrap();
        assert!(proof.audit_path().is_empty());
        // Root of a one-leaf tree is the leaf hash itself.
        proof
            .verify::<Sha256Hasher>(&leaf_hash_of(0), &tree.root())
            .unwrap();
    }

    #[test]
    fn index_out_of_range_is_rejected_by_generate_and_verify() {
        let tree = tree_of(5);
        assert_eq!(
            InclusionProof::generate(&tree, Index(5)).unwrap_err(),
            ProofError::IndexOutOfRange {
                index: 5,
                tree_size: 5,
            },
        );
        // A proof whose declared index is out of range fails verify cleanly.
        let bad = InclusionProof::from_parts(TreeSize(5), Index(9), Vec::new());
        assert_eq!(
            bad.verify::<Sha256Hasher>(&leaf_hash_of(0), &tree.root())
                .unwrap_err(),
            ProofError::IndexOutOfRange {
                index: 9,
                tree_size: 5,
            },
        );
    }

    #[test]
    fn every_leaf_of_small_trees_verifies() {
        // Spec section 19.2: for any tree size N and any leaf 0 <= i < N, the
        // generated inclusion proof verifies. Exhaustive up to 33 (crosses the
        // 32 subtree boundary).
        for n in 1..=33u64 {
            let tree = tree_of(n);
            let root = tree.root();
            for i in 0..n {
                let proof = InclusionProof::generate(&tree, Index(i)).unwrap();
                assert_eq!(proof.audit_path().len(), inclusion_path_len(i, n));
                proof
                    .verify::<Sha256Hasher>(&leaf_hash_of(i), &root)
                    .unwrap_or_else(|e| panic!("n={n} i={i} failed to verify: {e}"));
            }
        }
    }

    #[test]
    fn last_index_and_boundary_indices_verify() {
        for n in [2u64, 4, 7, 8, 9, 16, 31, 32] {
            let tree = tree_of(n);
            let root = tree.root();
            for &i in &[0, n / 2, n - 1] {
                let proof = InclusionProof::generate(&tree, Index(i)).unwrap();
                proof
                    .verify::<Sha256Hasher>(&leaf_hash_of(i), &root)
                    .unwrap();
            }
        }
    }

    #[test]
    fn wrong_leaf_hash_fails() {
        let tree = tree_of(8);
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();
        assert_eq!(
            proof
                .verify::<Sha256Hasher>(&leaf_hash_of(4), &tree.root())
                .unwrap_err(),
            ProofError::RootMismatch,
        );
    }

    #[test]
    fn tampered_sibling_fails() {
        let tree = tree_of(8);
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();
        let mut path = proof.audit_path().to_vec();
        path[0] = HashOutput([0xff; 32]);
        let tampered = InclusionProof::from_parts(proof.tree_size(), proof.leaf_index(), path);
        assert_eq!(
            tampered
                .verify::<Sha256Hasher>(&leaf_hash_of(3), &tree.root())
                .unwrap_err(),
            ProofError::RootMismatch,
        );
    }

    #[test]
    fn truncated_and_overlong_paths_are_rejected_before_hashing() {
        let tree = tree_of(8);
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();
        let full = proof.audit_path().len();

        let mut short = proof.audit_path().to_vec();
        short.pop();
        let short_proof = InclusionProof::from_parts(proof.tree_size(), proof.leaf_index(), short);
        assert_eq!(
            short_proof
                .verify::<Sha256Hasher>(&leaf_hash_of(3), &tree.root())
                .unwrap_err(),
            ProofError::MalformedPath {
                expected: full,
                actual: full - 1,
            },
        );

        let mut long = proof.audit_path().to_vec();
        long.push(HashOutput([0x00; 32]));
        let long_proof = InclusionProof::from_parts(proof.tree_size(), proof.leaf_index(), long);
        assert_eq!(
            long_proof
                .verify::<Sha256Hasher>(&leaf_hash_of(3), &tree.root())
                .unwrap_err(),
            ProofError::MalformedPath {
                expected: full,
                actual: full + 1,
            },
        );
    }

    #[test]
    fn wrong_size_proof_fails_cleanly() {
        // A proof generated for size 8 but presented against size > tree fails
        // the length check, not a panic.
        let tree = tree_of(8);
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();
        let mislabeled = InclusionProof::from_parts(
            TreeSize(9),
            proof.leaf_index(),
            proof.audit_path().to_vec(),
        );
        // Deterministic: inclusion_path_len(3, 8) == 3 but inclusion_path_len(3,
        // 9) == 4, so the 3-element path is rejected on the length check with the
        // exact expected/actual before any hashing.
        assert_eq!(
            mislabeled
                .verify::<Sha256Hasher>(&leaf_hash_of(3), &tree.root())
                .unwrap_err(),
            ProofError::MalformedPath {
                expected: 4,
                actual: 3,
            },
        );
    }

    #[test]
    fn wire_round_trip_known_shape() {
        let tree = tree_of(13);
        let proof = InclusionProof::generate(&tree, Index(5)).unwrap();
        let path_len = proof.audit_path().len();
        let bytes = assert_roundtrip!(proof);
        // 8 (tree_size) + 8 (leaf_index) + 2 (u16 vector length) + 32 * path.
        assert_eq!(bytes.len(), 8 + 8 + 2 + 32 * path_len);
    }

    proptest! {
        // Spec section 19.2: any leaf of any tree size verifies, and the proof
        // wire form round-trips.
        #[test]
        fn generated_proof_verifies_and_round_trips(n in 1u64..200, seed in 0u64..u64::MAX) {
            let tree = tree_of(n);
            let index = seed % n;
            let proof = InclusionProof::generate(&tree, Index(index)).unwrap();
            prop_assert!(proof
                .verify::<Sha256Hasher>(&leaf_hash_of(index), &tree.root())
                .is_ok());

            let bytes = proof.tls_serialize_to_vec().unwrap();
            let parsed = InclusionProof::tls_parse_exact(&bytes).unwrap();
            prop_assert_eq!(&parsed, &proof);
            prop_assert!(parsed
                .verify::<Sha256Hasher>(&leaf_hash_of(index), &tree.root())
                .is_ok());
        }

        // Spec section 19.2: appending preserves previously-valid inclusion
        // proofs — a proof made at size n still verifies against the root at
        // size n even after more leaves are appended (the historical root is
        // recomputed via subtree_hash).
        #[test]
        fn append_preserves_old_proofs(n in 1u64..160, extra in 0u64..48, seed in 0u64..u64::MAX) {
            let mut tree = tree_of(n);
            let index = seed % n;
            let proof = InclusionProof::generate(&tree, Index(index)).unwrap();
            let root_at_n = tree.root();
            for i in n..(n + extra) {
                tree.append(format!("entry-{i}").as_bytes());
            }
            // The proof was made against size n; it still verifies against the
            // size-n root, which is the subtree hash over [0, n) of the grown
            // tree.
            prop_assert_eq!(tree.subtree_hash(Index(0), Index(n)).unwrap(), root_at_n);
            prop_assert!(proof
                .verify::<Sha256Hasher>(&leaf_hash_of(index), &root_at_n)
                .is_ok());
        }

        // Tamper: flipping any single sibling makes the proof fail.
        #[test]
        fn any_tampered_sibling_is_rejected(n in 2u64..200, seed in 0u64..u64::MAX) {
            let tree = tree_of(n);
            let index = seed % n;
            let proof = InclusionProof::generate(&tree, Index(index)).unwrap();
            prop_assume!(!proof.audit_path().is_empty());
            let pos = usize::try_from(seed).unwrap_or(0) % proof.audit_path().len();
            let mut path = proof.audit_path().to_vec();
            // Flip one byte so the sibling is guaranteed different.
            path[pos].0[0] ^= 0x01;
            let tampered = InclusionProof::from_parts(proof.tree_size(), proof.leaf_index(), path);
            prop_assert_eq!(
                tampered.verify::<Sha256Hasher>(&leaf_hash_of(index), &tree.root()),
                Err(ProofError::RootMismatch),
            );
        }
    }
}
