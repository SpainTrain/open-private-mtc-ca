//! Consistency proofs: proof that one checkpoint's tree is an append-only
//! prefix of a later one (RFC 9162 §2.1.4, adopted by
//! `draft-ietf-plants-merkle-tree-certs-03`; spec section 19.2).

use std::io::{self, Write};

use super::{largest_power_of_two_below, ProofError};
use crate::tree::{empty_root, hash_node, Hasher, MerkleTree};
use crate::types::{HashOutput, Index, TreeSize};
use crate::wire::{write_bytes, write_vector_u16, TlsParse, TlsReader, TlsSerialize, WireError};

/// A Merkle **consistency proof** between an old and a new tree size of the same
/// append-only log (spec section 19.2, "Consistency proofs verify between any
/// two tree sizes").
///
/// It carries both sizes so [`verify`](Self::verify) can reject a non-monotonic
/// pair and validate the path length on its own. The two boundary shapes carry
/// an **empty** path: `old_size == 0` (the empty tree is a prefix of every tree)
/// and `old_size == new_size` (the trees are identical).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConsistencyProof {
    old_size: TreeSize,
    new_size: TreeSize,
    path: Vec<HashOutput>,
}

impl ConsistencyProof {
    /// Generates the consistency proof between `old_size` and `new_size` from
    /// `tree` (RFC 9162 §2.1.4.1). `new_size` must be `<= tree.len()`.
    ///
    /// # Errors
    ///
    /// - [`ProofError::NonMonotonicSizes`] if `old_size > new_size`.
    /// - [`ProofError::TreeTooSmall`] if `new_size > tree.len()`.
    pub fn generate<H: Hasher>(
        tree: &MerkleTree<H>,
        old_size: TreeSize,
        new_size: TreeSize,
    ) -> Result<Self, ProofError> {
        let (m, n) = (old_size.0, new_size.0);
        if m > n {
            return Err(ProofError::NonMonotonicSizes {
                old_size: m,
                new_size: n,
            });
        }
        let available = tree.len().0;
        if n > available {
            return Err(ProofError::TreeTooSmall {
                requested: n,
                available,
            });
        }
        // Boundary shapes carry an empty path (RFC: PROOF is defined for
        // 0 < m < n; m == 0 and m == n are the trivial prefixes).
        let path = if m == 0 || m == n {
            Vec::new()
        } else {
            let mut out = Vec::new();
            collect_subproof(tree, m, 0, n, true, &mut out)?;
            out
        };
        Ok(Self {
            old_size,
            new_size,
            path,
        })
    }

    /// Constructs a proof from already-known parts (e.g. decoded from the wire).
    /// Performs no validation; call [`verify`](Self::verify) to check it.
    #[must_use]
    pub const fn from_parts(old_size: TreeSize, new_size: TreeSize, path: Vec<HashOutput>) -> Self {
        Self {
            old_size,
            new_size,
            path,
        }
    }

    /// The earlier (old) tree size.
    #[must_use]
    pub const fn old_size(&self) -> TreeSize {
        self.old_size
    }

    /// The later (new) tree size.
    #[must_use]
    pub const fn new_size(&self) -> TreeSize {
        self.new_size
    }

    /// The consistency path (interior node hashes).
    #[must_use]
    pub fn path(&self) -> &[HashOutput] {
        &self.path
    }

    /// Verifies the proof: that a tree of `old_size` with root `old_root` is a
    /// prefix of a tree of `new_size` with root `new_root` (RFC 9162 §2.1.4.2).
    ///
    /// `H` is the log's hash function (SHA-256 for v1). The sizes are validated
    /// (monotonic) and the path length is checked against `(old_size, new_size)`
    /// **before** any hashing.
    ///
    /// # Errors
    ///
    /// - [`ProofError::NonMonotonicSizes`] if `old_size > new_size`.
    /// - [`ProofError::MalformedPath`] if the path is not the length the sizes
    ///   require.
    /// - [`ProofError::RootMismatch`] if the proof does not reconstruct both
    ///   `old_root` and `new_root` (or, for `old_size == 0`, if `old_root` is not
    ///   the empty-tree root).
    pub fn verify<H: Hasher>(
        &self,
        old_root: &HashOutput,
        new_root: &HashOutput,
    ) -> Result<(), ProofError> {
        let (m, n) = (self.old_size.0, self.new_size.0);
        if m > n {
            return Err(ProofError::NonMonotonicSizes {
                old_size: m,
                new_size: n,
            });
        }

        // Length validation before any hashing (crypto-review).
        let expected = consistency_path_len(m, n);
        if self.path.len() != expected {
            return Err(ProofError::MalformedPath {
                expected,
                actual: self.path.len(),
            });
        }

        // Boundary: the empty tree is a prefix of everything, and its only valid
        // root is the empty-tree root. Confirm the claimed old root is *the*
        // empty root, so a bogus old_root cannot ride an empty proof. When the
        // NEW tree is also empty (n == 0 — the degenerate (0, 0) pair), its root
        // must ALSO be the empty root; otherwise "same size => same root" goes
        // unenforced at size 0 and a garbage new_root is accepted for an
        // impossible tree state (crypto F1). This check must run before the
        // m == n arm so (0, 0) does not fall through to a bare old==new compare.
        if m == 0 {
            if *old_root != empty_root::<H>() {
                return Err(ProofError::RootMismatch);
            }
            if n == 0 && *new_root != empty_root::<H>() {
                return Err(ProofError::RootMismatch);
            }
            return Ok(());
        }
        // Boundary: identical (non-empty) trees.
        if m == n {
            return if old_root == new_root {
                Ok(())
            } else {
                Err(ProofError::RootMismatch)
            };
        }

        // 0 < m < n: reconstruct both roots from the path (RFC 9162 §2.1.4.2).
        // `node`/`last` are the positions of the old tree's rightmost node and
        // the new tree's rightmost node; shifting past the shared lower bits
        // aligns them. `first` accumulates the old root, `second` the new root.
        let mut node = m - 1;
        let mut last = n - 1;
        while node & 1 == 1 {
            node >>= 1;
            last >>= 1;
        }

        // Seed the recomputation. When `node > 0` the divergence point is an
        // interior node whose hash is the first path element; otherwise the old
        // tree is a full subtree of the new one and we seed from `old_root`.
        let (seed, mut idx) = if node > 0 {
            (*path_at(&self.path, 0, expected)?, 1usize)
        } else {
            (*old_root, 0usize)
        };
        let mut first = seed;
        let mut second = seed;

        while node > 0 {
            if node & 1 == 1 {
                let p = path_at(&self.path, idx, expected)?;
                first = hash_node::<H>(p, &first);
                second = hash_node::<H>(p, &second);
                idx += 1;
            } else if node < last {
                let p = path_at(&self.path, idx, expected)?;
                second = hash_node::<H>(&second, p);
                idx += 1;
            }
            node >>= 1;
            last >>= 1;
        }
        while last > 0 {
            let p = path_at(&self.path, idx, expected)?;
            second = hash_node::<H>(&second, p);
            idx += 1;
            last >>= 1;
        }

        if idx != self.path.len() {
            return Err(ProofError::MalformedPath {
                expected: idx,
                actual: self.path.len(),
            });
        }
        if first == *old_root && second == *new_root {
            Ok(())
        } else {
            Err(ProofError::RootMismatch)
        }
    }
}

/// Bounds-checked path access that turns an out-of-range index — which the
/// length pre-check makes unreachable — into a [`ProofError::MalformedPath`]
/// rather than a panic.
fn path_at(path: &[HashOutput], idx: usize, expected: usize) -> Result<&HashOutput, ProofError> {
    path.get(idx).ok_or(ProofError::MalformedPath {
        expected,
        actual: path.len(),
    })
}

/// The number of hashes a consistency proof between `m` and `n` contains,
/// computed without hashing (mirrors [`collect_subproof`]).
fn consistency_path_len(m: u64, n: u64) -> usize {
    if m == 0 || m == n {
        0
    } else {
        subproof_len(m, n, true)
    }
}

/// Length of `SUBPROOF(m, D[0:n], b)` (RFC 9162 §2.1.4.1), by structural
/// recursion on the same split as [`collect_subproof`].
fn subproof_len(m: u64, n: u64, b: bool) -> usize {
    if m == n {
        return usize::from(!b);
    }
    let k = largest_power_of_two_below(n);
    if m <= k {
        subproof_len(m, k, b) + 1
    } else {
        subproof_len(m - k, n - k, false) + 1
    }
}

/// Appends `SUBPROOF(m, D[start:end], b)` to `out` (RFC 9162 §2.1.4.1).
///
/// `m` is the old tree's size *within* the subtree `[start, end)`; `b` marks
/// whether that subtree is entirely the old tree (so its own hash is derivable
/// from `old_root` and need not be sent). All ranges are within `[0, n)` and
/// non-empty, so [`MerkleTree::subtree_hash`] always returns `Some`; an
/// unreachable `None` becomes [`ProofError::TreeTooSmall`].
fn collect_subproof<H: Hasher>(
    tree: &MerkleTree<H>,
    m: u64,
    start: u64,
    end: u64,
    b: bool,
    out: &mut Vec<HashOutput>,
) -> Result<(), ProofError> {
    let n = end - start;
    if m == n {
        if !b {
            out.push(subtree(tree, start, end)?);
        }
        return Ok(());
    }
    let k = largest_power_of_two_below(n);
    if m <= k {
        collect_subproof(tree, m, start, start + k, b, out)?;
        out.push(subtree(tree, start + k, end)?);
    } else {
        collect_subproof(tree, m - k, start + k, end, false, out)?;
        out.push(subtree(tree, start, start + k)?);
    }
    Ok(())
}

/// The Merkle Tree Hash over `[start, end)`, mapping the unreachable
/// out-of-range case to a typed error instead of unwrapping.
fn subtree<H: Hasher>(
    tree: &MerkleTree<H>,
    start: u64,
    end: u64,
) -> Result<HashOutput, ProofError> {
    tree.subtree_hash(Index(start), Index(end))
        .ok_or(ProofError::TreeTooSmall {
            requested: end,
            available: tree.len().0,
        })
}

impl TlsSerialize for ConsistencyProof {
    /// Wire form (clean-room, TLS presentation language, RFC 9162 §2.1.4 shape):
    ///
    /// ```text
    /// struct {
    ///     uint64 old_size;
    ///     uint64 new_size;
    ///     NodeHash consistency_path<0..2^16-1>;  // NodeHash = opaque[32]
    /// } ConsistencyProof;
    /// ```
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_bytes(writer, &self.old_size.0.to_be_bytes())?;
        write_bytes(writer, &self.new_size.0.to_be_bytes())?;
        let arrays: Vec<[u8; 32]> = self.path.iter().map(|h| h.0).collect();
        write_vector_u16(writer, &arrays)
    }
}

impl TlsParse for ConsistencyProof {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        let old_size = u64::from_be_bytes(reader.read_array::<8>()?);
        let new_size = u64::from_be_bytes(reader.read_array::<8>()?);
        // Empty paths are valid (boundary cases), so no wire minimum is imposed;
        // the semantic length is enforced in `verify`.
        let arrays: Vec<[u8; 32]> = reader.read_vector_u16::<[u8; 32]>()?;
        let path = arrays.into_iter().map(HashOutput).collect();
        Ok(Self {
            old_size: TreeSize(old_size),
            new_size: TreeSize(new_size),
            path,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{consistency_path_len, ConsistencyProof};
    use crate::leaf::LeafBytes;
    use crate::tree::Sha256Hasher;
    use crate::types::{HashOutput, TreeSize};
    use crate::wire::{TlsParse, TlsSerialize};
    use crate::{assert_roundtrip, MerkleTree, ProofError};

    type Tree = MerkleTree<Sha256Hasher>;

    fn tree_of(n: u64) -> Tree {
        let mut tree = Tree::new();
        for i in 0..n {
            tree.append(&LeafBytes::from_framed(format!("entry-{i}").into_bytes()));
        }
        tree
    }

    /// The root of the log at size `k` (a prefix of the size-`n` tree).
    fn root_at(tree: &Tree, k: u64) -> HashOutput {
        if k == 0 {
            crate::empty_root::<Sha256Hasher>()
        } else {
            tree.subtree_hash(crate::Index(0), crate::Index(k)).unwrap()
        }
    }

    #[test]
    fn non_monotonic_sizes_are_rejected() {
        let tree = tree_of(8);
        assert_eq!(
            ConsistencyProof::generate(&tree, TreeSize(6), TreeSize(3)).unwrap_err(),
            ProofError::NonMonotonicSizes {
                old_size: 6,
                new_size: 3,
            },
        );
        let bad = ConsistencyProof::from_parts(TreeSize(6), TreeSize(3), Vec::new());
        assert_eq!(
            bad.verify::<Sha256Hasher>(&root_at(&tree, 6), &root_at(&tree, 3))
                .unwrap_err(),
            ProofError::NonMonotonicSizes {
                old_size: 6,
                new_size: 3,
            },
        );
    }

    #[test]
    fn new_size_beyond_tree_is_rejected() {
        let tree = tree_of(4);
        assert_eq!(
            ConsistencyProof::generate(&tree, TreeSize(2), TreeSize(9)).unwrap_err(),
            ProofError::TreeTooSmall {
                requested: 9,
                available: 4,
            },
        );
    }

    #[test]
    fn empty_old_tree_is_prefix_of_everything() {
        let tree = tree_of(7);
        let proof = ConsistencyProof::generate(&tree, TreeSize(0), TreeSize(7)).unwrap();
        assert!(proof.path().is_empty());
        proof
            .verify::<Sha256Hasher>(&root_at(&tree, 0), &root_at(&tree, 7))
            .unwrap();
        // A non-empty "old root" over an empty proof is rejected.
        assert_eq!(
            proof
                .verify::<Sha256Hasher>(&HashOutput([0x00; 32]), &root_at(&tree, 7))
                .unwrap_err(),
            ProofError::RootMismatch,
        );
    }

    #[test]
    fn degenerate_zero_zero_requires_empty_new_root() {
        // crypto F1: the (0, 0) pair must not slip through the m == 0 arm and
        // accept an arbitrary new_root. At size 0 the only valid root is the
        // empty-tree root, so "same size => same root" must hold at n == 0 too.
        let empty = crate::empty_root::<Sha256Hasher>();
        let proof = ConsistencyProof::from_parts(TreeSize(0), TreeSize(0), Vec::new());
        // Empty old root but a garbage new root: rejected (the regression).
        assert_eq!(
            proof
                .verify::<Sha256Hasher>(&empty, &HashOutput([0xde; 32]))
                .unwrap_err(),
            ProofError::RootMismatch,
        );
        // Both roots empty: consistent.
        proof.verify::<Sha256Hasher>(&empty, &empty).unwrap();
        // A non-empty old root at (0, 0) is likewise rejected.
        assert_eq!(
            proof
                .verify::<Sha256Hasher>(&HashOutput([0x11; 32]), &empty)
                .unwrap_err(),
            ProofError::RootMismatch,
        );
    }

    #[test]
    fn identical_sizes_need_matching_roots() {
        let tree = tree_of(5);
        let proof = ConsistencyProof::generate(&tree, TreeSize(5), TreeSize(5)).unwrap();
        assert!(proof.path().is_empty());
        proof
            .verify::<Sha256Hasher>(&root_at(&tree, 5), &root_at(&tree, 5))
            .unwrap();
        assert_eq!(
            proof
                .verify::<Sha256Hasher>(&root_at(&tree, 5), &HashOutput([0x11; 32]))
                .unwrap_err(),
            ProofError::RootMismatch,
        );
    }

    #[test]
    fn worked_example_three_to_seven() {
        // A concrete mid-tree case exercised end to end.
        let tree = tree_of(7);
        let proof = ConsistencyProof::generate(&tree, TreeSize(3), TreeSize(7)).unwrap();
        assert_eq!(proof.path().len(), consistency_path_len(3, 7));
        proof
            .verify::<Sha256Hasher>(&root_at(&tree, 3), &root_at(&tree, 7))
            .unwrap();
    }

    #[test]
    fn all_size_pairs_of_small_trees_verify() {
        // Spec section 19.2: consistency proofs verify between any two sizes.
        // Exhaustive for every 0 <= old <= new <= 33.
        for n in 0..=33u64 {
            let tree = tree_of(n);
            for old in 0..=n {
                let proof = ConsistencyProof::generate(&tree, TreeSize(old), TreeSize(n)).unwrap();
                assert_eq!(proof.path().len(), consistency_path_len(old, n));
                proof
                    .verify::<Sha256Hasher>(&root_at(&tree, old), &root_at(&tree, n))
                    .unwrap_or_else(|e| panic!("old={old} new={n} failed: {e}"));
            }
        }
    }

    #[test]
    fn tampered_and_wrong_length_paths_fail() {
        let tree = tree_of(9);
        let proof = ConsistencyProof::generate(&tree, TreeSize(4), TreeSize(9)).unwrap();
        let full = proof.path().len();

        // Tamper one node.
        let mut path = proof.path().to_vec();
        path[0].0[0] ^= 0x01;
        let tampered = ConsistencyProof::from_parts(TreeSize(4), TreeSize(9), path);
        assert_eq!(
            tampered
                .verify::<Sha256Hasher>(&root_at(&tree, 4), &root_at(&tree, 9))
                .unwrap_err(),
            ProofError::RootMismatch,
        );

        // Truncate.
        let mut short = proof.path().to_vec();
        short.pop();
        let short_proof = ConsistencyProof::from_parts(TreeSize(4), TreeSize(9), short);
        assert_eq!(
            short_proof
                .verify::<Sha256Hasher>(&root_at(&tree, 4), &root_at(&tree, 9))
                .unwrap_err(),
            ProofError::MalformedPath {
                expected: full,
                actual: full - 1,
            },
        );
    }

    #[test]
    fn wire_round_trips() {
        let tree = tree_of(20);
        let proof = ConsistencyProof::generate(&tree, TreeSize(6), TreeSize(20)).unwrap();
        let path_len = proof.path().len();
        let bytes = assert_roundtrip!(proof);
        assert_eq!(bytes.len(), 8 + 8 + 2 + 32 * path_len);
    }

    proptest! {
        // Spec section 19.2: consistency verifies between any two sizes, and the
        // wire form round-trips.
        #[test]
        fn any_size_pair_verifies_and_round_trips(a in 0u64..200, b in 0u64..200) {
            let (old, new) = (a.min(b), a.max(b));
            let tree = tree_of(new);
            let proof = ConsistencyProof::generate(&tree, TreeSize(old), TreeSize(new)).unwrap();
            prop_assert!(proof
                .verify::<Sha256Hasher>(&root_at(&tree, old), &root_at(&tree, new))
                .is_ok());

            let bytes = proof.tls_serialize_to_vec().unwrap();
            let parsed = ConsistencyProof::tls_parse_exact(&bytes).unwrap();
            prop_assert_eq!(&parsed, &proof);
            prop_assert!(parsed
                .verify::<Sha256Hasher>(&root_at(&tree, old), &root_at(&tree, new))
                .is_ok());
        }

        // A consistency proof must not verify against a wrong new root.
        #[test]
        fn wrong_new_root_is_rejected(a in 1u64..200, b in 1u64..200) {
            let (old, new) = (a.min(b), a.max(b));
            prop_assume!(old < new && old > 0);
            let tree = tree_of(new);
            let proof = ConsistencyProof::generate(&tree, TreeSize(old), TreeSize(new)).unwrap();
            let wrong_new = HashOutput([0x5a; 32]);
            prop_assert_eq!(
                proof.verify::<Sha256Hasher>(&root_at(&tree, old), &wrong_new),
                Err(ProofError::RootMismatch),
            );
        }
    }
}
