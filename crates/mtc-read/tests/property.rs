//! Property test for [`mtc_read::plan_inclusion`] (spec §19.2, §12.2 step 3).
//!
//! For arbitrary tree size `N` and leaf `i < N`, the hashes extracted from real
//! `mtc`-built tiles per the plan — and combined by the documented rule — reproduce
//! **exactly** the inclusion proof `mtc::InclusionProof::generate` computes
//! directly. This is the acceptance criterion binding the planner to the proof math.

// Integration-test helpers sit outside #[test] fns, so the allow-unwrap-in-tests
// exemption does not reach them (documented scoped-allow pattern,
// docs/lint-policy.md deviation 1).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use mtc::{
    build_tiles, hash_leaf, hash_node, tiles_for_inclusion, HashOutput, InclusionProof, Index,
    MerkleTree, Sha256Hasher, TileCoord, TreeSize,
};
use mtc_read::{plan_inclusion, PathStep, TilePlan, TileSlotRun};
use proptest::prelude::*;

/// The per-tile hashes of a tree, addressed by coordinate.
type TileMap = HashMap<TileCoord, Vec<HashOutput>>;

/// Entry bytes for leaf `i` (any deterministic mapping works; the tree and the
/// tiles must agree, which they do by using this for both).
fn entry_bytes(i: u64) -> Vec<u8> {
    format!("entry-{i}").into_bytes()
}

/// Builds a tree of `n` entries, its leaf hashes, and a coordinate->hashes tile
/// map (via `mtc::build_tiles`), all from the same entry bytes.
fn build(n: u64) -> (MerkleTree<Sha256Hasher>, Vec<HashOutput>, TileMap) {
    let mut tree = MerkleTree::<Sha256Hasher>::new();
    let mut leaves = Vec::with_capacity(usize::try_from(n).unwrap());
    for i in 0..n {
        let bytes = entry_bytes(i);
        tree.append(&bytes);
        leaves.push(hash_leaf::<Sha256Hasher>(&bytes));
    }
    let mut map = TileMap::new();
    for tile in build_tiles::<Sha256Hasher>(&leaves) {
        map.insert(tile.coord(), tile.hashes().to_vec());
    }
    (tree, leaves, map)
}

/// The balanced Merkle Tree Hash of a non-empty slice of subtree roots (RFC 9162
/// §2.1.1), reproducing `mtc`'s `combine_subtree_roots` for a complete aligned
/// block's slot-run.
fn balanced_mth(hs: &[HashOutput]) -> HashOutput {
    if hs.len() == 1 {
        return hs[0];
    }
    let k = 1usize << (hs.len() - 1).ilog2(); // largest power of two < len
    hash_node::<Sha256Hasher>(&balanced_mth(&hs[..k]), &balanced_mth(&hs[k..]))
}

/// The root of one complete-subtree block: the balanced MTH of its slot-run.
fn block_hash(map: &TileMap, run: &TileSlotRun) -> HashOutput {
    let hashes = map.get(&run.coord()).expect("plan references a real tile");
    let s = usize::from(run.slot());
    let l = usize::from(run.slot_count());
    balanced_mth(&hashes[s..s + l])
}

/// One audit-path sibling: the right-leaning combination of its block roots
/// (blocks ascending by leaf, strictly decreasing in size).
fn sibling_hash(map: &TileMap, step: &PathStep) -> HashOutput {
    let mut blocks = step.blocks().iter().rev();
    let mut acc = block_hash(map, blocks.next().expect("a step has >= 1 block"));
    for block in blocks {
        acc = hash_node::<Sha256Hasher>(&block_hash(map, block), &acc);
    }
    acc
}

/// Assembles the audit path from the plan and the tiles.
fn assemble_path(map: &TileMap, plan: &TilePlan) -> Vec<HashOutput> {
    plan.steps().iter().map(|s| sibling_hash(map, s)).collect()
}

/// Asserts the plan for `(n, i)` reproduces `mtc`'s proof exactly: the assembled
/// audit path equals the generated proof's, the leaf slot holds the leaf hash,
/// and the tile set equals `mtc::tiles_for_inclusion`.
fn assert_plan_reproduces_proof(
    n: u64,
    i: u64,
    tree: &MerkleTree<Sha256Hasher>,
    leaves: &[HashOutput],
    map: &TileMap,
) {
    let plan = plan_inclusion(TreeSize(n), Index(i)).unwrap();
    let proof = InclusionProof::generate(tree, Index(i)).unwrap();

    // (1) The extracted audit path matches the generated proof bit-for-bit.
    let assembled = assemble_path(map, &plan);
    assert_eq!(
        assembled,
        proof.audit_path(),
        "assembled audit path != mtc proof for n={n}, i={i}",
    );

    // (2) The leaf slot holds exactly this leaf's hash.
    assert_eq!(
        block_hash(map, &plan.leaf()),
        leaves[usize::try_from(i).unwrap()],
        "leaf slot hash mismatch for n={n}, i={i}",
    );

    // (3) The tile set equals the canonical mtc set.
    assert_eq!(
        plan.tiles(),
        tiles_for_inclusion(Index(i), TreeSize(n)).unwrap(),
        "plan.tiles() != tiles_for_inclusion for n={n}, i={i}",
    );

    // (4) The assembled proof, re-wrapped, verifies against the real root — the
    // end-to-end guarantee a proof server relies on.
    let rebuilt = InclusionProof::from_parts(TreeSize(n), Index(i), assembled);
    rebuilt
        .verify::<Sha256Hasher>(&leaves[usize::try_from(i).unwrap()], &tree.root())
        .unwrap_or_else(|e| panic!("assembled proof fails to verify for n={n}, i={i}: {e}"));
}

proptest! {
    // Bounded case count so the debug-build `cargo test` gate stays fast; the
    // planner is size-driven, so this range plus the deterministic multi-level
    // case below covers the acceptance criterion's ≤ 2^20 span.
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    // Spec §19.2 / §12.2 step 3 acceptance criterion: for arbitrary N and i < N,
    // hashes extracted per the plan reproduce mtc's inclusion proof exactly.
    #[test]
    fn plan_reproduces_proof(n in 1u64..2048, seed in any::<u64>()) {
        let (tree, leaves, map) = build(n);
        let i = seed % n;
        assert_plan_reproduces_proof(n, i, &tree, &leaves, &map);
    }
}

#[test]
fn boundary_sizes_reproduce_proof() {
    // Deterministic coverage of the level-0/level-1 tile boundaries (the
    // 255/256/257 seam, partial right-edge tiles) at spread indices.
    for &n in &[1u64, 2, 255, 256, 257, 511, 512, 513, 1000, 4096, 8192] {
        let (tree, leaves, map) = build(n);
        let mut indices = vec![0, n / 2, n - 1];
        for probe in [1u64, 255, 256, 257, 511, 512, 4096] {
            if probe < n {
                indices.push(probe);
            }
        }
        for i in indices {
            assert_plan_reproduces_proof(n, i, &tree, &leaves, &map);
        }
    }
}

#[test]
fn level_two_tiles_reproduce_proof() {
    // Exercise level-2 tiles (the third tile level, appearing anywhere in the
    // acceptance criterion's ≤ 2^20 range). Two shapes reach a level-2 tile:
    //
    //   * N = 65537, leaf 65536: the tree splits at 65536, so the sibling is the
    //     complete 2^16-leaf subtree [0, 65536) — a single level-2 hash (tile
    //     (2,0), width 1).
    //   * N = 131072, leaf 0: the top sibling is the complete 2^16-leaf subtree
    //     [65536, 131072) — a level-2 hash.
    //
    // The tile-level structure and partial-tile geometry across [2^16, 2^20) are
    // identical to these; a literal 2^20 build is omitted only for debug-build
    // test speed (the geometry is level-count-bounded, not size-dependent).
    let (tree, leaves, map) = build(65_537);
    for i in [0u64, 1, 255, 256, 65_535, 65_536] {
        assert_plan_reproduces_proof(65_537, i, &tree, &leaves, &map);
    }
    // Confirm a level-2 tile is genuinely referenced (not just present).
    let plan = plan_inclusion(TreeSize(65_537), Index(65_536)).unwrap();
    assert!(
        plan.tiles().iter().any(|t| t.level() == mtc::TileLevel(2)),
        "leaf 65536 of a 65537-leaf tree must read a level-2 tile",
    );

    let (tree2, leaves2, map2) = build(131_072);
    for i in [0u64, 65_536, 131_071] {
        assert_plan_reproduces_proof(131_072, i, &tree2, &leaves2, &map2);
    }
}
