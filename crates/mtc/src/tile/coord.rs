//! Tile coordinates, geometry, and the `tlog-tiles` path format.
//!
//! A tile is addressed by three quantities (matching the `tlog-tiles` serving
//! format, `c2sp.org/tlog-tiles`, adopted by the architecture spec section 28):
//!
//! - a **level** [`TileLevel`] `L` — level 0 tiles hold leaf hashes; at level
//!   `L >= 1` each hash is the Merkle Tree Hash of a *full* tile at the level
//!   below;
//! - an **index** [`TileIndex`] `N` — the tile's position within its level;
//! - a **width** [`TileWidth`] `W` in `1..=256` — the number of hashes the tile
//!   holds. A *full* tile has `W == 256`; a *partial* (right-edge) tile has
//!   `1 <= W <= 255` and, per `tlog-tiles`, "MUST NOT be hashed into a tile at
//!   the level above".
//!
//! ## Tile geometry (`tlog-tiles`)
//!
//! With tile height `H = 8` (so a full tile is `2^8 = 256` hashes), the `i`-th
//! hash (`0 <= i < W`) of the tile at level `L`, index `N` is
//!
//! ```text
//! MTH( D[ (N*256 + i) * 256^L : (N*256 + i + 1) * 256^L ) ] )
//! ```
//!
//! i.e. the Merkle Tree Hash (RFC 9162 §2.1.1) of a *complete* `256^L`-leaf
//! subtree. Hence the whole tile `(L, N)` covers the leaf range
//! `[N*256^(L+1), N*256^(L+1) + W*256^L)`, and the number of hashes stored at
//! level `L` for a tree of `n` leaves is `floor(n / 256^L)` — only *complete*
//! `256^L`-leaf subtrees have a stored hash. The rightmost tile at a level is
//! partial exactly when that count is not a multiple of 256 (spec section 28;
//! `tlog-tiles` partial-tile width `W = floor(n / 256^L) mod 256`).
//!
//! ## Path format
//!
//! [`TileCoord::path`] renders the canonical `tlog-tiles` path
//! `tile/<L>/<N>[.p/<W>]`, with `N` encoded as `x`-prefixed zero-padded
//! three-digit groups (`c2sp.org/tlog-tiles`). This is the read-path/serving
//! address; the spec section 8.1 S3 layout (`tiles/<L>/...`) is the storage
//! epic's key scheme layered on top and is out of scope here.

use super::TileError;
use crate::tree::{decompose_range, Subtree};
use crate::types::{Index, TreeSize};

/// The tile height `H` (`tlog-tiles`): a full tile is `2^H` hashes wide.
///
/// Fixed at 8 by `tlog-tiles` (spec section 28), so a full tile is
/// [`FULL_TILE_HASHES`] = 256 hashes.
pub const TILE_HEIGHT: u8 = 8;

/// The number of hashes in a full tile (`2^TILE_HEIGHT` = 256).
///
/// A full tile serializes to `256 * 32 = 8192` bytes (`tlog-tiles`: "Full tiles
/// MUST be exactly 256 hashes wide, or 8,192 bytes").
pub const FULL_TILE_HASHES: u16 = 1 << TILE_HEIGHT;

/// `256^level`, the number of leaves under one hash at tile `level`.
///
/// Computed in `u128` so that `level == 8` (`256^8 == 2^64`) does not overflow;
/// `level` is bounded by [`MAX_LEVEL`], which keeps `8 * level < 128`.
const fn leaf_span(level: u8) -> u128 {
    1u128 << (TILE_HEIGHT as u32 * level as u32)
}

/// The largest tile level this module will form. A `u64` tree size has at most
/// `ceil(64/8) = 8` tile levels, so level 8 is the ceiling; the guard keeps the
/// `leaf_span` shift (`8 * level`) well within `u128`.
const MAX_LEVEL: u8 = 8;

/// The number of stored hashes at tile `level` for a tree of `tree_size`
/// leaves: `floor(tree_size / 256^level)` — the count of *complete*
/// `256^level`-leaf subtrees (`tlog-tiles`; spec section 28).
#[must_use]
pub fn hashes_at_level(tree_size: TreeSize, level: u8) -> u64 {
    if level > MAX_LEVEL {
        return 0;
    }
    // `u128` division cannot overflow and the quotient fits `u64` because the
    // dividend does.
    u64::try_from(u128::from(tree_size.0) / leaf_span(level)).unwrap_or(0)
}

/// The width of the tile at (`level`, `index`) for a tree of `tree_size`
/// leaves, or `None` if that tile holds no complete-subtree hash (its first
/// hash index is at or beyond the stored count).
///
/// The rightmost non-empty tile at a level is partial (`1..=255`) when the
/// level's stored-hash count is not a multiple of 256; every tile to its left
/// is full (256). This is the `tlog-tiles` partial-tile rule (spec section 28).
#[must_use]
pub fn tile_width(level: u8, index: u64, tree_size: TreeSize) -> Option<TileWidth> {
    let count = hashes_at_level(tree_size, level);
    // First stored-hash index this tile would hold.
    let base = index.checked_mul(u64::from(FULL_TILE_HASHES))?;
    if base >= count {
        return None;
    }
    // `count - base >= 1`; clamp to a full tile.
    let available = count - base;
    let width = u16::try_from(available.min(u64::from(FULL_TILE_HASHES))).ok()?;
    TileWidth::new(width)
}

/// The level `L` of a tile (`tlog-tiles`): level 0 holds leaf hashes, level
/// `L >= 1` holds Merkle Tree Hashes of full tiles at level `L - 1`.
///
/// A newtype (rule `use-newtypes`, spec section 22.1) so a level cannot be
/// confused with an index or a width. `u8` is ample: a `u64`-sized tree has at
/// most 8 tile levels.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TileLevel(pub u8);

/// The index `N` of a tile within its level (`tlog-tiles`): the tile's position,
/// counting full-tile-sized blocks of stored hashes from the left.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TileIndex(pub u64);

/// The width `W` of a tile: the number of hashes it holds, always `1..=256`.
///
/// A full tile has width [`FULL_TILE_HASHES`] (256); a partial (right-edge) tile
/// has `1..=255`. The `1..=256` invariant is established once, at construction
/// ([`TileWidth::new`]), so an out-of-range width — including the empty-tile
/// width 0 that `tlog-tiles` forbids — can never reach a tile or a path
/// (crypto review F3 / bead `mtc-qka.3`: minimum-length fields are enforced by
/// construction, not assumed).
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TileWidth(u16);

impl TileWidth {
    /// The width of a full tile (256 hashes).
    pub const FULL: Self = Self(FULL_TILE_HASHES);

    /// Creates a width, returning `None` unless `1 <= width <= 256`.
    ///
    /// Rejecting `0` is the hand-enforced minimum-length check: a tile always
    /// holds at least one hash (`tlog-tiles`; crypto review F3).
    #[must_use]
    pub const fn new(width: u16) -> Option<Self> {
        if width >= 1 && width <= FULL_TILE_HASHES {
            Some(Self(width))
        } else {
            None
        }
    }

    /// The width as a `u16` (always `1..=256`).
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// The width as a `usize` (always `1..=256`).
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Whether this is a full tile (`width == 256`).
    #[must_use]
    pub const fn is_full(self) -> bool {
        self.0 == FULL_TILE_HASHES
    }

    /// Whether this is a partial (right-edge) tile (`width < 256`).
    #[must_use]
    pub const fn is_partial(self) -> bool {
        !self.is_full()
    }
}

/// The full address of a tile: its level, index, and width (`tlog-tiles`).
///
/// Width is part of the coordinate because it selects the tile's byte length
/// and its path (`.p/<W>` for a partial tile); the same `(level, index)` at two
/// tree sizes can be a full tile or a partial one.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TileCoord {
    level: TileLevel,
    index: TileIndex,
    width: TileWidth,
}

impl TileCoord {
    /// Constructs a tile coordinate from its level, index, and width.
    #[must_use]
    pub const fn new(level: TileLevel, index: TileIndex, width: TileWidth) -> Self {
        Self {
            level,
            index,
            width,
        }
    }

    /// The tile level `L`.
    #[must_use]
    pub const fn level(self) -> TileLevel {
        self.level
    }

    /// The tile index `N` within its level.
    #[must_use]
    pub const fn index(self) -> TileIndex {
        self.index
    }

    /// The tile width `W` (`1..=256`).
    #[must_use]
    pub const fn width(self) -> TileWidth {
        self.width
    }

    /// Whether this is a partial (right-edge) tile.
    #[must_use]
    pub const fn is_partial(self) -> bool {
        self.width.is_partial()
    }

    /// The half-open range of leaf indices this tile's hashes cover,
    /// `[start, end)` with `start = N*256^(L+1)` and
    /// `end = start + W*256^L`.
    ///
    /// Returns `None` only if the arithmetic would overflow `u64` (a tree far
    /// larger than any this library builds).
    #[must_use]
    pub fn leaf_range(self) -> Option<(u64, u64)> {
        // `TileCoord::new` is public and unvalidated, so `level` may be
        // attacker-controlled (a future tlog-tiles URL parser). Reject levels
        // beyond any u64-sized tree's tiling first: this keeps every `leaf_span`
        // shift below `u128`'s width, so the computation cannot shift-overflow
        // (debug panic) or silently mask the shift (release) (crypto review F1).
        if self.level.0 > MAX_LEVEL {
            return None;
        }
        let span_below = leaf_span(self.level.0); // 256^level (level <= 8: safe)
        let tile_span = span_below.checked_mul(u128::from(FULL_TILE_HASHES))?; // 256^(level+1)
                                                                               // start = index * 256^(level+1). `checked_mul` rejects the value
                                                                               // overflow a huge index would otherwise wrap — `u128::checked_shl` would
                                                                               // *not*, as it only guards the shift count, not the significant bits.
        let start_u128 = u128::from(self.index.0).checked_mul(tile_span)?;
        let start = u64::try_from(start_u128).ok()?;
        let width_leaves = u128::from(self.width.get()).checked_mul(span_below)?;
        let end = u64::try_from(start_u128.checked_add(width_leaves)?).ok()?;
        Some((start, end))
    }

    /// The canonical `tlog-tiles` path for this tile: `tile/<L>/<N>` for a full
    /// tile and `tile/<L>/<N>.p/<W>` for a partial one, with `N` encoded as
    /// `x`-prefixed zero-padded three-digit groups (`c2sp.org/tlog-tiles`).
    ///
    /// Examples: the full level-0 tile 0 is `tile/0/000`; a partial level-0
    /// tile 1 of width 5 is `tile/0/001.p/5`; level 1, index 1234067 is
    /// `tile/1/x001/x234/067`.
    #[must_use]
    pub fn path(self) -> String {
        let index = encode_index(self.index.0);
        if self.width.is_partial() {
            format!("tile/{}/{index}.p/{}", self.level.0, self.width.get())
        } else {
            format!("tile/{}/{index}", self.level.0)
        }
    }
}

/// Encodes a tile index as `tlog-tiles` path elements: zero-padded three-digit
/// groups, most-significant first, every group but the last prefixed with `x`
/// (`c2sp.org/tlog-tiles`; e.g. `1234067 -> "x001/x234/067"`, `0 -> "000"`).
fn encode_index(mut n: u64) -> String {
    // Split into base-1000 groups, least-significant first.
    let mut groups = Vec::new();
    loop {
        groups.push(n % 1000);
        n /= 1000;
        if n == 0 {
            break;
        }
    }
    groups.reverse();
    let last = groups.len() - 1;
    groups
        .iter()
        .enumerate()
        .map(|(i, group)| {
            if i == last {
                format!("{group:03}")
            } else {
                format!("x{group:03}")
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The minimal set of tile coordinates a proof server must read to serve an
/// inclusion proof for `leaf` in a tree of `tree_size` leaves.
///
/// Supports spec section 12.2 step 3 ("Identify tiles needed for inclusion
/// path").
///
/// An RFC 9162 §2.1.1 inclusion proof for a single leaf combines that leaf with
/// the roots of the *complete* subtrees that tile the rest of the tree — exactly
/// the blocks of [`decompose_range`] over `[0, leaf)` and `[leaf+1, tree_size)`.
/// Each such complete-subtree root, and the leaf hash itself, lives in one tile;
/// this function maps each to its tile and returns the deduplicated, sorted set.
/// Because whole tiles are fetched, several proof hashes sharing a tile collapse
/// to one coordinate, so the set is minimal in tiles.
///
/// # Errors
///
/// [`TileError::LeafOutOfRange`] if `leaf >= tree_size` (there is no such leaf
/// to prove). [`TileError::WidthOutOfRange`] is returned only if an internal
/// width computation falls outside `1..=256`, which cannot happen for a valid
/// `leaf < tree_size`; it is surfaced as an error rather than a panic.
pub fn tiles_for_inclusion(leaf: Index, tree_size: TreeSize) -> Result<Vec<TileCoord>, TileError> {
    let (i, n) = (leaf.0, tree_size.0);
    if i >= n {
        return Err(TileError::LeafOutOfRange {
            leaf: i,
            tree_size: n,
        });
    }

    // The leaf's own tile (its single-leaf "subtree" [i, i+1)) plus the
    // complete-subtree siblings on either side.
    let mut ranges = vec![Subtree::new(Index(i), Index(i + 1))];
    ranges.extend(decompose_range(Index(0), Index(i)));
    ranges.extend(decompose_range(Index(i + 1), Index(n)));

    let mut coords = Vec::with_capacity(ranges.len());
    for block in ranges {
        coords.push(tile_for_subtree(block, tree_size)?);
    }
    coords.sort_unstable();
    coords.dedup();
    Ok(coords)
}

/// Maps one complete, aligned power-of-two subtree `block` to the tile that
/// holds (or, for a within-tile interior node, can recompute) its root, at the
/// given tree size.
///
/// For a subtree of `2^k` leaves starting at `a`: its root sits at tree level
/// `k`, so at tile level `L = floor(k / 8)`, in the tile `N = a / 256^(L+1)`.
fn tile_for_subtree(block: Subtree, tree_size: TreeSize) -> Result<TileCoord, TileError> {
    let a = block.start().0;
    let k = block.len().trailing_zeros(); // block length is a power of two
    let level = u8::try_from(k / u32::from(TILE_HEIGHT)).unwrap_or(MAX_LEVEL);
    // Tile index = a / 256^(level+1). Guard the shift: a >> s is 0 once
    // s >= 64 (a is a u64), and would otherwise panic.
    let shift = u32::from(TILE_HEIGHT) * (u32::from(level) + 1);
    let tile_index = if shift >= u64::BITS { 0 } else { a >> shift };
    let width =
        tile_width(level, tile_index, tree_size).ok_or(TileError::WidthOutOfRange { width: 0 })?;
    Ok(TileCoord::new(
        TileLevel(level),
        TileIndex(tile_index),
        width,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        encode_index, hashes_at_level, tile_width, tiles_for_inclusion, TileCoord, TileIndex,
        TileLevel, TileWidth, FULL_TILE_HASHES, MAX_LEVEL,
    };
    use crate::tree::decompose_range;
    use crate::types::{Index, TreeSize};
    use proptest::prelude::*;

    #[test]
    fn tile_width_new_enforces_range() {
        assert_eq!(TileWidth::new(0), None);
        assert_eq!(TileWidth::new(1).map(TileWidth::get), Some(1));
        assert_eq!(TileWidth::new(256).map(TileWidth::get), Some(256));
        assert_eq!(TileWidth::new(257), None);
        assert!(TileWidth::FULL.is_full());
        assert!(TileWidth::new(255).unwrap().is_partial());
        assert!(TileWidth::new(1).unwrap().is_partial());
    }

    #[test]
    fn hashes_at_level_counts_complete_subtrees() {
        // 1000 leaves: 1000 level-0 hashes, floor(1000/256)=3 level-1 hashes,
        // floor(1000/65536)=0 level-2 hashes.
        assert_eq!(hashes_at_level(TreeSize(1000), 0), 1000);
        assert_eq!(hashes_at_level(TreeSize(1000), 1), 3);
        assert_eq!(hashes_at_level(TreeSize(1000), 2), 0);
        // Exact powers of 256.
        assert_eq!(hashes_at_level(TreeSize(65536), 1), 256);
        assert_eq!(hashes_at_level(TreeSize(65536), 2), 1);
    }

    #[test]
    fn tile_width_full_partial_and_empty() {
        // 1000 level-0 hashes -> tiles 0..3: three full (256), one partial (232).
        assert_eq!(tile_width(0, 0, TreeSize(1000)), TileWidth::new(256));
        assert_eq!(tile_width(0, 3, TreeSize(1000)), TileWidth::new(1000 - 768));
        // Tile 4 at level 0 has no hashes (1000 < 4*256).
        assert_eq!(tile_width(0, 4, TreeSize(1000)), None);
        // Level 1 has 3 hashes -> a single partial tile of width 3.
        assert_eq!(tile_width(1, 0, TreeSize(1000)), TileWidth::new(3));
    }

    #[test]
    fn index_path_encoding_matches_tlog_tiles() {
        assert_eq!(encode_index(0), "000");
        assert_eq!(encode_index(1), "001");
        assert_eq!(encode_index(999), "999");
        assert_eq!(encode_index(1000), "x001/000");
        assert_eq!(encode_index(1_234_067), "x001/x234/067");
    }

    #[test]
    fn tile_path_full_and_partial() {
        let full = TileCoord::new(TileLevel(0), TileIndex(0), TileWidth::FULL);
        assert_eq!(full.path(), "tile/0/000");
        let partial = TileCoord::new(TileLevel(0), TileIndex(1), TileWidth::new(5).unwrap());
        assert_eq!(partial.path(), "tile/0/001.p/5");
        let deep = TileCoord::new(TileLevel(1), TileIndex(1_234_067), TileWidth::FULL);
        assert_eq!(deep.path(), "tile/1/x001/x234/067");
    }

    #[test]
    fn leaf_range_never_panics_for_any_level() {
        // Crypto review F1: `TileCoord::new` is public and unvalidated, so a
        // future tlog-tiles URL parser could feed an attacker-controlled level.
        // `leaf_range` must never panic (no shift overflow at any level) and
        // must return `None` for a level beyond the u64 tiling ceiling. The
        // maximal index (`u64::MAX`) also exercises the value-overflow guard.
        for l in 0..=u8::MAX {
            let coord = TileCoord::new(TileLevel(l), TileIndex(u64::MAX), TileWidth::FULL);
            let range = coord.leaf_range(); // must return, never panic
            if l > MAX_LEVEL {
                assert_eq!(range, None, "level {l} (> {MAX_LEVEL}) must yield None");
            }
        }
    }

    #[test]
    fn leaf_range_covers_expected_leaves() {
        // Full level-0 tile 0: leaves [0, 256).
        let t = TileCoord::new(TileLevel(0), TileIndex(0), TileWidth::FULL);
        assert_eq!(t.leaf_range(), Some((0, 256)));
        // Full level-0 tile 1: leaves [256, 512).
        let t = TileCoord::new(TileLevel(0), TileIndex(1), TileWidth::FULL);
        assert_eq!(t.leaf_range(), Some((256, 512)));
        // Level-1 tile 0, width 3: leaves [0, 3*256).
        let t = TileCoord::new(TileLevel(1), TileIndex(0), TileWidth::new(3).unwrap());
        assert_eq!(t.leaf_range(), Some((0, 768)));
    }

    #[test]
    fn tiles_for_inclusion_rejects_out_of_range_leaf() {
        assert!(matches!(
            tiles_for_inclusion(Index(5), TreeSize(5)),
            Err(super::TileError::LeafOutOfRange {
                leaf: 5,
                tree_size: 5
            }),
        ));
    }

    #[test]
    fn tiles_for_inclusion_single_leaf_tree() {
        // n=1: proving leaf 0 needs only the partial level-0 tile (0,0) width 1.
        let coords = tiles_for_inclusion(Index(0), TreeSize(1)).unwrap();
        assert_eq!(
            coords,
            vec![TileCoord::new(
                TileLevel(0),
                TileIndex(0),
                TileWidth::new(1).unwrap()
            )],
        );
    }

    #[test]
    fn tiles_for_inclusion_right_edge_uses_lower_level_tile() {
        // n=257, leaf 0: the right sibling is leaf 256 (a single-leaf subtree),
        // which lives in the partial level-0 tile (0,1) width 1 -- NOT in a
        // level-1 tile. The left siblings are all in the full tile (0,0).
        let coords = tiles_for_inclusion(Index(0), TreeSize(257)).unwrap();
        assert_eq!(
            coords,
            vec![
                TileCoord::new(TileLevel(0), TileIndex(0), TileWidth::FULL),
                TileCoord::new(TileLevel(0), TileIndex(1), TileWidth::new(1).unwrap()),
            ],
        );
    }

    proptest! {
        // Section-12.2-step-3 correctness. A tiled inclusion proof for leaf i in
        // a tree of n leaves reads the roots of the *complete* subtrees tiling
        // [0, i) and [i+1, n) -- exactly `decompose_range` of those two ranges --
        // plus the leaf's own tile. (An RFC 9162 right-edge sibling may be an
        // unbalanced range that the server *reconstructs* from several of these
        // complete-subtree roots, so it is not itself a single tile.) Every such
        // complete subtree, computed independently here, must be contained in one
        // returned tile whose level is floor(k/8) for a 2^k-leaf block -- proving
        // `tiles_for_inclusion` returns every tile the proof needs. Widths are
        // valid and the set is sorted and duplicate-free (minimal in tiles).
        #[test]
        fn returned_tiles_cover_every_complete_subtree(
            n in 1u64..3000,
            raw in 0u64..3000,
        ) {
            let i = raw % n;
            let coords = tiles_for_inclusion(Index(i), TreeSize(n)).unwrap();

            // The complete subtrees a tiled proof reads (independent of the
            // impl's own tile mapping): the leaf, and the siblings on each side.
            let mut blocks = vec![(i, 1u64)];
            for b in decompose_range(Index(0), Index(i)) {
                blocks.push((b.start().0, b.len()));
            }
            for b in decompose_range(Index(i + 1), Index(n)) {
                blocks.push((b.start().0, b.len()));
            }

            for (start, len) in blocks {
                let k = len.trailing_zeros();
                let want_level = u8::try_from(k / 8).unwrap();
                // Independently: the block's leaf range must sit inside some
                // returned tile at the block's tile level.
                let covered = coords.iter().any(|c| {
                    let (ts, te) = c.leaf_range().unwrap();
                    c.level().0 == want_level && ts <= start && start + len <= te
                });
                prop_assert!(
                    covered,
                    "complete subtree [{start}, {}) (level {want_level}) uncovered; coords={coords:?}",
                    start + len,
                );
            }

            // Every returned width is a real 1..=256 width, and the set is sorted
            // and duplicate-free.
            for c in &coords {
                prop_assert!(c.width().get() >= 1 && c.width().get() <= FULL_TILE_HASHES);
            }
            let mut sorted = coords.clone();
            sorted.sort_unstable();
            sorted.dedup();
            prop_assert_eq!(sorted, coords);
        }
    }
}
