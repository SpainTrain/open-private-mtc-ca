//! Decomposition of an arbitrary entry range `[start, end)` into the canonical
//! set of aligned power-of-two subtrees (spec section 2, "Subtree").
//!
//! A range of consecutive entries is rarely itself a complete subtree of the
//! Merkle tree. It is, however, always the disjoint union of a small number of
//! *aligned power-of-two* blocks — ranges `[i, i + 2^l)` with `i` a multiple of
//! `2^l` — each of which **is** a complete subtree with a single node hash.
//! This is the decomposition inclusion/consistency proofs and `tlog-tiles`
//! serving are built on (spec section 28). [`decompose_range`] returns the
//! blocks left to right; feeding each to
//! [`MerkleTree::subtree_hash`](crate::MerkleTree::subtree_hash) yields the
//! node hashes that cover the range.
//!
//! Computing inclusion/consistency proofs themselves, and the tile mapping, are
//! out of scope here (`mtclib-inclusion-proofs`, `mtclib-tiles`).

use crate::types::Index;

/// One aligned power-of-two block of the tree: the half-open entry range
/// `[start, end)`.
///
/// Every `Subtree` returned by [`decompose_range`] is **non-empty**
/// (`start < end`), has a power-of-two length, and is **aligned**
/// (`start` is a multiple of its length) — so it corresponds to exactly one
/// complete subtree, hence one interior (or leaf) node, of the Merkle tree.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Subtree {
    start: Index,
    end: Index,
}

impl Subtree {
    /// Constructs the half-open range `[start, end)`.
    ///
    /// This is a plain range value; callers that need the alignment invariant
    /// should obtain `Subtree`s from [`decompose_range`], which only ever emits
    /// non-empty aligned power-of-two blocks.
    #[must_use]
    pub const fn new(start: Index, end: Index) -> Self {
        Self { start, end }
    }

    /// The inclusive lower bound of the range.
    #[must_use]
    pub const fn start(self) -> Index {
        self.start
    }

    /// The exclusive upper bound of the range.
    #[must_use]
    pub const fn end(self) -> Index {
        self.end
    }

    /// The number of entries the range covers (`end - start`).
    #[must_use]
    pub const fn len(self) -> u64 {
        self.end.0 - self.start.0
    }

    /// Whether the range covers no entries (`start == end`). Never true for a
    /// block returned by [`decompose_range`].
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }
}

/// Decomposes `[start, end)` into the canonical, ordered set of aligned
/// power-of-two subtrees.
///
/// The returned blocks are pairwise disjoint, cover exactly `[start, end)` with
/// no gaps, are each non-empty and aligned power-of-two, and appear in
/// ascending order. An empty or inverted range (`start >= end`) yields an empty
/// vector.
///
/// The count is at most `2 * bit_length(end - start)`, hence at most
/// `2 * bit_length(tree_size)` for any range within a tree of size `tree_size`
/// (spec acceptance criterion; `bit_length(n) = floor(log2 n) + 1`). The greedy
/// rule — at each position take the largest aligned block that still fits —
/// produces this canonical decomposition: block sizes strictly increase while
/// alignment is the binding constraint, then strictly decrease as the remaining
/// span is, and each phase visits distinct powers of two, bounding it by
/// `bit_length(end - start)`.
#[must_use]
pub fn decompose_range(start: Index, end: Index) -> Vec<Subtree> {
    let (mut i, end) = (start.0, end.0);
    let mut blocks = Vec::new();
    while i < end {
        // Largest block permitted by `i`'s alignment. At i == 0 alignment is
        // unbounded, so only the remaining span limits the block.
        let by_alignment = if i == 0 {
            u64::MAX
        } else {
            1u64 << i.trailing_zeros()
        };
        // Largest power of two not exceeding the remaining span.
        let by_span = largest_pow2_le(end - i);
        let step = by_alignment.min(by_span);
        blocks.push(Subtree::new(Index(i), Index(i + step)));
        i += step;
    }
    blocks
}

/// The largest power of two `<= x`, for `x >= 1`.
fn largest_pow2_le(x: u64) -> u64 {
    debug_assert!(x >= 1, "largest_pow2_le requires x >= 1");
    1u64 << x.ilog2()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{decompose_range, Subtree};
    use crate::types::Index;

    /// Number of significant bits in `n` (`0` for `n == 0`, else
    /// `floor(log2 n) + 1`). Used to state the decomposition size bound.
    fn bit_length(n: u64) -> u32 {
        u64::BITS - n.leading_zeros()
    }

    /// The proven upper bound on the number of blocks [`decompose_range`]
    /// returns for a range of length `len`: `2 * bit_length(len)` (and `0` for
    /// `len == 0`).
    fn max_blocks(len: u64) -> u64 {
        2 * u64::from(bit_length(len))
    }

    /// Asserts the full contract for `decompose_range(start, end)` within a
    /// tree of size `tree_size`, and returns the block count.
    fn assert_decomposition(start: u64, end: u64, tree_size: u64) -> usize {
        let blocks = decompose_range(Index(start), Index(end));

        if start >= end {
            assert!(blocks.is_empty(), "empty range must yield no blocks");
            return 0;
        }

        // Cover exactly [start, end) with no gaps or overlaps, in order.
        assert_eq!(blocks.first().unwrap().start(), Index(start));
        assert_eq!(blocks.last().unwrap().end(), Index(end));
        for pair in blocks.windows(2) {
            assert_eq!(
                pair[0].end(),
                pair[1].start(),
                "gap or overlap between blocks"
            );
        }

        for b in &blocks {
            let len = b.len();
            assert!(len >= 1, "blocks are non-empty");
            assert!(
                len.is_power_of_two(),
                "block length is a power of two: {len}"
            );
            assert_eq!(b.start().0 % len, 0, "block is aligned to its length");
            assert!(!b.is_empty());
        }

        // Count bound (the acceptance criterion): at most 2*bit_length(range),
        // and hence at most 2*bit_length(tree_size).
        let n = u64::try_from(blocks.len()).unwrap();
        assert!(n <= max_blocks(end - start), "exceeds 2*bit_length(range)");
        assert!(
            n <= max_blocks(tree_size),
            "exceeds 2*bit_length(tree_size)"
        );

        blocks.len()
    }

    #[test]
    fn bit_length_and_max_blocks_edge_cases() {
        assert_eq!(bit_length(0), 0);
        assert_eq!(bit_length(1), 1);
        assert_eq!(bit_length(2), 2);
        assert_eq!(bit_length(7), 3);
        assert_eq!(bit_length(8), 4);
        assert_eq!(max_blocks(0), 0);
        assert_eq!(max_blocks(1), 2);
    }

    #[test]
    fn empty_and_inverted_ranges_are_empty() {
        assert!(decompose_range(Index(5), Index(5)).is_empty());
        assert!(decompose_range(Index(9), Index(4)).is_empty());
    }

    #[test]
    fn aligned_power_of_two_range_is_one_block() {
        let blocks = decompose_range(Index(0), Index(8));
        assert_eq!(blocks, vec![Subtree::new(Index(0), Index(8))]);
        let blocks = decompose_range(Index(4), Index(8));
        assert_eq!(blocks, vec![Subtree::new(Index(4), Index(8))]);
    }

    #[test]
    fn worked_example_zero_to_seven() {
        // [0,7) -> [0,4), [4,6), [6,7).
        let blocks = decompose_range(Index(0), Index(7));
        assert_eq!(
            blocks,
            vec![
                Subtree::new(Index(0), Index(4)),
                Subtree::new(Index(4), Index(6)),
                Subtree::new(Index(6), Index(7)),
            ],
        );
    }

    #[test]
    fn worked_example_one_to_four() {
        // [1,4) -> [1,2), [2,4).
        let blocks = decompose_range(Index(1), Index(4));
        assert_eq!(
            blocks,
            vec![
                Subtree::new(Index(1), Index(2)),
                Subtree::new(Index(2), Index(4)),
            ],
        );
    }

    #[test]
    fn exhaustive_small_ranges_hold_the_contract() {
        // Every sub-range of every tree size up to 64 satisfies disjointness,
        // exact cover, alignment, and the count bound.
        for tree_size in 0..=64u64 {
            for start in 0..=tree_size {
                for end in start..=tree_size {
                    assert_decomposition(start, end, tree_size);
                }
            }
        }
    }

    proptest! {
        #[test]
        fn decomposition_contract_holds(
            tree_size in 0u64..1_000_000,
            a in 0u64..1_000_000,
            b in 0u64..1_000_000,
        ) {
            let start = a.min(b).min(tree_size);
            let end = a.max(b).min(tree_size);
            // assert_decomposition panics on any violation; wrap so proptest
            // shrinks a counterexample rather than aborting.
            let count = assert_decomposition(start, end, tree_size);
            if start < end {
                prop_assert!(count >= 1);
            }
        }

        // Directly re-check the headline bound against tree size for large,
        // maximally-fragmenting ranges (odd start, odd end).
        #[test]
        fn count_within_twice_bitlength(tree_size in 1u64..(1u64 << 40)) {
            let start = 1.min(tree_size);
            let end = tree_size;
            let blocks = decompose_range(Index(start), Index(end));
            let n = u64::try_from(blocks.len()).unwrap();
            prop_assert!(n <= max_blocks(tree_size));
        }
    }
}
