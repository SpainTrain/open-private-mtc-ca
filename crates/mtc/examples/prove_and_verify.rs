//! Issues an inclusion proof for a random leaf and verifies it, then does the
//! same for a consistency proof between two checkpoints (ticket
//! `mtclib-inclusion-proofs`).
//!
//! ```console
//! $ cargo run -p mtc --example prove_and_verify
//! ```

use mtc::{hash_leaf, ConsistencyProof, InclusionProof, Index, MerkleTree, Sha256Hasher, TreeSize};
use rand_core::{OsRng, RngCore};

fn main() -> Result<(), mtc::ProofError> {
    // Build a log of a few thousand entries.
    const SIZE: u64 = 5_000;
    let mut tree: MerkleTree = MerkleTree::with_capacity(5_000);
    for i in 0..SIZE {
        tree.append(format!("entry-{i}").as_bytes());
    }
    let root = tree.root();
    println!("log size        : {}", tree.len().0);
    println!("checkpoint root : {root:?}");

    // --- Inclusion proof for a random leaf ---------------------------------
    let index = OsRng.next_u64() % SIZE;
    let proof = InclusionProof::generate(&tree, Index(index))?;
    let leaf = hash_leaf::<Sha256Hasher>(format!("entry-{index}").as_bytes());

    println!("\ninclusion proof for leaf {index}");
    println!(
        "  audit path    : {} sibling hashes",
        proof.audit_path().len()
    );
    proof.verify::<Sha256Hasher>(&leaf, &root)?;
    println!("  verification  : OK");

    // A tampered proof must be rejected, not accepted, and not panic.
    let mut tampered_path = proof.audit_path().to_vec();
    if let Some(first) = tampered_path.first_mut() {
        first.0[0] ^= 0x01;
    }
    let tampered = InclusionProof::from_parts(proof.tree_size(), proof.leaf_index(), tampered_path);
    match tampered.verify::<Sha256Hasher>(&leaf, &root) {
        Ok(()) => return Err(mtc::ProofError::RootMismatch), // never happens
        Err(err) => println!("  tampered      : rejected ({err})"),
    }

    // --- Consistency proof between an old checkpoint and the current one ----
    let old_size = OsRng.next_u64() % SIZE;
    let old_root = if old_size == 0 {
        mtc::empty_root::<Sha256Hasher>()
    } else {
        // The size-`old_size` root is the subtree hash over the first
        // `old_size` leaves of the current tree.
        tree.subtree_hash(Index(0), Index(old_size)).unwrap_or(root)
    };
    let consistency = ConsistencyProof::generate(&tree, TreeSize(old_size), tree.len())?;
    println!("\nconsistency proof {old_size} -> {SIZE}");
    println!("  path          : {} node hashes", consistency.path().len());
    consistency.verify::<Sha256Hasher>(&old_root, &root)?;
    println!("  verification  : OK");

    Ok(())
}
