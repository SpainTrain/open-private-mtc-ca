//! End-to-end conformance vector for the write-path leaf-framing invariant
//! (crypto audit 2026-08-05, Finding 2; ticket mtc-qka.10).
//!
//! This test builds the issuance-log Merkle tree **exactly as the CA write path
//! will** — for each index it obtains a [`LeafBytes`] from a [`LogEntry`] via
//! [`LogEntry::leaf_bytes`] and hands it to [`MerkleTree::append`] — and then
//! runs the relying-party read path against it: [`InclusionProof::generate`] +
//! [`InclusionProof::verify`] using [`LogEntry::leaf_hash`]. It pins that
//! **write-path committed framing == read-path reconstruction**, byte for byte,
//! across both entry discriminants (`00 00` null / `00 01…` certificate) and an
//! abandoned-batch gap filled with `null_entry`.
//!
//! It is the regression that would have caught the raw-`&[u8]` trap the audit
//! found: appending the bare TBS bytes (dropping the `00 01` frame) yields a
//! leaf hash that verification rejects. Under the new API that mistake does not
//! even typecheck (`tests/compile_fail/append_raw_bytes.rs`); here we show that
//! *were* it possible, the read path would reject it — i.e. the frame is
//! load-bearing, exactly as the audit oracle demonstrated.

// Test-only crate: `unwrap`/`expect` are the ergonomic choice here and the
// no-unwrap-in-prod rule exempts tests (matches tests/serialization.rs).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mtc::{
    hash_leaf, null_entry, HashOutput, InclusionProof, Index, LogEntry, MerkleTree, Sha256Hasher,
    SubjectInfoHash, SubjectType, TbsCertificateLogEntry, TlsSerialize,
};

/// A distinct certificate entry for index `i` (the subject-info hash encodes
/// `i`, so no two leaves coincide).
fn cert_entry(i: u64) -> TbsCertificateLogEntry {
    let mut subject_info_hash = [0u8; 32];
    subject_info_hash[..8].copy_from_slice(&i.to_be_bytes());
    TbsCertificateLogEntry::builder()
        .subject_type(SubjectType::Tls)
        .subject_info_hash(SubjectInfoHash::from_hash(HashOutput(subject_info_hash)))
        .build()
}

/// The fixed conformance sequence: mostly certificate entries with two
/// `null_entry` gaps at indices 3 and 5 — the shape the write path produces when
/// a batch is abandoned (spec §11.2: "abandoned indices become permanent gaps
/// filled with `null_entry`").
fn conformance_log() -> Vec<LogEntry> {
    (0..8u64)
        .map(|i| {
            if i == 3 || i == 5 {
                null_entry()
            } else {
                LogEntry::certificate(cert_entry(i))
            }
        })
        .collect()
}

/// Builds the tree the way the CA write path does: append each entry's
/// `LeafBytes`. Returns the tree and the entries so the read path can be checked
/// against the same values.
fn build_log() -> (MerkleTree, Vec<LogEntry>) {
    let entries = conformance_log();
    let mut tree = MerkleTree::new();
    for (i, entry) in entries.iter().enumerate() {
        let leaf = entry
            .leaf_bytes()
            .expect("conformance entries always encode");
        let index = tree.append(&leaf);
        assert_eq!(index, Index(i as u64), "append assigns sequential indices");
    }
    (tree, entries)
}

/// The crux: for every leaf, the hash the *tree committed* on the write path is
/// byte-identical to the hash a relying party *reconstructs* from the entry on
/// the read path, and an inclusion proof verifies against the committed root.
#[test]
fn write_path_framing_equals_read_path_reconstruction() {
    let (tree, entries) = build_log();
    let root = tree.root();

    for (i, entry) in entries.iter().enumerate() {
        let index = Index(i as u64);

        // Read-path reconstruction: leaf = HASH(0x00 || LogEntry::tls_serialize).
        let reconstructed = entry.leaf_hash::<Sha256Hasher>().expect("entry encodes");

        // Write-path commitment: what the tree actually stored for this leaf.
        let committed = tree.leaf_hash(index).expect("index is within the tree");

        assert_eq!(
            committed, reconstructed,
            "committed leaf {i} must equal the relying-party reconstruction",
        );

        // Run the read path end to end: generate and verify an inclusion proof
        // with the reconstructed leaf hash against the committed checkpoint root.
        let proof = InclusionProof::generate::<Sha256Hasher>(&tree, index).expect("index in range");
        proof
            .verify::<Sha256Hasher>(&reconstructed, &root)
            .expect("inclusion proof verifies for the framed leaf");
    }
}

/// The exact trap from the audit oracle: appending the bare `TBSCertificateLogEntry`
/// bytes (dropping the `00 01` entry-type discriminant) commits the *wrong* leaf,
/// which the read path rejects. Under the typed API this preimage can no longer
/// reach `append` at all (see the compile-fail test); here we prove the frame is
/// what makes the leaf verify.
#[test]
fn dropping_the_entry_discriminant_would_break_verification() {
    let (tree, entries) = build_log();
    let root = tree.root();

    // Pick a certificate leaf (index 0) and forge the "raw TBS" leaf hash the
    // buggy write path would have produced.
    let index = Index(0);
    let LogEntry::Certificate(tbs) = &entries[0] else {
        panic!("index 0 is a certificate entry");
    };
    let raw_tbs_bytes = tbs.tls_serialize_to_vec().expect("tbs encodes");
    let trap_leaf_hash = hash_leaf::<Sha256Hasher>(&raw_tbs_bytes);

    // The framed commitment differs from the un-framed one: the discriminant is
    // part of the preimage.
    let committed = tree.leaf_hash(index).expect("in range");
    assert_ne!(
        committed, trap_leaf_hash,
        "the 00 01 discriminant must change the leaf hash",
    );

    // And the read path rejects the un-framed leaf against the real root — the
    // "silently fails all RP verification" outcome, now surfaced as an error.
    let proof = InclusionProof::generate::<Sha256Hasher>(&tree, index).expect("in range");
    let verdict = proof.verify::<Sha256Hasher>(&trap_leaf_hash, &root);
    assert!(
        verdict.is_err(),
        "inclusion of the un-framed (raw TBS) leaf must NOT verify",
    );
    // For contrast, the correctly framed leaf verifies.
    proof
        .verify::<Sha256Hasher>(&committed, &root)
        .expect("the framed leaf verifies");
}

/// Locks the on-wire framing bytes and hashes as a fixed vector, tying this
/// end-to-end test to the entry-layer known-answer values (draft-03 §5.3).
#[test]
fn framing_known_answer_vector() {
    // null_entry: type = null_entry(0), Empty body -> exactly `00 00`.
    let null_leaf = null_entry().leaf_bytes().expect("null encodes");
    assert_eq!(null_leaf.as_bytes(), &[0x00, 0x00]);
    // Its leaf hash is the value locked by the entry-layer KAT.
    assert_eq!(
        format!("{:?}", hash_leaf::<Sha256Hasher>(null_leaf.as_bytes())),
        "HashOutput(709e80c88487a2411e1ee4dfb9f22a861492d20c4765150c0c794abd70f8147c)",
    );

    // A certificate entry: type = tbs_cert_entry(1) -> framing starts `00 01`.
    let cert = LogEntry::certificate(cert_entry(0));
    let cert_leaf = cert.leaf_bytes().expect("cert encodes");
    assert_eq!(&cert_leaf.as_bytes()[0..2], &[0x00, 0x01]);

    // The framed leaf preimage is `hash_leaf`'s input, so leaf_bytes and
    // leaf_hash are the same serialization by construction.
    assert_eq!(
        hash_leaf::<Sha256Hasher>(cert_leaf.as_bytes()),
        cert.leaf_hash::<Sha256Hasher>().expect("cert encodes"),
    );
}
