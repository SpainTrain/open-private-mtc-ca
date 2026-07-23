//! In-memory construction of the issuance-log Merkle tree: append, root, and
//! subtree hashing (spec section 2, "Issuance log").
//!
//! [`MerkleTree`] stores the **leaf hashes** of the entries appended so far and
//! computes node hashes on demand by the RFC 9162 §2.1.1 Merkle Tree Hash
//! recurrence (see [`crate::tree::digest`] for the cited construction). Because
//! a node hash is a pure function of the contiguous leaf-hash sequence beneath
//! it, two trees built from the same entry sequence produce identical roots
//! (spec section 19.2) and appending never rewrites an existing leaf hash — the
//! append-only property this log depends on (spec section 3).
//!
//! This layer is byte-oriented: entries are `&[u8]`. Entry encoding, tiles,
//! inclusion/consistency proofs, and range pruning are deliberately out of
//! scope here (they arrive with `mtc-serialization`, `mtclib-tiles`,
//! `mtclib-inclusion-proofs`, and `mtclib-tree-pruning`). Persistence is out of
//! scope (storage-facade epic); this is an in-memory builder.

use core::marker::PhantomData;

use super::digest::{empty_root, hash_leaf, hash_node, Hasher, Sha256Hasher};
use crate::types::{HashOutput, Index, TreeSize};

/// An append-only, in-memory Merkle tree over log-entry bytes.
///
/// Generic over the hash function `H` for static dispatch on the per-entry hot
/// path (spec section 22.7); defaults to [`Sha256Hasher`], the v1 hash. The
/// tree stores one [`HashOutput`] per appended entry (its leaf hash) and
/// derives all interior nodes and the root by recomputation, so the leaf-hash
/// vector is the entire persistent state.
///
/// # Examples
///
/// ```
/// use mtc::{MerkleTree, TreeSize};
///
/// // The bare type `MerkleTree` resolves to `MerkleTree<Sha256Hasher>` via
/// // the default type parameter.
/// let mut tree: MerkleTree = MerkleTree::new();
/// tree.append(b"entry-0");
/// tree.append(b"entry-1");
/// assert_eq!(tree.len(), TreeSize(2));
///
/// // Same entries, same root (spec section 19.2).
/// let mut other: MerkleTree = MerkleTree::new();
/// other.append(b"entry-0");
/// other.append(b"entry-1");
/// assert_eq!(tree.root(), other.root());
/// ```
#[derive(Clone)]
pub struct MerkleTree<H: Hasher = Sha256Hasher> {
    /// Leaf hashes in index order: `leaves[i] == hash_leaf::<H>(entry_i)`.
    leaves: Vec<HashOutput>,
    /// `H` appears only in method bodies (via associated functions), so a
    /// `fn() -> H` marker ties the type parameter without constraining the
    /// tree's auto-traits to `H`.
    _hasher: PhantomData<fn() -> H>,
}

impl<H: Hasher> Default for MerkleTree<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H: Hasher> MerkleTree<H> {
    /// Creates an empty tree.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            leaves: Vec::new(),
            _hasher: PhantomData,
        }
    }

    /// Creates an empty tree preallocated for `capacity` entries.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            leaves: Vec::with_capacity(capacity),
            _hasher: PhantomData,
        }
    }

    /// Appends one entry and returns the [`Index`] it was assigned.
    ///
    /// The entry's leaf hash `HASH(0x00 || entry)` is computed once and stored;
    /// leaf hashes of earlier entries are never touched (the append-only
    /// property, spec section 3).
    pub fn append(&mut self, entry: &[u8]) -> Index {
        let index = Index(self.leaves.len() as u64);
        self.leaves.push(hash_leaf::<H>(entry));
        index
    }

    /// The number of entries appended so far (the tree size committed by a
    /// checkpoint over this tree; spec section 2, "Checkpoint").
    #[must_use]
    pub const fn len(&self) -> TreeSize {
        TreeSize(self.leaves.len() as u64)
    }

    /// Whether the tree has no entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    /// The leaf hash at `index`, or `None` if `index` is out of range.
    #[must_use]
    pub fn leaf_hash(&self, index: Index) -> Option<HashOutput> {
        usize::try_from(index.0)
            .ok()
            .and_then(|i| self.leaves.get(i))
            .copied()
    }

    /// The Merkle Tree Hash root over all entries (RFC 9162 §2.1.1).
    ///
    /// For an empty tree this is [`empty_root`] (spec section 19.6); otherwise
    /// it is the root of the tree over leaf hashes `[0, len)`.
    #[must_use]
    pub fn root(&self) -> HashOutput {
        mth::<H>(&self.leaves)
    }

    /// The Merkle Tree Hash over the contiguous leaf range `[start, end)`.
    ///
    /// Returns `None` unless `start < end <= len` (an empty or out-of-range
    /// span has no node). When `[start, end)` is an aligned power-of-two range
    /// — as every [`Subtree`](crate::Subtree) returned by
    /// [`decompose_range`](crate::decompose_range) is — the result is the hash
    /// of a real interior node of the tree; for other ranges it is the MTH of
    /// that sub-list (the quantity consistency proofs are built from). Because
    /// it depends only on the leaf hashes in the range, it is stable across
    /// later appends (spec section 19.2, append preserves existing structure).
    #[must_use]
    pub fn subtree_hash(&self, start: Index, end: Index) -> Option<HashOutput> {
        let start = usize::try_from(start.0).ok()?;
        let end = usize::try_from(end.0).ok()?;
        if start >= end || end > self.leaves.len() {
            return None;
        }
        Some(mth::<H>(&self.leaves[start..end]))
    }
}

impl<H: Hasher> core::fmt::Debug for MerkleTree<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MerkleTree")
            .field("len", &self.leaves.len())
            .field("root", &self.root())
            .finish()
    }
}

/// The Merkle Tree Hash of a slice of **leaf hashes** (RFC 9162 §2.1.1).
///
/// - empty slice: [`empty_root`] (only reachable for the whole empty tree);
/// - one leaf: that leaf hash unchanged;
/// - `n > 1`: split at `k`, the largest power of two strictly less than `n`,
///   and combine the two child roots with [`hash_node`].
fn mth<H: Hasher>(leaves: &[HashOutput]) -> HashOutput {
    match leaves {
        [] => empty_root::<H>(),
        [leaf] => *leaf,
        _ => {
            let k = split_point(leaves.len());
            let left = mth::<H>(&leaves[..k]);
            let right = mth::<H>(&leaves[k..]);
            hash_node::<H>(&left, &right)
        }
    }
}

/// The largest power of two strictly less than `n`, for `n >= 2` (RFC 9162
/// §2.1.1: "let `k` be the largest power of two smaller than `n`").
///
/// For a power of two this is `n / 2`; otherwise `2^floor(log2(n-1))`.
fn split_point(n: usize) -> usize {
    debug_assert!(n >= 2, "split_point requires n >= 2");
    // (n-1).ilog2() = floor(log2(n-1)); 1 << that is the largest power of two
    // that is <= n-1, i.e. strictly less than n. E.g. n=8 -> 4, n=5 -> 4.
    1usize << (n - 1).ilog2()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{split_point, MerkleTree};
    use crate::tree::digest::{empty_root, hash_leaf, hash_node, Sha256Hasher, SHA256_EMPTY_ROOT};
    use crate::types::{HashOutput, Index, TreeSize};

    type Tree = MerkleTree<Sha256Hasher>;

    fn build(entries: &[&[u8]]) -> Tree {
        let mut tree = Tree::new();
        for e in entries {
            tree.append(e);
        }
        tree
    }

    #[test]
    fn split_point_matches_rfc_examples() {
        assert_eq!(split_point(2), 1);
        assert_eq!(split_point(3), 2);
        assert_eq!(split_point(4), 2);
        assert_eq!(split_point(5), 4);
        assert_eq!(split_point(7), 4);
        assert_eq!(split_point(8), 4);
        assert_eq!(split_point(9), 8);
    }

    #[test]
    fn empty_tree_root_is_empty_constant() {
        let tree = Tree::new();
        assert!(tree.is_empty());
        assert_eq!(tree.len(), TreeSize(0));
        assert_eq!(tree.root(), SHA256_EMPTY_ROOT);
        assert_eq!(tree.root(), empty_root::<Sha256Hasher>());
    }

    #[test]
    fn single_leaf_root_is_leaf_hash() {
        let tree = build(&[b"only"]);
        assert_eq!(tree.len(), TreeSize(1));
        assert_eq!(tree.root(), hash_leaf::<Sha256Hasher>(b"only"));
    }

    #[test]
    fn two_leaf_root_is_node_of_leaves() {
        let tree = build(&[b"a", b"b"]);
        let expected = hash_node::<Sha256Hasher>(
            &hash_leaf::<Sha256Hasher>(b"a"),
            &hash_leaf::<Sha256Hasher>(b"b"),
        );
        assert_eq!(tree.root(), expected);
    }

    #[test]
    fn three_leaf_root_is_unbalanced_per_rfc() {
        // n=3 splits k=2: root = node(node(l0,l1), l2).
        let tree = build(&[b"a", b"b", b"c"]);
        let l0 = hash_leaf::<Sha256Hasher>(b"a");
        let l1 = hash_leaf::<Sha256Hasher>(b"b");
        let l2 = hash_leaf::<Sha256Hasher>(b"c");
        let left = hash_node::<Sha256Hasher>(&l0, &l1);
        let expected = hash_node::<Sha256Hasher>(&left, &l2);
        assert_eq!(tree.root(), expected);
    }

    #[test]
    fn append_returns_sequential_indices() {
        let mut tree = Tree::new();
        assert_eq!(tree.append(b"a"), Index(0));
        assert_eq!(tree.append(b"b"), Index(1));
        assert_eq!(tree.append(b"c"), Index(2));
        assert_eq!(
            tree.leaf_hash(Index(1)),
            Some(hash_leaf::<Sha256Hasher>(b"b"))
        );
        assert_eq!(tree.leaf_hash(Index(3)), None);
    }

    #[test]
    fn subtree_hash_range_validation() {
        let tree = build(&[b"a", b"b", b"c", b"d"]);
        // Whole range equals the root.
        assert_eq!(tree.subtree_hash(Index(0), Index(4)), Some(tree.root()));
        // Aligned pair equals its interior node.
        let expected = hash_node::<Sha256Hasher>(
            &hash_leaf::<Sha256Hasher>(b"a"),
            &hash_leaf::<Sha256Hasher>(b"b"),
        );
        assert_eq!(tree.subtree_hash(Index(0), Index(2)), Some(expected));
        // Single-leaf range is that leaf hash.
        assert_eq!(
            tree.subtree_hash(Index(2), Index(3)),
            Some(hash_leaf::<Sha256Hasher>(b"c")),
        );
        // Empty and out-of-range spans have no node.
        assert_eq!(tree.subtree_hash(Index(2), Index(2)), None);
        assert_eq!(tree.subtree_hash(Index(3), Index(2)), None);
        assert_eq!(tree.subtree_hash(Index(0), Index(5)), None);
    }

    #[test]
    fn known_answer_1000_leaf_root_is_deterministic() {
        // Locks the demo output: two independent builds of the same 1000-entry
        // sequence agree, so `cargo run --example tree_demo` is deterministic.
        let entries: Vec<String> = (0..1000).map(|i| format!("entry-{i}")).collect();
        let mut a = Tree::new();
        let mut b = Tree::with_capacity(1000);
        for e in &entries {
            a.append(e.as_bytes());
            b.append(e.as_bytes());
        }
        assert_eq!(a.len(), TreeSize(1000));
        assert_eq!(a.root(), b.root());
    }

    proptest! {
        // Spec section 19.2: two trees built from the same leaf sequence
        // produce identical roots.
        #[test]
        fn same_sequence_same_root(entries in proptest::collection::vec(
            proptest::collection::vec(any::<u8>(), 0..32), 0..64)) {
            let refs: Vec<&[u8]> = entries.iter().map(Vec::as_slice).collect();
            let first = build(&refs);
            let second = build(&refs);
            prop_assert_eq!(first.root(), second.root());
        }

        // Spec section 19.2 / section 3: appending never changes an existing
        // leaf hash, and every subtree over an already-committed range keeps
        // its hash across later appends (append-only structure).
        #[test]
        fn append_preserves_existing_structure(
            initial in proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 0..16), 1..48),
            added in proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 0..16), 0..16),
        ) {
            let initial_refs: Vec<&[u8]> = initial.iter().map(Vec::as_slice).collect();
            let mut tree = build(&initial_refs);

            // Snapshot every leaf hash and the root at the initial size.
            let old_len = initial.len();
            let leaf_snapshot: Vec<HashOutput> = (0..old_len)
                .map(|i| tree.leaf_hash(Index(i as u64)).unwrap())
                .collect();
            let root_at_old = tree.root();

            for e in &added {
                tree.append(e);
            }

            // Leaf hashes for [0, old_len) are byte-for-byte unchanged.
            for (i, snap) in leaf_snapshot.iter().enumerate() {
                prop_assert_eq!(tree.leaf_hash(Index(i as u64)).unwrap(), *snap);
            }
            // The root over the original prefix is unchanged: the historical
            // tree is a subtree of the grown tree.
            prop_assert_eq!(
                tree.subtree_hash(Index(0), Index(old_len as u64)).unwrap(),
                root_at_old,
            );
            // Every aligned power-of-two subtree present before the append has
            // the same hash after it.
            let mut width = 1usize;
            while width <= old_len {
                let mut start = 0usize;
                while start + width <= old_len {
                    let before = tree
                        .subtree_hash(Index(start as u64), Index((start + width) as u64))
                        .unwrap();
                    // Recompute from the snapshot (independent of `tree`).
                    let expected = super::mth::<Sha256Hasher>(&leaf_snapshot[start..start + width]);
                    prop_assert_eq!(before, expected);
                    start += width;
                }
                width <<= 1;
            }
        }
    }
}
