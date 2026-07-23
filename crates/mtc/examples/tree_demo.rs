//! Deterministic Merkle-tree demo (ticket `tree-primitives`).
//!
//! Builds a 1000-leaf issuance-log Merkle tree from a fixed entry sequence and
//! prints its root hash, the empty-tree root, and a sample range decomposition.
//! The root is deterministic: every run prints the same value.
//!
//! ```console
//! $ cargo run -p mtc --example tree_demo
//! ```

use mtc::{decompose_range, Index, MerkleTree, SHA256_EMPTY_ROOT};

fn main() {
    // Empty-tree root (RFC 9162 §2.1.1 HASH(); spec section 19.6).
    println!("empty-tree root : {SHA256_EMPTY_ROOT:?}");

    // Build a 1000-leaf tree from a fixed sequence of entries. The bare type
    // `MerkleTree` resolves to `MerkleTree<Sha256Hasher>` via the default type
    // parameter.
    let mut tree: MerkleTree = MerkleTree::with_capacity(1000);
    for i in 0..1000u64 {
        let entry = format!("entry-{i}");
        tree.append(entry.as_bytes());
    }

    println!("tree size       : {}", tree.len().0);
    println!("root hash       : {:?}", tree.root());

    // Show the canonical subtree decomposition of an arbitrary range.
    let (start, end) = (Index(3), Index(1000));
    let blocks = decompose_range(start, end);
    println!(
        "range [{}, {}) -> {} aligned subtrees:",
        start.0,
        end.0,
        blocks.len(),
    );
    for block in &blocks {
        // Every block is in range and aligned, so subtree_hash is always Some;
        // fall back to the empty root rather than unwrap/expect (denied off
        // the test path) so the demo cannot panic.
        let hash = tree
            .subtree_hash(block.start(), block.end())
            .unwrap_or(SHA256_EMPTY_ROOT);
        println!(
            "  [{:>4}, {:>4})  width {:>4}  hash {hash:?}",
            block.start().0,
            block.end().0,
            block.len(),
        );
    }
}
