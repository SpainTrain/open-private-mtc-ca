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

use thiserror::Error;

use crate::types::Index;

/// Errors constructing a [`Subtree`] through a checked constructor
/// ([`Subtree::try_new`] / [`Subtree::try_aligned`]).
///
/// The unchecked [`Subtree::new`] enforces its `start <= end` precondition only
/// with a `debug_assert!`; these variants make the same faults recoverable, so
/// proof and tile code (which must never panic on bad input) can reject them
/// rather than construct a `Subtree` whose [`len`](Subtree::len) would otherwise
/// have wrapped (crypto-review: `Subtree::len` computed `end - start`, which
/// wraps in release for an inverted range).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Error)]
pub enum SubtreeError {
    /// `start > end`: the half-open range is inverted and has no valid length.
    #[error("inverted subtree range: start {start} > end {end}")]
    Inverted {
        /// The (larger) inclusive lower bound that was supplied.
        start: u64,
        /// The (smaller) exclusive upper bound that was supplied.
        end: u64,
    },

    /// The range is empty, its length is not a power of two, or `start` is not
    /// a multiple of its length — i.e. it is not the aligned power-of-two block
    /// that a Merkle subtree node requires.
    #[error(
        "misaligned subtree range [{start}, {end}): \
         not a non-empty, aligned, power-of-two block"
    )]
    Misaligned {
        /// The inclusive lower bound that was supplied.
        start: u64,
        /// The exclusive upper bound that was supplied.
        end: u64,
    },
}

/// One aligned power-of-two block of the tree: the half-open entry range
/// `[start, end)`.
///
/// Every `Subtree` returned by [`decompose_range`] is **non-empty**
/// (`start < end`), has a power-of-two length, and is **aligned**
/// (`start` is a multiple of its length) — so it corresponds to exactly one
/// complete subtree, hence one interior (or leaf) node, of the Merkle tree.
///
/// # Alignment invariant (crypto-hardening)
///
/// The **only** representable range with `start > end` is one built by the
/// unchecked [`new`](Subtree::new) in a release build, and even then
/// [`len`](Subtree::len) *saturates* to `0` rather than wrapping to a huge
/// value. Callers handling untrusted sizes should build ranges with
/// [`try_new`](Subtree::try_new) (rejects inversion) or
/// [`try_aligned`](Subtree::try_aligned) (rejects inversion **and**
/// misalignment), so a `Subtree` that reaches proof/tile hashing carries its
/// invariant by construction.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Subtree {
    start: Index,
    end: Index,
}

impl Subtree {
    /// Constructs the half-open range `[start, end)` without allocation.
    ///
    /// This is the fast, unchecked path used by [`decompose_range`], which only
    /// ever passes non-empty aligned power-of-two blocks. The `start <= end`
    /// precondition is enforced by a `debug_assert!`: misuse panics in debug and
    /// test builds (so it cannot slip through CI), while release builds pay no
    /// runtime check. Callers holding untrusted bounds should prefer the fallible
    /// [`try_new`](Self::try_new) / [`try_aligned`](Self::try_aligned)
    /// constructors, which return a [`SubtreeError`] instead of relying on the
    /// assertion.
    #[must_use]
    pub const fn new(start: Index, end: Index) -> Self {
        debug_assert!(
            start.0 <= end.0,
            "Subtree::new requires start <= end (inverted range)"
        );
        Self { start, end }
    }

    /// Constructs the half-open range `[start, end)`, rejecting an inverted
    /// range at runtime.
    ///
    /// Unlike [`new`](Self::new) this never panics; it is the constructor to use
    /// on any range derived from untrusted input.
    ///
    /// # Errors
    ///
    /// [`SubtreeError::Inverted`] if `start > end`.
    pub const fn try_new(start: Index, end: Index) -> Result<Self, SubtreeError> {
        if start.0 > end.0 {
            return Err(SubtreeError::Inverted {
                start: start.0,
                end: end.0,
            });
        }
        Ok(Self { start, end })
    }

    /// Constructs an **aligned power-of-two** block — the full invariant every
    /// [`decompose_range`] output satisfies and the only shape that names a real
    /// interior (or leaf) node of the Merkle tree.
    ///
    /// # Errors
    ///
    /// - [`SubtreeError::Inverted`] if `start > end`.
    /// - [`SubtreeError::Misaligned`] if the range is empty, its length is not a
    ///   power of two, or `start` is not a multiple of that length.
    pub const fn try_aligned(start: Index, end: Index) -> Result<Self, SubtreeError> {
        if start.0 > end.0 {
            return Err(SubtreeError::Inverted {
                start: start.0,
                end: end.0,
            });
        }
        // Non-wrapping: `start <= end` was just proven.
        let len = end.0 - start.0;
        if len == 0 || !len.is_power_of_two() || !start.0.is_multiple_of(len) {
            return Err(SubtreeError::Misaligned {
                start: start.0,
                end: end.0,
            });
        }
        Ok(Self { start, end })
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
    ///
    /// Uses a **saturating** subtraction so an inverted range (only reachable by
    /// misusing the unchecked [`new`](Self::new) in a release build) yields `0`
    /// rather than a wrapped, near-`u64::MAX` value — a length that would drive a
    /// downstream allocation or hash loop into disaster. For every well-formed
    /// `Subtree` (`start <= end`) this is exactly `end - start`.
    #[must_use]
    pub const fn len(self) -> u64 {
        self.end.0.saturating_sub(self.start.0)
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

    use super::{decompose_range, Subtree, SubtreeError};
    use crate::types::Index;

    #[test]
    fn try_new_rejects_only_inverted_ranges() {
        // Inverted: rejected with the exact bounds.
        assert_eq!(
            Subtree::try_new(Index(9), Index(4)).unwrap_err(),
            SubtreeError::Inverted { start: 9, end: 4 },
        );
        // Empty and forward ranges are accepted (try_new checks inversion only).
        assert!(Subtree::try_new(Index(5), Index(5)).is_ok());
        assert!(Subtree::try_new(Index(4), Index(9)).is_ok());
    }

    #[test]
    fn try_aligned_rejects_inverted_empty_and_misaligned() {
        // Inverted -> Inverted (checked before alignment).
        assert_eq!(
            Subtree::try_aligned(Index(8), Index(4)).unwrap_err(),
            SubtreeError::Inverted { start: 8, end: 4 },
        );
        // Empty, non-power-of-two length, and unaligned start -> Misaligned.
        for (start, end) in [(4u64, 4u64), (0, 3), (1, 3), (2, 6)] {
            assert_eq!(
                Subtree::try_aligned(Index(start), Index(end)).unwrap_err(),
                SubtreeError::Misaligned { start, end },
                "expected [{start}, {end}) to be rejected as misaligned",
            );
        }
        // A genuine aligned power-of-two block is accepted.
        let block = Subtree::try_aligned(Index(4), Index(8)).unwrap();
        assert_eq!(block.len(), 4);
    }

    #[test]
    fn every_decompose_range_block_is_try_aligned_valid() {
        // The invariant decompose_range promises is exactly the one try_aligned
        // enforces: each emitted block round-trips through the checked
        // constructor without error.
        for tree_size in 0..=64u64 {
            for start in 0..=tree_size {
                for end in start..=tree_size {
                    for block in decompose_range(Index(start), Index(end)) {
                        assert!(
                            Subtree::try_aligned(block.start(), block.end()).is_ok(),
                            "decompose_range emitted a non-aligned block {block:?}",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn try_new_empty_range_has_zero_saturating_len() {
        let empty = Subtree::try_new(Index(5), Index(5)).unwrap();
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "start <= end")]
    fn new_debug_asserts_on_inverted_range() {
        // In debug/test builds the unchecked constructor refuses an inverted
        // range, so a misaligned/inverted Subtree is unconstructable in CI.
        let _ = Subtree::new(Index(9), Index(4));
    }

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
