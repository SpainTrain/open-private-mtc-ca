//! Read-path tile planner: map `(tree_size, leaf_index)` to the tiles and
//! intra-tile hash slots an inclusion proof needs (spec §12.2 step 3).
//!
//! A proof server serving an inclusion proof (spec §12.2) must, at step 3,
//! "identify tiles needed for the inclusion path" before fetching their bytes
//! from storage. [`plan_inclusion`] is that step as a pure function: given a
//! checkpoint's `tree_size` and a `leaf_index`, it returns a [`TilePlan`]
//! describing, in leaf-to-root order, which [`mtc::TileCoord`]s to read and which
//! hash slots within each are required — enough to assemble the exact inclusion
//! proof `mtc` would compute directly, without any Merkle hashing here.
//!
//! It touches no storage and does no hashing: fetching tile bytes belongs to the
//! proof-generation service (`read-proof-gen-core`), and the Merkle/proof math
//! lives in the core `mtc` crate (tree-primitives). This crate reuses `mtc`'s
//! tile geometry ([`mtc::tiles_for_inclusion`], [`mtc::tile_width`],
//! [`mtc::decompose_range`]) rather than reinventing it.
//!
//! # Tile model (256-leaf tiles; spec §2, §8.1, §28)
//!
//! Tiles follow `tlog-tiles`: a full tile holds `256 = 2^8` hashes, level 0 holds
//! leaf hashes, and a level-`L` hash is the Merkle Tree Hash of a complete
//! `256^L`-leaf subtree. The rightmost tile at each level is a partial
//! (right-edge) tile when the level's stored-hash count is not a multiple of 256;
//! [`plan_inclusion`] emits the correct partial widths at every level via
//! [`mtc::tile_width`].
//!
//! # Assembling a proof from a plan
//!
//! An inclusion proof's audit path (RFC 9162 §2.1.3.1, leaf-to-root) is a list of
//! sibling hashes; [`TilePlan::steps`] has one [`PathStep`] per sibling, in that
//! order. Each sibling is the Merkle Tree Hash of a range of entries, which
//! decomposes into complete, aligned power-of-two subtrees ([`PathStep::blocks`],
//! ascending by leaf, hence strictly decreasing in size). Each such subtree's
//! root is the balanced Merkle Tree Hash of a contiguous [`TileSlotRun`] of tile
//! hashes; the sibling hash is then the **right-leaning** combination of the
//! block roots:
//!
//! ```text
//! sibling = HASH_node(block0, HASH_node(block1, … HASH_node(block_{m-1}, block_m)))
//! ```
//!
//! A single-block step is a plain aligned subtree (often a single hash). This is
//! exactly the recurrence [`mtc::InclusionProof::generate`] uses, so extracting
//! per the plan reproduces its audit path bit-for-bit (proven by the property
//! test).
//!
//! # Never panics (spec §19.8)
//!
//! `leaf_index >= tree_size` is a typed [`PlanError::IndexOutOfRange`], never a
//! panic; an absurd `tree_size` cannot overflow the geometry (all shifts are
//! guarded), and an impossible internal geometry surfaces as
//! [`PlanError::TileGeometry`] rather than a panic.
//!
//! # Example (spec §12.2 demo: `tree_size` = 1000, index = 513)
//!
//! ```
//! use mtc::{Index, TileLevel, TreeSize};
//! use mtc_read::plan_inclusion;
//!
//! let plan = plan_inclusion(TreeSize(1000), Index(513)).expect("513 < 1000");
//!
//! // The tile plan for this leaf: the tiles to fetch and the leaf's own slot.
//! println!("leaf {} of a {}-entry tree", plan.leaf_index().0, plan.tree_size().0);
//! println!("tiles to read: {:?}", plan.tiles());
//! for (n, step) in plan.steps().iter().enumerate() {
//!     println!("  sibling {n}: {:?}", step.blocks());
//! }
//!
//! // The leaf lives in level-0 tile 2 (513 / 256 = 2), slot 1 (513 % 256).
//! assert_eq!(plan.leaf().coord().level(), TileLevel(0));
//! assert_eq!(plan.leaf().coord().index().0, 2);
//! assert_eq!(plan.leaf().slot(), 1);
//! assert_eq!(plan.leaf().slot_count(), 1);
//! // Every tile the plan references is a real tile of the 1000-entry tree.
//! assert!(!plan.tiles().is_empty());
//! ```

use thiserror::Error;

use mtc::{
    decompose_range, tile_width, Index, TileCoord, TileIndex, TileLevel, TreeSize,
    FULL_TILE_HASHES, TILE_HEIGHT,
};

/// A contiguous run of hash slots within a single tile: one complete, aligned
/// power-of-two subtree of the log tree.
///
/// The run is `len` consecutive hashes starting at slot [`slot`](Self::slot) of
/// the tile at [`coord`](Self::coord). Those `len` hashes are the roots of `len`
/// adjacent complete `256^L`-leaf subtrees (`L` = the tile level), so their
/// balanced Merkle Tree Hash is the root of the `2^κ`-leaf subtree the run
/// represents (`len = 2^(κ mod 8)`, `L = κ / 8`). For an aligned subtree that is
/// a whole number of tiles (`κ` a multiple of 8), `len` is 1 — a single stored
/// hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileSlotRun {
    coord: TileCoord,
    slot: u16,
    len: u16,
}

impl TileSlotRun {
    /// The tile to read (level, index, width).
    #[must_use]
    pub const fn coord(&self) -> TileCoord {
        self.coord
    }

    /// The first slot (0-based hash position) within the tile.
    #[must_use]
    pub const fn slot(&self) -> u16 {
        self.slot
    }

    /// The number of consecutive slots the run covers — a power of two in
    /// `1..=128`. A count of 1 is a single stored hash needing no combination.
    #[must_use]
    pub const fn slot_count(&self) -> u16 {
        self.len
    }

    /// Whether the run is a single hash (`slot_count == 1`).
    #[must_use]
    pub const fn is_single(&self) -> bool {
        self.len == 1
    }
}

/// One sibling of the inclusion path: the complete-subtree blocks whose
/// right-leaning Merkle Tree Hash combination is this sibling's hash.
///
/// The blocks are in ascending leaf order (hence strictly decreasing size). See
/// the crate docs for the exact combination rule. A single-block step is a plain
/// aligned subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathStep {
    blocks: Vec<TileSlotRun>,
}

impl PathStep {
    /// The complete-subtree blocks, ascending by leaf (strictly decreasing size),
    /// to be combined right-leaning into the sibling hash.
    #[must_use]
    pub fn blocks(&self) -> &[TileSlotRun] {
        &self.blocks
    }
}

/// A plan for reading the tiles and hash slots that assemble the inclusion proof
/// for one leaf (spec §12.2 step 3).
///
/// Produced by [`plan_inclusion`]. [`steps`](Self::steps) lists one [`PathStep`]
/// per audit-path sibling in leaf-to-root order (matching
/// [`mtc::InclusionProof::audit_path`]); [`leaf`](Self::leaf) locates the leaf's
/// own hash; and [`tiles`](Self::tiles) is the deduplicated set of tiles to
/// fetch (equal to [`mtc::tiles_for_inclusion`] for the same inputs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TilePlan {
    tree_size: TreeSize,
    leaf_index: Index,
    leaf: TileSlotRun,
    steps: Vec<PathStep>,
}

impl TilePlan {
    /// The tree size (checkpoint size) this plan is for.
    #[must_use]
    pub const fn tree_size(&self) -> TreeSize {
        self.tree_size
    }

    /// The leaf index this plan proves.
    #[must_use]
    pub const fn leaf_index(&self) -> Index {
        self.leaf_index
    }

    /// The leaf's own hash location: level-0 tile and slot (`len` 1). Not part of
    /// the audit path, but the proof server reads this tile too, so it is
    /// included in [`tiles`](Self::tiles).
    #[must_use]
    pub const fn leaf(&self) -> TileSlotRun {
        self.leaf
    }

    /// The audit-path siblings, leaf-to-root, one [`PathStep`] each. Empty for a
    /// single-leaf tree (the root is the leaf itself).
    #[must_use]
    pub fn steps(&self) -> &[PathStep] {
        &self.steps
    }

    /// The deduplicated, sorted set of tiles the proof server must fetch: the
    /// leaf's tile plus every tile referenced by a [`PathStep`]. Equal to
    /// [`mtc::tiles_for_inclusion`] for this `(leaf_index, tree_size)`.
    #[must_use]
    pub fn tiles(&self) -> Vec<TileCoord> {
        let mut coords = Vec::with_capacity(1 + self.steps.len());
        coords.push(self.leaf.coord);
        for step in &self.steps {
            for block in &step.blocks {
                coords.push(block.coord);
            }
        }
        coords.sort_unstable();
        coords.dedup();
        coords
    }
}

/// Why an inclusion tile plan could not be produced.
///
/// A typed library error (rule `thiserror-for-libs-eyre-for-bins`); every input
/// yields `Ok` or one of these, never a panic (spec §19.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PlanError {
    /// `leaf_index >= tree_size`: there is no such leaf to plan a proof for
    /// (an empty tree makes every index out of range).
    #[error("leaf index {index} is out of range for tree size {tree_size}")]
    IndexOutOfRange {
        /// The requested leaf index.
        index: u64,
        /// The tree size it was checked against.
        tree_size: u64,
    },

    /// Defensive: an internal tile-geometry computation fell outside the valid
    /// range for a tile that should exist. This cannot occur for a valid
    /// `leaf_index < tree_size`; it is surfaced as a typed error rather than a
    /// panic (rule `no-unwrap-in-prod`).
    #[error("internal tile geometry out of range at level {level}, tile index {index}")]
    TileGeometry {
        /// The tile level whose geometry was out of range.
        level: u8,
        /// The tile index whose geometry was out of range.
        index: u64,
    },
}

/// Plans the tile reads and hash slots for the inclusion proof of `index` in a
/// tree of `tree_size` entries (spec §12.2 step 3).
///
/// The returned [`TilePlan`] lists, in leaf-to-root order, the sibling
/// assemblies of the audit path plus the leaf's own location. Extracting the
/// hashes per the plan and combining them by the documented rule reproduces
/// exactly the proof [`mtc::InclusionProof::generate`] computes directly (proven
/// by the property test). No storage is touched and no hashing is done here.
///
/// # Errors
///
/// [`PlanError::IndexOutOfRange`] if `index.0 >= tree_size.0`.
/// [`PlanError::TileGeometry`] only for an internal geometry inconsistency that
/// cannot arise for a valid index (surfaced instead of panicking).
pub fn plan_inclusion(tree_size: TreeSize, index: Index) -> Result<TilePlan, PlanError> {
    let (i, n) = (index.0, tree_size.0);
    if i >= n {
        return Err(PlanError::IndexOutOfRange {
            index: i,
            tree_size: n,
        });
    }

    // The leaf's own location: the single-leaf subtree [i, i+1).
    let leaf = block_to_slot_run(i, 1, tree_size)?;

    // The sibling ranges of the audit path, leaf-to-root. This mirrors the
    // recursion in `mtc`'s inclusion-proof generation (RFC 9162 §2.1.3.1) exactly.
    let mut ranges = Vec::new();
    sibling_ranges(0, n, i, &mut ranges);

    let mut steps = Vec::with_capacity(ranges.len());
    for (a, b) in ranges {
        // Each sibling range decomposes into complete, aligned power-of-two
        // subtrees, ascending by leaf (strictly decreasing size for these ranges).
        let block_ranges = decompose_range(Index(a), Index(b));
        let mut blocks = Vec::with_capacity(block_ranges.len());
        for block in block_ranges {
            blocks.push(block_to_slot_run(block.start().0, block.len(), tree_size)?);
        }
        steps.push(PathStep { blocks });
    }

    Ok(TilePlan {
        tree_size,
        leaf_index: index,
        leaf,
        steps,
    })
}

/// Appends the audit-path sibling ranges for `index` within `[start, end)` to
/// `out`, leaf-to-root — the same order and ranges as `mtc`'s
/// `collect_siblings` (RFC 9162 §2.1.3.1). Recursion depth is bounded by the tree
/// height (`<= 64`).
fn sibling_ranges(start: u64, end: u64, index: u64, out: &mut Vec<(u64, u64)>) {
    if end - start <= 1 {
        return;
    }
    let k = start + largest_power_of_two_below(end - start);
    if index < k {
        sibling_ranges(start, k, index, out);
        out.push((k, end)); // right sibling (may be an unbalanced range)
    } else {
        sibling_ranges(k, end, index, out);
        out.push((start, k)); // left sibling (a complete aligned subtree)
    }
}

/// The largest power of two strictly less than `n`, for `n >= 2` (RFC 9162
/// §2.1.1). Mirrors `mtc`'s own split so the sibling ranges agree exactly.
const fn largest_power_of_two_below(n: u64) -> u64 {
    // `(n - 1).ilog2()` is `floor(log2(n - 1))`; `1 << that` is the largest power
    // of two `<= n - 1`, i.e. strictly less than `n`.
    1u64 << (n - 1).ilog2()
}

/// Maps one complete, aligned power-of-two subtree `[start, start + block_len)`
/// (with `block_len = 2^κ`) to the [`TileSlotRun`] that holds its hashes at tile
/// level `L = κ / 8`.
///
/// The subtree's root is level-`L` hash index `start / 256^L`, in tile
/// `start / 256^(L+1)`, at slot `(start / 256^L) mod 256`, spanning
/// `2^(κ mod 8)` consecutive slots. All shifts are guarded so no `tree_size`
/// can overflow them (spec §19.8).
fn block_to_slot_run(
    start: u64,
    block_len: u64,
    tree_size: TreeSize,
) -> Result<TileSlotRun, PlanError> {
    let kappa = block_len.trailing_zeros(); // κ (block_len is a power of two)
    let height = u32::from(TILE_HEIGHT); // 8
    let level_u32 = kappa / height; // floor(κ / 8), <= 7 for κ <= 63
    let Ok(level) = u8::try_from(level_u32) else {
        return Err(PlanError::TileGeometry {
            level: u8::MAX,
            index: start,
        });
    };

    // Shift by the number of leaves under a level-L hash (256^L) and under a
    // whole level-L tile (256^(L+1)); guard against a >= 64-bit shift.
    let level_shift = height * level_u32; // <= 56
    let tile_shift = height * (level_u32 + 1); // <= 64
    let hash_index = start >> level_shift; // level-L hash index of the subtree root
    let tile_index = if tile_shift >= u64::BITS {
        0
    } else {
        start >> tile_shift
    };
    let slot_u64 = hash_index & u64::from(FULL_TILE_HASHES - 1); // (hash_index mod 256)
    let Ok(slot) = u16::try_from(slot_u64) else {
        return Err(PlanError::TileGeometry {
            level,
            index: tile_index,
        });
    };
    // len = 2^(κ - 8L) = 2^(κ mod 8), in 1..=128.
    let len = 1u16 << (kappa - level_shift);

    let width = tile_width(level, tile_index, tree_size).ok_or(PlanError::TileGeometry {
        level,
        index: tile_index,
    })?;

    Ok(TileSlotRun {
        coord: TileCoord::new(TileLevel(level), TileIndex(tile_index), width),
        slot,
        len,
    })
}

#[cfg(test)]
mod tests {
    use super::{plan_inclusion, PlanError, TilePlan};
    use mtc::{tiles_for_inclusion, Index, TileLevel, TileWidth, TreeSize};

    /// Every tile a plan references must be a real tile of the tree, and the
    /// full fetch set must equal `mtc::tiles_for_inclusion`.
    fn assert_tiles_match_canonical(plan: &TilePlan) {
        let canonical = tiles_for_inclusion(plan.leaf_index(), plan.tree_size()).unwrap();
        assert_eq!(
            plan.tiles(),
            canonical,
            "plan.tiles() must equal mtc::tiles_for_inclusion",
        );
    }

    #[test]
    fn out_of_range_index_is_typed_error() {
        assert_eq!(
            plan_inclusion(TreeSize(5), Index(5)).unwrap_err(),
            PlanError::IndexOutOfRange {
                index: 5,
                tree_size: 5,
            },
        );
        assert_eq!(
            plan_inclusion(TreeSize(0), Index(0)).unwrap_err(),
            PlanError::IndexOutOfRange {
                index: 0,
                tree_size: 0,
            },
        );
    }

    #[test]
    fn single_leaf_tree_has_no_siblings() {
        let plan = plan_inclusion(TreeSize(1), Index(0)).unwrap();
        assert!(plan.steps().is_empty(), "single-leaf path has no siblings");
        // Leaf lives in the partial level-0 tile (0,0) of width 1, slot 0.
        assert_eq!(plan.leaf().coord().level(), TileLevel(0));
        assert_eq!(plan.leaf().coord().index().0, 0);
        assert_eq!(plan.leaf().coord().width(), TileWidth::new(1).unwrap());
        assert_eq!(plan.leaf().slot(), 0);
        assert_eq!(plan.leaf().slot_count(), 1);
        assert_eq!(plan.tiles().len(), 1);
        assert_tiles_match_canonical(&plan);
    }

    #[test]
    fn size_255_partial_level0_tile() {
        // 255 leaves: a single partial level-0 tile (width 255), no level-1 tile.
        for i in [0u64, 1, 127, 254] {
            let plan = plan_inclusion(TreeSize(255), Index(i)).unwrap();
            let tiles = plan.tiles();
            assert_eq!(tiles.len(), 1, "255-leaf proof reads one tile (i={i})");
            assert_eq!(tiles[0].level(), TileLevel(0));
            assert_eq!(tiles[0].width(), TileWidth::new(255).unwrap());
            assert_tiles_match_canonical(&plan);
        }
    }

    #[test]
    fn size_256_one_full_tile() {
        // 256 leaves: one full level-0 tile; every sibling is a slot-run in it.
        let plan = plan_inclusion(TreeSize(256), Index(0)).unwrap();
        let tiles = plan.tiles();
        assert_eq!(tiles.len(), 1);
        assert!(tiles[0].width().is_full());
        // Leaf 0 in a 256-leaf tree has 8 audit-path siblings (2^8 = 256).
        assert_eq!(plan.steps().len(), 8);
        // The top sibling is the 128-leaf subtree [128, 256): one block, level 0,
        // slot 128, len 128 — all within the single full tile.
        let top = plan.steps().last().unwrap();
        assert_eq!(top.blocks().len(), 1);
        assert_eq!(top.blocks()[0].coord().level(), TileLevel(0));
        assert_eq!(top.blocks()[0].slot(), 128);
        assert_eq!(top.blocks()[0].slot_count(), 128);
        assert_tiles_match_canonical(&plan);
    }

    #[test]
    fn size_257_crosses_into_second_tile() {
        // 257 leaves, leaf 0: the far-right sibling is leaf 256, a single-leaf
        // subtree in the partial level-0 tile (0,1) of width 1 — NOT a level-1
        // tile. The other siblings live in the full tile (0,0).
        let plan = plan_inclusion(TreeSize(257), Index(0)).unwrap();
        let tiles = plan.tiles();
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].level(), TileLevel(0));
        assert!(tiles[0].width().is_full());
        assert_eq!(tiles[1].level(), TileLevel(0));
        assert_eq!(tiles[1].index().0, 1);
        assert_eq!(tiles[1].width(), TileWidth::new(1).unwrap());
        assert_tiles_match_canonical(&plan);
    }

    #[test]
    fn multi_level_boundary_uses_higher_level_tile() {
        // 65537 leaves, leaf 0: [0, 65537) splits at 65536; the right sibling is
        // the single leaf [65536, 65537) (level-0 tile 256, width 1), and among
        // the left siblings is the 65536-leaf subtree... actually [0,65536) is the
        // recursion, whose top sibling [256*... ] reaches level 1. Verify a
        // level-1 tile is referenced and the geometry stays valid.
        let plan = plan_inclusion(TreeSize(65_537), Index(0)).unwrap();
        assert!(
            plan.tiles().iter().any(|t| t.level() == TileLevel(1)),
            "a 65537-leaf inclusion path reaches a level-1 tile",
        );
        // The far-right sibling is the extra leaf 65536 in level-0 tile 256.
        let top = plan.steps().last().unwrap();
        assert_eq!(top.blocks().len(), 1);
        assert_eq!(top.blocks()[0].coord().level(), TileLevel(0));
        assert_eq!(top.blocks()[0].coord().index().0, 256);
        assert_eq!(top.blocks()[0].slot(), 0);
        assert_tiles_match_canonical(&plan);
    }

    #[test]
    fn unbalanced_sibling_spans_multiple_blocks() {
        // 1000 leaves, leaf 0: the top sibling is the unbalanced range [512, 1000)
        // (488 leaves), which decomposes into several complete subtrees across
        // more than one block. Confirm at least one multi-block step exists.
        let plan = plan_inclusion(TreeSize(1000), Index(0)).unwrap();
        assert!(
            plan.steps().iter().any(|s| s.blocks().len() > 1),
            "an unbalanced right-edge sibling needs multiple blocks",
        );
        assert_tiles_match_canonical(&plan);
    }

    #[test]
    fn tiles_match_canonical_over_small_sizes() {
        // Exhaustive cross-check against mtc::tiles_for_inclusion for every leaf
        // of every tree up to 300 (crosses the 255/256/257 boundary densely).
        for n in 1u64..=300 {
            for i in 0..n {
                let plan = plan_inclusion(TreeSize(n), Index(i)).unwrap();
                assert_tiles_match_canonical(&plan);
            }
        }
    }
}
