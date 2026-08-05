//! Tile structures for the issuance-log Merkle tree (`tlog-tiles`; spec
//! sections 2 and 28).
//!
//! A *tile* is a fixed 256-leaf (`2^8`) chunk of the tree's hashes at one level:
//! the `tlog-tiles` serving format (`c2sp.org/tlog-tiles`) the architecture spec
//! adopts (section 28) so that a proof server fetches a handful of immutable,
//! cacheable tiles instead of streaming every leaf. This module builds tiles
//! from a tree's leaf hashes, serializes and parses their bytes, addresses them
//! by coordinate and path, reconstructs the tree root from a tile set, and maps
//! an inclusion path to the tiles it needs (spec section 12.2 step 3).
//!
//! - [`coord`] — coordinates ([`TileLevel`], [`TileIndex`], [`TileWidth`],
//!   [`TileCoord`]), the tile geometry, the `tlog-tiles` [path format](TileCoord::path),
//!   and [`tiles_for_inclusion`].
//! - This module — the [`Tile`] value (a coordinate plus its hashes), tile
//!   [building](build_tiles), [root reconstruction](reconstruct_root), and the
//!   wire codec ([`Tile::to_bytes`] / [`Tile::from_bytes`]).
//!
//! # Hashing agreement (spec: "tile hashes must agree with tree node hashes")
//!
//! Tile hashes are computed with the *same* domain-separated constructions as
//! the tree (`hash_leaf` / [`hash_node`] from [`crate::tree::digest`]): a level-0
//! tile hash is a leaf hash, and a hash at level `L >= 1` is the Merkle Tree
//! Hash (RFC 9162 §2.1.1) of a complete `256^L`-leaf subtree — identical to the
//! interior node the tree computes for that range. There is no second hashing
//! path, so tile and tree hashes agree at every tile boundary by construction
//! (checked by the cross-tests below and against [`crate::MerkleTree`]).
//!
//! # Wire format
//!
//! A tile's bytes are the bare concatenation of its `W` hashes — `W * 32` bytes,
//! `8192` for a full tile — with the coordinate (level, index, width) carried
//! out of band in the [path](TileCoord::path), exactly as `tlog-tiles`
//! specifies. [`Tile::from_bytes`] parses through the bounded [`TlsReader`]
//! (spec section 19.3): it reads exactly the coordinate's width in hashes and
//! rejects truncated or oversized input with a [`WireError`] rather than
//! panicking. The minimum-length invariant — a tile holds at least one hash — is
//! hand-enforced by [`TileWidth`] (crypto review F3 / bead `mtc-qka.3`).

pub mod coord;

pub use coord::{
    hashes_at_level, tile_width, tiles_for_inclusion, TileCoord, TileIndex, TileLevel, TileWidth,
    FULL_TILE_HASHES, TILE_HEIGHT,
};

use std::io::{self, Write};

use thiserror::Error;

use crate::tree::{empty_root, hash_node, Hasher};
use crate::types::{HashOutput, TreeSize};
use crate::wire::{write_bytes, TlsReader, TlsSerialize, WireError};

/// The byte length of one hash on the wire (SHA-256; [`HashOutput::LEN`]).
const HASH_BYTES: usize = HashOutput::LEN;

/// An error constructing or addressing a tile.
///
/// Parsing tile *bytes* fails with [`WireError`] instead (the untrusted-input
/// path, spec section 19.3); this enum covers the structural invariants of
/// building and indexing tiles (rule `thiserror-for-libs-eyre-for-bins`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TileError {
    /// A tile was constructed with a hash count that disagrees with its
    /// coordinate's width. The width is the single source of truth for how many
    /// hashes a tile holds (`tlog-tiles`).
    #[error("tile hash count {actual} does not match coordinate width {expected}")]
    HashCountMismatch {
        /// Hash count the coordinate's width requires.
        expected: usize,
        /// Hash count actually supplied.
        actual: usize,
    },

    /// An inclusion path was requested for a leaf that is not in the tree.
    #[error("leaf index {leaf} is out of range for a tree of {tree_size} leaves")]
    LeafOutOfRange {
        /// The requested leaf index.
        leaf: u64,
        /// The tree size the leaf was requested against.
        tree_size: u64,
    },

    /// An internal tile-width computation produced a value outside `1..=256`.
    ///
    /// Defensive: this cannot occur for a valid `leaf < tree_size` (see
    /// [`tiles_for_inclusion`]); it is surfaced as an error rather than a panic
    /// (rule `no-unwrap-in-prod`).
    #[error("computed tile width {width} is outside the valid range 1..=256")]
    WidthOutOfRange {
        /// The offending width.
        width: u16,
    },
}

/// A materialized tile: a [`TileCoord`] together with the `W` hashes it holds,
/// where `W` is the coordinate's [width](TileCoord::width).
///
/// The invariant `hashes.len() == coord.width()` is upheld at construction
/// ([`Tile::new`]) and by [`build_tiles`], so every method can rely on it. The
/// hashes are in slot order: hash `i` is the Merkle Tree Hash of the `i`-th
/// complete `256^L`-leaf subtree the tile covers (see [`coord`]).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tile {
    coord: TileCoord,
    hashes: Vec<HashOutput>,
}

impl Tile {
    /// Constructs a tile from a coordinate and its hashes.
    ///
    /// # Errors
    ///
    /// [`TileError::HashCountMismatch`] unless `hashes.len()` equals the
    /// coordinate's width. Because [`TileWidth`] is `1..=256`, a valid tile
    /// always has between 1 and 256 hashes.
    pub fn new(coord: TileCoord, hashes: Vec<HashOutput>) -> Result<Self, TileError> {
        let expected = coord.width().as_usize();
        if hashes.len() != expected {
            return Err(TileError::HashCountMismatch {
                expected,
                actual: hashes.len(),
            });
        }
        Ok(Self { coord, hashes })
    }

    /// The tile's coordinate (level, index, width).
    #[must_use]
    pub const fn coord(&self) -> TileCoord {
        self.coord
    }

    /// The tile's hashes in slot order (length equals the coordinate's width).
    #[must_use]
    pub fn hashes(&self) -> &[HashOutput] {
        &self.hashes
    }

    /// The hash at `slot`, or `None` if `slot` is beyond the tile's width.
    #[must_use]
    pub fn hash(&self, slot: usize) -> Option<HashOutput> {
        self.hashes.get(slot).copied()
    }

    /// Whether this is a partial (right-edge) tile.
    #[must_use]
    pub const fn is_partial(&self) -> bool {
        self.coord.is_partial()
    }

    /// The tile's wire bytes: the bare concatenation of its hashes,
    /// `width * 32` bytes (`8192` for a full tile), per `tlog-tiles`.
    ///
    /// No length prefix and no coordinate are encoded; the coordinate travels in
    /// the [path](TileCoord::path). Round-trips with [`Tile::from_bytes`] given
    /// the same coordinate.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.hashes.len() * HASH_BYTES);
        for hash in &self.hashes {
            buf.extend_from_slice(hash.as_bytes());
        }
        buf
    }

    /// Parses a tile's bytes for a known coordinate, reading exactly the
    /// coordinate's width in 32-byte hashes.
    ///
    /// The coordinate (hence the width) comes from the tile's `tlog-tiles`
    /// [path](TileCoord::path); the bytes are the bare hash concatenation.
    /// Parsing goes through the bounded [`TlsReader`] (spec section 19.3): it
    /// never panics.
    ///
    /// # Errors
    ///
    /// [`WireError::UnexpectedEof`] if `bytes` is shorter than `width * 32`
    /// (truncated), and [`WireError::TrailingBytes`] if it is longer
    /// (oversized). Both are returned, never panicked.
    pub fn from_bytes(coord: TileCoord, bytes: &[u8]) -> Result<Self, WireError> {
        let width = coord.width().as_usize();
        let mut reader = TlsReader::new(bytes);
        let mut hashes = Vec::with_capacity(width);
        for _ in 0..width {
            hashes.push(HashOutput::from(reader.read_array::<HASH_BYTES>()?));
        }
        // Reject any trailing bytes: an oversized tile file is malformed.
        reader.finish()?;
        Ok(Self { coord, hashes })
    }
}

/// Serializes a tile as its bare hash concatenation (`tlog-tiles` tile bytes),
/// so a tile composes into the wire framework like any other value.
///
/// This is the same byte string as [`Tile::to_bytes`]; there is no length prefix
/// (the width is carried by the coordinate/path).
impl TlsSerialize for Tile {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        for hash in &self.hashes {
            write_bytes(writer, hash.as_bytes())?;
        }
        Ok(())
    }
}

/// Builds the complete set of tiles for a tree over `leaves` (its leaf hashes),
/// at every level, in level-then-index order (`tlog-tiles`; spec section 28).
///
/// Level 0 holds the leaf hashes; level `L >= 1` holds the Merkle Tree Hash of
/// each complete `256^L`-leaf subtree, computed with [`hash_node`] so the tile
/// hashes agree with the tree's node hashes at every boundary. The rightmost
/// tile at a level is [partial](TileWidth::is_partial) when the level's
/// stored-hash count is not a multiple of 256. An empty `leaves` yields no tiles
/// (the empty tree has no committed hashes; its root is
/// [`empty_root`](crate::tree::empty_root)).
///
/// Generic over the hash `H` so tiles match a [`MerkleTree<H>`](crate::MerkleTree)
/// of the same hasher (spec section 22.7); use [`Sha256Hasher`] for v1.
#[must_use]
pub fn build_tiles<H: Hasher>(leaves: &[HashOutput]) -> Vec<Tile> {
    let n = leaves.len() as u64;
    let tree_size = TreeSize(n);
    let mut tiles = Vec::new();
    let mut level: u8 = 0;
    loop {
        let count = hashes_at_level(tree_size, level);
        if count == 0 {
            break;
        }
        let num_tiles = count.div_ceil(u64::from(FULL_TILE_HASHES));
        for tile_n in 0..num_tiles {
            let base = tile_n * u64::from(FULL_TILE_HASHES);
            // width is 1..=256; the `min` bounds it, so the `try_from` never
            // actually fails (the fallback is a defensive full tile).
            let width = u16::try_from((count - base).min(u64::from(FULL_TILE_HASHES)))
                .unwrap_or(FULL_TILE_HASHES);
            let mut hashes = Vec::with_capacity(usize::from(width));
            for slot in 0..u64::from(width) {
                hashes.push(subtree_root::<H>(leaves, level, base + slot));
            }
            // width is 1..=256 and hashes.len() == width by construction.
            if let Some(w) = TileWidth::new(width) {
                tiles.push(Tile {
                    coord: TileCoord::new(TileLevel(level), TileIndex(tile_n), w),
                    hashes,
                });
            }
        }
        // Once a level has a single hash it is the tree root; nothing sits
        // above it. (The next level's count is 0, so the loop would break
        // anyway; this just avoids an extra iteration.)
        if count == 1 {
            break;
        }
        level += 1;
    }
    tiles
}

/// The Merkle Tree Hash of the complete `256^level`-leaf subtree whose root is
/// the level-`level` hash at `hash_index` (its leaf range is
/// `[hash_index*256^level, (hash_index+1)*256^level)`).
fn subtree_root<H: Hasher>(leaves: &[HashOutput], level: u8, hash_index: u64) -> HashOutput {
    // span = 256^level; computed in u128 then narrowed — callers only reach a
    // level whose complete subtrees fit within `leaves`.
    let span = 1u128 << (u32::from(TILE_HEIGHT) * u32::from(level));
    let start = usize::try_from(u128::from(hash_index) * span).unwrap_or(usize::MAX);
    let end = usize::try_from((u128::from(hash_index) + 1) * span).unwrap_or(usize::MAX);
    // The slice is a complete power-of-two block; `combine_subtree_roots`
    // reproduces its MTH (a single leaf for level 0). `get` keeps the impossible
    // out-of-range case panic-free.
    combine_subtree_roots::<H>(leaves.get(start..end).unwrap_or(&[]))
}

/// The Merkle Tree Hash (RFC 9162 §2.1.1) over a slice of already-computed
/// subtree-root hashes — the same recurrence [`crate::MerkleTree`] applies to
/// leaf hashes, so combining a tile's hashes reproduces the corresponding tree
/// node, and combining recovered leaf hashes reproduces the tree root.
///
/// Defined locally (rather than on the tree) to keep the merge with the sibling
/// `crates/mtc` beads mechanical. The empty slice maps to
/// [`empty_root`](crate::tree::empty_root); callers here only pass non-empty
/// slices.
fn combine_subtree_roots<H: Hasher>(hashes: &[HashOutput]) -> HashOutput {
    match hashes {
        [] => empty_root::<H>(),
        [single] => *single,
        _ => {
            let k = split_point(hashes.len());
            hash_node::<H>(
                &combine_subtree_roots::<H>(&hashes[..k]),
                &combine_subtree_roots::<H>(&hashes[k..]),
            )
        }
    }
}

/// The largest power of two strictly less than `n`, for `n >= 2` (RFC 9162
/// §2.1.1 split point). Mirrors the tree builder's own split so tile hashes and
/// tree node hashes agree.
fn split_point(n: usize) -> usize {
    debug_assert!(n >= 2, "split_point requires n >= 2");
    1usize << (n - 1).ilog2()
}

/// Reconstructs the tree root from a complete tile set, using only its level-0
/// (leaf-hash) tiles.
///
/// This is the cross-check that a tile set reproduces the root computed directly
/// from leaves (spec section 28; ticket acceptance criterion).
///
/// The level-0 tiles must contiguously cover `[0, n)`: their indices run
/// `0, 1, 2, ...` with no gap. Returns the Merkle Tree Hash of the recovered
/// leaf-hash sequence, or `None` if there are no level-0 tiles or they are not
/// contiguous. Higher-level tiles are derived from these and are validated
/// separately (see the consistency tests); they are not needed to recompute the
/// root.
///
/// Generic over `H` to match the hasher the tiles were built with.
#[must_use]
pub fn reconstruct_root<H: Hasher>(tiles: &[Tile]) -> Option<HashOutput> {
    let mut level0: Vec<&Tile> = tiles
        .iter()
        .filter(|t| t.coord.level() == TileLevel(0))
        .collect();
    level0.sort_unstable_by_key(|t| t.coord.index().0);
    if level0.is_empty() {
        return None;
    }
    let mut leaves = Vec::new();
    for (expected, tile) in level0.iter().enumerate() {
        if tile.coord.index().0 != expected as u64 {
            return None; // gap or duplicate in the level-0 cover
        }
        leaves.extend_from_slice(tile.hashes());
    }
    Some(combine_subtree_roots::<H>(&leaves))
}

#[cfg(test)]
mod tests {
    use super::{
        build_tiles, combine_subtree_roots, reconstruct_root, Tile, TileCoord, TileError,
        TileIndex, TileLevel, TileWidth, FULL_TILE_HASHES,
    };
    use crate::tree::{hash_leaf, MerkleTree, Sha256Hasher};
    use crate::types::{HashOutput, Index};
    use crate::wire::{TlsSerialize, WireError};
    use proptest::prelude::*;

    type Tree = MerkleTree<Sha256Hasher>;

    fn leaf_hashes(n: u64) -> Vec<HashOutput> {
        (0..n)
            .map(|i| hash_leaf::<Sha256Hasher>(format!("entry-{i}").as_bytes()))
            .collect()
    }

    fn tree_of(n: u64) -> Tree {
        let mut tree = Tree::new();
        for i in 0..n {
            tree.append(format!("entry-{i}").as_bytes());
        }
        tree
    }

    #[test]
    fn tile_new_enforces_hash_count() {
        let coord = TileCoord::new(TileLevel(0), TileIndex(0), TileWidth::new(2).unwrap());
        assert!(Tile::new(coord, vec![HashOutput([0; 32]); 2]).is_ok());
        assert_eq!(
            Tile::new(coord, vec![HashOutput([0; 32]); 3]).unwrap_err(),
            TileError::HashCountMismatch {
                expected: 2,
                actual: 3
            },
        );
    }

    #[test]
    fn empty_tree_has_no_tiles() {
        assert!(build_tiles::<Sha256Hasher>(&[]).is_empty());
        assert_eq!(reconstruct_root::<Sha256Hasher>(&[]), None);
    }

    #[test]
    fn full_tile_is_8192_bytes() {
        // A tree of exactly 256 leaves has one full level-0 tile: 256*32 bytes.
        let leaves = leaf_hashes(256);
        let tiles = build_tiles::<Sha256Hasher>(&leaves);
        let level0: Vec<_> = tiles
            .iter()
            .filter(|t| t.coord().level() == TileLevel(0))
            .collect();
        assert_eq!(level0.len(), 1);
        assert!(level0[0].coord().width().is_full());
        assert_eq!(level0[0].to_bytes().len(), 8192);
    }

    #[test]
    fn level0_tile_hashes_are_leaf_hashes() {
        let leaves = leaf_hashes(300);
        let tiles = build_tiles::<Sha256Hasher>(&leaves);
        // Tile (0,0) holds leaves 0..256; tile (0,1) holds leaves 256..300.
        let t0 = tiles
            .iter()
            .find(|t| t.coord() == TileCoord::new(TileLevel(0), TileIndex(0), TileWidth::FULL))
            .unwrap();
        for (i, h) in t0.hashes().iter().enumerate() {
            assert_eq!(*h, leaves[i]);
        }
        let partial = tiles
            .iter()
            .find(|t| t.coord().level() == TileLevel(0) && t.coord().index() == TileIndex(1))
            .unwrap();
        assert_eq!(partial.coord().width(), TileWidth::new(300 - 256).unwrap());
        assert!(partial.is_partial());
        for (i, h) in partial.hashes().iter().enumerate() {
            assert_eq!(*h, leaves[256 + i]);
        }
    }

    #[test]
    fn level1_tile_hash_matches_tree_subtree_hash() {
        // A level-1 tile hash is the root of a complete 256-leaf subtree; it must
        // equal the tree's own node hash for that range (agreement at tile
        // boundaries).
        let n = 600;
        let tree = tree_of(n);
        let leaves = leaf_hashes(n);
        let tiles = build_tiles::<Sha256Hasher>(&leaves);
        let level1 = tiles
            .iter()
            .find(|t| t.coord().level() == TileLevel(1))
            .unwrap();
        // floor(600/256) = 2 complete 256-leaf subtrees -> width 2.
        assert_eq!(level1.coord().width(), TileWidth::new(2).unwrap());
        for (j, h) in level1.hashes().iter().enumerate() {
            let start = (j as u64) * 256;
            let expected = tree.subtree_hash(Index(start), Index(start + 256)).unwrap();
            assert_eq!(*h, expected, "level-1 hash {j} disagrees with tree node");
        }
    }

    #[test]
    fn reconstruct_root_matches_tree_root_1000() {
        // The demo case: 1000-leaf tree, root rebuilt from tiles alone.
        let n = 1000;
        let tree = tree_of(n);
        let leaves = leaf_hashes(n);
        let tiles = build_tiles::<Sha256Hasher>(&leaves);
        assert_eq!(reconstruct_root::<Sha256Hasher>(&tiles), Some(tree.root()));
    }

    #[test]
    fn to_bytes_agrees_with_tls_serialize() {
        let leaves = leaf_hashes(300);
        for tile in build_tiles::<Sha256Hasher>(&leaves) {
            assert_eq!(tile.to_bytes(), tile.tls_serialize_to_vec().unwrap());
        }
    }

    #[test]
    fn from_bytes_rejects_truncated_and_oversized() {
        let leaves = leaf_hashes(256);
        let tile = build_tiles::<Sha256Hasher>(&leaves)
            .into_iter()
            .find(|t| t.coord().level() == TileLevel(0))
            .unwrap();
        let bytes = tile.to_bytes();
        // Exact bytes round-trip.
        assert_eq!(Tile::from_bytes(tile.coord(), &bytes).unwrap(), tile);
        // One byte short -> EOF, never a panic.
        assert!(matches!(
            Tile::from_bytes(tile.coord(), &bytes[..bytes.len() - 1]),
            Err(WireError::UnexpectedEof { .. }),
        ));
        // One byte extra -> trailing bytes.
        let mut oversized = tile.to_bytes();
        oversized.push(0);
        assert!(matches!(
            Tile::from_bytes(tile.coord(), &oversized),
            Err(WireError::TrailingBytes { .. }),
        ));
    }

    // Each level-(l+1) hash equals the MTH of the corresponding full level-l
    // tile's 256 hashes: the whole tile set is internally consistent, so higher
    // tiles genuinely derive from lower ones (exercises tiles above level 0).
    fn assert_parent_consistency(tiles: &[Tile]) {
        for parent in tiles {
            let l = parent.coord().level().0;
            if l == 0 {
                continue;
            }
            for (slot, parent_hash) in parent.hashes().iter().enumerate() {
                // Child hash index j at level l-1 = parent hash global index * 256.
                let parent_hash_index =
                    parent.coord().index().0 * u64::from(FULL_TILE_HASHES) + slot as u64;
                let child_index = parent_hash_index; // level l-1 hash index of first child
                let child_tile_index = child_index; // tile (l-1, child_index) holds 256 children
                let child = tiles
                    .iter()
                    .find(|t| {
                        t.coord().level() == TileLevel(l - 1)
                            && t.coord().index() == TileIndex(child_tile_index)
                    })
                    .expect("child tile present for a parent hash");
                // A parent hash always sits above a *full* child tile.
                assert!(child.coord().width().is_full());
                let combined = combine_subtree_roots::<Sha256Hasher>(child.hashes());
                assert_eq!(*parent_hash, combined);
            }
        }
    }

    #[test]
    fn tile_set_is_internally_consistent_across_levels() {
        for n in [256u64, 257, 512, 600, 65536, 65537, 70000] {
            let tiles = build_tiles::<Sha256Hasher>(&leaf_hashes(n));
            assert_parent_consistency(&tiles);
        }
    }

    proptest! {
        // Spec section 28 / acceptance criterion: for any tree size, rebuilding
        // the root from the full tile set reproduces the tree's own root, and
        // every tile round-trips through its bytes given its coordinate. Small
        // sizes straddle the partial-tile boundary (255/256/257) explicitly.
        #[test]
        fn tiles_round_trip_and_reconstruct_root(n in 1u64..1100) {
            let tree = tree_of(n);
            let leaves = leaf_hashes(n);
            let tiles = build_tiles::<Sha256Hasher>(&leaves);

            // Root reconstruction from tiles alone.
            prop_assert_eq!(reconstruct_root::<Sha256Hasher>(&tiles), Some(tree.root()));

            // Byte round-trip for every tile, and each tile's width invariant.
            for tile in &tiles {
                let bytes = tile.to_bytes();
                prop_assert_eq!(bytes.len(), tile.coord().width().as_usize() * 32);
                let parsed = Tile::from_bytes(tile.coord(), &bytes).unwrap();
                prop_assert_eq!(&parsed, tile);
                prop_assert_eq!(tile.hashes().len(), tile.coord().width().as_usize());
            }
        }

        // The partial-tile boundary: sizes just below/at/above a full tile put
        // the right-edge tile at width n (n<256), 256 (full), and 1 (n=257 ->
        // level-0 tile 1 width 1). Never panics; widths are exact.
        #[test]
        fn partial_right_edge_widths_are_exact(n in 1u64..300) {
            let tiles = build_tiles::<Sha256Hasher>(&leaf_hashes(n));
            let level0: Vec<_> = tiles
                .iter()
                .filter(|t| t.coord().level() == TileLevel(0))
                .collect();
            // Number of level-0 tiles = ceil(n/256).
            let expected_tiles = n.div_ceil(256);
            prop_assert_eq!(level0.len() as u64, expected_tiles);
            // The last level-0 tile carries the remainder; earlier ones are full.
            for tile in &level0 {
                let idx = tile.coord().index().0;
                if idx + 1 < expected_tiles {
                    prop_assert!(tile.coord().width().is_full());
                } else {
                    let rem = n - idx * 256;
                    let expected = if rem == 0 { 256 } else { rem };
                    prop_assert_eq!(u64::from(tile.coord().width().get()), expected);
                }
            }
        }
    }
}
