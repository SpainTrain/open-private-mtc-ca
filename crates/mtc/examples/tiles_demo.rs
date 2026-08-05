//! Tile demo (ticket `mtclib-tiles`).
//!
//! Builds a 1000-leaf issuance-log Merkle tree, emits its full `tlog-tiles` tile
//! set (with each tile's canonical path and byte length), reconstructs the tree
//! root from the tiles alone, and lists the tiles an inclusion proof for a
//! sample leaf needs. The output is deterministic: every run prints the same
//! root.
//!
//! ```console
//! $ cargo run -p mtc --example tiles_demo
//! ```

use mtc::{
    build_tiles, hash_leaf, reconstruct_root, tiles_for_inclusion, Index, MerkleTree, Sha256Hasher,
    TreeSize,
};

fn main() {
    let n: u64 = 1000;

    // Build the tree and, independently, its leaf hashes (both deterministic).
    let mut tree: MerkleTree = MerkleTree::new();
    let mut leaves = Vec::new();
    for i in 0..n {
        let entry = format!("entry-{i}");
        tree.append(entry.as_bytes());
        leaves.push(hash_leaf::<Sha256Hasher>(entry.as_bytes()));
    }

    println!("tree size : {}", tree.len().0);
    println!("root hash : {:?}", tree.root());

    // Emit the full tile set (all levels), in level-then-index order.
    let tiles = build_tiles::<Sha256Hasher>(&leaves);
    println!("\n{} tiles for {n} leaves:", tiles.len());
    for tile in &tiles {
        let coord = tile.coord();
        let kind = if coord.is_partial() {
            "partial"
        } else {
            "full"
        };
        println!(
            "  {:<16} level {} index {:>3} width {:>3}  {:>4} bytes ({kind})",
            coord.path(),
            coord.level().0,
            coord.index().0,
            coord.width().get(),
            tile.to_bytes().len(),
        );
    }

    // Reconstruct the root from the tiles alone and confirm it matches.
    match reconstruct_root::<Sha256Hasher>(&tiles) {
        Some(root) if root == tree.root() => {
            println!("\nroot reconstructed from tiles alone matches: {root:?}");
        }
        other => {
            println!("\nRECONSTRUCTION MISMATCH: {other:?}");
        }
    }

    // Which tiles a proof server fetches to serve an inclusion proof for a leaf.
    let leaf = Index(613);
    match tiles_for_inclusion(leaf, TreeSize(n)) {
        Ok(needed) => {
            println!(
                "\ninclusion path for leaf {} needs {} tile(s):",
                leaf.0,
                needed.len(),
            );
            for coord in &needed {
                println!("  {}", coord.path());
            }
        }
        Err(err) => println!("\ninclusion path error: {err}"),
    }
}
