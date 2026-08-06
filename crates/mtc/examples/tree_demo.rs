//! Deterministic Merkle-tree demo (ticket `tree-primitives`).
//!
//! Builds a 1000-leaf issuance-log Merkle tree by appending real `LogEntry`
//! leaves the way the CA write path does — each leaf enters the tree as the
//! framed `LeafBytes` a relying party reconstructs — and prints its root hash,
//! the empty-tree root, and a sample range decomposition. The root is
//! deterministic: every run prints the same value.
//!
//! ```console
//! $ cargo run -p mtc --example tree_demo
//! ```

use std::error::Error;

use mtc::{
    decompose_range, HashOutput, Index, LogEntry, MerkleTree, SubjectInfoHash, SubjectType,
    TbsCertificateLogEntry, SHA256_EMPTY_ROOT,
};

/// A distinct certificate log entry for index `i` (the subject-info hash encodes
/// `i`, so every leaf differs) — a stand-in for what the CA commits per
/// issuance. Leaves can only enter the tree through a `LogEntry`, which is what
/// keeps the committed framing identical to the read-path reconstruction.
fn log_entry_for(i: u64) -> LogEntry {
    let mut subject_info_hash = [0u8; 32];
    subject_info_hash[..8].copy_from_slice(&i.to_be_bytes());
    LogEntry::certificate(
        TbsCertificateLogEntry::builder()
            .subject_type(SubjectType::Tls)
            .subject_info_hash(SubjectInfoHash::from_hash(HashOutput(subject_info_hash)))
            .build(),
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    // Empty-tree root (RFC 9162 §2.1.1 HASH(); spec section 19.6).
    println!("empty-tree root : {SHA256_EMPTY_ROOT:?}");

    // Build a 1000-leaf tree exactly as the CA write path does: append the
    // framed `LeafBytes` of each `LogEntry`. The bare type `MerkleTree` resolves
    // to `MerkleTree<Sha256Hasher>` via the default type parameter.
    let mut tree: MerkleTree = MerkleTree::with_capacity(1000);
    for i in 0..1000u64 {
        tree.append(&log_entry_for(i).leaf_bytes()?);
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

    Ok(())
}
