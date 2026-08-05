//! Relying-party verification core for Merkle Tree Certificates: spec §12.1
//! steps 4-6.
//!
//! A relying party validating an MTC certificate runs the seven-step flow of
//! spec §12.1. Steps 1-3 (parse the certificate, decode the [`MTCProof`], check
//! revocation) and step 7 (X.509 path validation) are the certificate layer
//! (ticket `read-verify-cert`). This crate is the cryptographic core in the
//! middle — steps **4-6**:
//!
//! ```text
//! 4. Reconstruct leaf hash from the log entry (TBSCertificate)
//! 5. Apply the inclusion proof to compute the expected subtree root
//! 6. Verify the CA's checkpoint signature over that subtree
//!    (NO cosigner signatures required — spec §1)
//! ```
//!
//! [`verify_inclusion`] performs all three and returns a [`Verified`] witness on
//! success, or a typed [`VerifyError`] naming exactly which step failed and why.
//!
//! # Scope — what the caller MUST still check (read this before embedding)
//!
//! [`verify_inclusion`] proves the entry is in *a* tree whose `(tree_size,
//! root_hash)` this CA key signed. It deliberately does **not** bind two fields
//! of the checkpoint, which remain the caller's responsibility:
//!
//! - **`log_id` / trust anchor is NOT bound.** A checkpoint signed by the same
//!   CA key for a *different* log, over the same root, will verify here. The
//!   `log_id` / trust-anchor binding is part of the certificate layer (spec
//!   §12.1 steps 1-3, 7; ticket `read-verify-cert`), not steps 4-6. So the
//!   caller MUST confirm the checkpoint belongs to the expected log —
//!   [`Verified::log_id`] returns the checkpoint's `log_id` precisely so callers
//!   can perform that binding.
//! - **`signed_at` is unauthenticated.** The checkpoint timestamp is not part of
//!   the signed `MTCSubtreeSignatureInput` (draft §5.4.1), so it must **not** be
//!   used for freshness/recency decisions. Freshness comes from the landmark /
//!   checkpoint-distribution layer, not from this signature.
//!
//! # v1 algorithms (spec §4, §14.1)
//!
//! The tree hash is SHA-256 ([`mtc::Sha256Hasher`]) and the checkpoint signature
//! is ECDSA P-256 ([`mtc::EcdsaP256`]). ML-DSA (v2) is feature-gated in the core
//! `mtc` crate and out of scope here.
//!
//! # Minimal dependencies (embeddable in relying parties)
//!
//! This crate depends only on the core [`mtc`] domain crate — no storage, no
//! service framework, no async runtime — so it links into relying-party code
//! (agents, CLIs, embedded verifiers) with a small footprint. Every hash is the
//! domain-separated leaf/interior construction of [`mtc::tree`] (spec §19.2);
//! there is no second hashing path.
//!
//! # Never panics on adversarial input (spec §19.8)
//!
//! Tampered proofs, truncated hashes, out-of-range indices, an absurd
//! `tree_size`, a wrong key, or a mangled signature all return a
//! [`VerifyError`], never a panic. Each variant maps to a stable
//! [`reason`](VerifyError::reason) label for the §20.2 verification-failure
//! breakdown telemetry.
//!
//! # Example
//!
//! ```
//! use mtc::{
//!     CheckpointBuilder, EcdsaP256, HashOutput, Index, InclusionProof, LogEntry,
//!     LogId, MerkleTree, Sha256Hasher, SignedAt, TreeSize,
//! };
//! use mtc_verify::verify_inclusion;
//!
//! // A log of a few certificate entries. The tree commits to `hash_leaf` of
//! // each entry's serialized bytes, exactly what `LogEntry::leaf_hash` computes.
//! let entries: Vec<LogEntry> = (0..5u8)
//!     .map(|i| LogEntry::certificate(sample_entry(i)))
//!     .collect();
//! let mut tree = MerkleTree::<Sha256Hasher>::new();
//! for entry in &entries {
//!     tree.append(&entry.tls_serialize_to_vec()?);
//! }
//!
//! // The CA signs a checkpoint over the current root.
//! let (signing_key, ca_pubkey) = EcdsaP256::generate_keypair();
//! let checkpoint = CheckpointBuilder::new(LogId::new("demo-log")?)
//!     .root_hash(tree.root())
//!     .tree_size(tree.len())
//!     .signed_at(SignedAt(0))
//!     .build()
//!     .sign(&EcdsaP256, &signing_key)?;
//!
//! // A proof server produces an inclusion proof for entry 3.
//! let index = Index(3);
//! let proof = InclusionProof::generate(&tree, index)?;
//!
//! // The relying party verifies steps 4-6 end to end.
//! let verified = verify_inclusion(&entries[3], &proof, &checkpoint, &ca_pubkey)?;
//! assert_eq!(verified.leaf_index(), index);
//! assert_eq!(verified.root_hash(), tree.root());
//! // The caller must still confirm this is the expected log (log_id is not
//! // bound by verify_inclusion — see the "Scope" note).
//! assert_eq!(verified.log_id().as_str(), "demo-log");
//! # use mtc::{Claim, DnsName, SubjectInfoHash, SubjectType, TbsCertificateLogEntry, TlsSerialize};
//! # fn sample_entry(i: u8) -> TbsCertificateLogEntry {
//! #     TbsCertificateLogEntry::builder()
//! #         .subject_type(SubjectType::Tls)
//! #         .subject_info_hash(SubjectInfoHash::from_hash(HashOutput([i; 32])))
//! #         .claim(Claim::dns(vec![DnsName::new(b"example.com".to_vec()).unwrap()]).unwrap())
//! #         .build()
//! # }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use thiserror::Error;

use mtc::{
    Checkpoint, EcdsaP256, HashOutput, InclusionProof, Index, LogEntry, LogId, ProofError,
    Sha256Hasher, Signed, TreeSize, VerifyingKey,
};

/// Proof that an entry's inclusion in a signed checkpoint was fully verified
/// (spec §12.1 steps 4-6).
///
/// Returned by [`verify_inclusion`] only after the leaf hash was reconstructed,
/// the inclusion proof reconstructed the checkpoint's committed root, and the
/// CA's checkpoint signature verified. Holding a `Verified` is therefore
/// evidence all three checks passed; it carries the committed facts the caller
/// can rely on downstream (the checkpoint's `log_id`, the tree size, the leaf
/// index, and the root the checkpoint committed to).
///
/// The [`log_id`](Self::log_id) is surfaced so the caller can complete the
/// binding this core does not do (see the crate-level "Scope" note): the
/// signature covers `log_id`, but `verify_inclusion` does not check that it is
/// the *expected* log, so the caller must compare [`Verified::log_id`] against
/// the log it expected.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "a Verified witness records a successful verification; dropping it discards the result"]
pub struct Verified {
    log_id: LogId,
    tree_size: TreeSize,
    leaf_index: Index,
    root_hash: HashOutput,
}

impl Verified {
    /// The `log_id` (trust-anchor id) the verified checkpoint commits to.
    ///
    /// `verify_inclusion` proves the signature covers this `log_id` but does
    /// **not** check it is the log the caller expected — comparing it to the
    /// expected log is the caller's binding step (crate-level "Scope" note;
    /// ticket `read-verify-cert`).
    #[must_use]
    pub const fn log_id(&self) -> &LogId {
        &self.log_id
    }

    /// The tree size the verified checkpoint commits to (equal to the proof's
    /// tree size — [`verify_inclusion`] rejects a mismatch).
    #[must_use]
    pub const fn tree_size(&self) -> TreeSize {
        self.tree_size
    }

    /// The leaf index whose inclusion was proven.
    #[must_use]
    pub const fn leaf_index(&self) -> Index {
        self.leaf_index
    }

    /// The Merkle root the checkpoint committed to and the proof reconstructed.
    #[must_use]
    pub const fn root_hash(&self) -> HashOutput {
        self.root_hash
    }
}

/// Why relying-party inclusion verification failed (spec §12.1 steps 4-6).
///
/// Each variant is a distinct failure reason, so a relying party can report the
/// §20.2 "verification failure breakdown by reason" without leaking anything
/// secret-dependent (every distinction here is structural, computed over public
/// data). Use [`reason`](Self::reason) for a stable telemetry label.
///
/// The enum is `#[non_exhaustive]`: future algorithm versions may add reasons,
/// so external matches need a catch-all arm.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum VerifyError {
    /// Step 4: the log entry could not be serialized to reconstruct its leaf
    /// hash (a claim body overflowing its length prefix). The entry the caller
    /// holds is not a well-formed log entry, so no leaf hash exists to prove.
    #[error("malformed log entry: cannot serialize it to reconstruct the leaf hash")]
    MalformedEntry,

    /// Step 5: the inclusion proof commits to a different tree size than the
    /// checkpoint. The proof proves membership in another tree, so it says
    /// nothing about the tree this signed checkpoint commits to. Rejecting this
    /// binds the proof to the checkpoint (a proof for a small historical tree
    /// must not be replayed against a later checkpoint's root).
    #[error(
        "size mismatch: proof commits to tree size {proof_tree_size}, \
         checkpoint to {checkpoint_tree_size}"
    )]
    SizeMismatch {
        /// The tree size the inclusion proof declares.
        proof_tree_size: u64,
        /// The tree size the signed checkpoint commits to.
        checkpoint_tree_size: u64,
    },

    /// Step 5: the proof's leaf index is not `< tree_size` — there is no such
    /// leaf to include. Reachable only from a malformed/adversarial proof; a
    /// typed error rather than a panic (spec §19.8).
    #[error("leaf index {index} is out of range for tree size {tree_size}")]
    IndexOutOfRange {
        /// The offending leaf index.
        index: u64,
        /// The tree size it was checked against.
        tree_size: u64,
    },

    /// Step 5: the proof's audit path is not the length the `(leaf_index,
    /// tree_size)` pair requires (truncated or padded). Detected before any
    /// hashing, so an overlong path never drives extra work (spec §19.8).
    #[error("malformed proof: expected {expected} audit-path element(s), found {actual}")]
    MalformedProof {
        /// The audit-path length implied by the proof's index and tree size.
        expected: usize,
        /// The audit-path length actually present.
        actual: usize,
    },

    /// Step 5: the audit path is well-formed but the root it reconstructs does
    /// not equal the checkpoint's committed root. The entry is not at this index
    /// of this tree, or the proof (or entry, or root) was tampered with.
    #[error("wrong root: the reconstructed subtree root does not match the checkpoint")]
    WrongRoot,

    /// Step 6: the checkpoint's signature did not verify under the supplied CA
    /// key. Covers a genuine bad signature, a wrong key, a malformed key or
    /// signature, an algorithm mismatch, and a checkpoint whose signed input
    /// could not be reconstructed (an over-long trust-anchor id) — all of which
    /// mean "this checkpoint is not authentically signed by this CA".
    #[error("bad signature: the checkpoint is not authentically signed by the given CA key")]
    BadSignature,
}

impl VerifyError {
    /// A stable, low-cardinality label for the §20.2 verification-failure
    /// breakdown telemetry. Stable across releases so dashboards keep working;
    /// the label omits the per-instance numbers carried by the variant fields.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::MalformedEntry => "malformed_entry",
            Self::SizeMismatch { .. } => "size_mismatch",
            Self::IndexOutOfRange { .. } => "index_out_of_range",
            Self::MalformedProof { .. } => "malformed_proof",
            Self::WrongRoot => "wrong_root",
            Self::BadSignature => "bad_signature",
        }
    }
}

/// Verifies that `entry` is included at `proof.leaf_index()` in the tree the
/// signed `checkpoint` commits to (spec §12.1 steps 4-6).
///
/// The three checks, in spec order:
///
/// 1. **Step 4 — leaf hash.** Reconstruct `HASH(0x00 || entry)` from the log
///    entry with the domain-separated [`mtc::hash_leaf`] construction (via
///    [`LogEntry::leaf_hash`]). Leaf and interior preimages stay separated, so a
///    tree node can never be presented as a leaf (spec §19.2).
/// 2. **Step 5 — apply the proof.** Require the proof and checkpoint to agree on
///    `tree_size`, then apply the inclusion proof (RFC 9162 §2.1.3.2) to the leaf
///    hash and require the reconstructed root to equal the checkpoint's committed
///    root. The proof's path length is validated against `(leaf_index,
///    tree_size)` before any hashing.
/// 3. **Step 6 — checkpoint signature.** Verify the CA's ECDSA P-256 signature
///    over the checkpoint's domain-separated `MTCSubtreeSignatureInput` under
///    `ca_pubkey`. No cosigner signatures are required (spec §1).
///
/// All three must pass; the returned [`Verified`] is evidence of that. Steps 5
/// and 6 are both required and neither branch is secret-dependent, so their
/// order does not affect the security conclusion — the spec's ordering is
/// followed for readability.
///
/// # Not checked here (caller's responsibility)
///
/// This binds the entry to a `(tree_size, root_hash)` **this CA key signed**, but
/// it does **not** bind the checkpoint's `log_id` — a same-key checkpoint for a
/// *different* log over the same root will verify. Confirm the log via
/// [`Verified::log_id`] (see the crate-level "Scope" note). The checkpoint's
/// `signed_at` is unauthenticated (draft §5.4.1) and must not be used for
/// freshness.
///
/// # Errors
///
/// Returns the [`VerifyError`] naming the first failing check:
/// [`MalformedEntry`](VerifyError::MalformedEntry) (step 4);
/// [`SizeMismatch`](VerifyError::SizeMismatch),
/// [`IndexOutOfRange`](VerifyError::IndexOutOfRange),
/// [`MalformedProof`](VerifyError::MalformedProof), or
/// [`WrongRoot`](VerifyError::WrongRoot) (step 5); or
/// [`BadSignature`](VerifyError::BadSignature) (step 6). Never panics on any
/// input (spec §19.8).
pub fn verify_inclusion(
    entry: &LogEntry,
    proof: &InclusionProof,
    checkpoint: &Checkpoint<Signed>,
    ca_pubkey: &VerifyingKey,
) -> Result<Verified, VerifyError> {
    // Step 4: reconstruct the leaf hash. A serialization failure means the entry
    // itself is malformed (never a panic; spec §19.8).
    let leaf_hash = entry
        .leaf_hash::<Sha256Hasher>()
        .map_err(|_| VerifyError::MalformedEntry)?;

    // Step 5a: bind the proof to the checkpoint. A proof for a different tree
    // size proves nothing about this checkpoint's root.
    let proof_size = proof.tree_size();
    let checkpoint_size = checkpoint.tree_size();
    if proof_size != checkpoint_size {
        return Err(VerifyError::SizeMismatch {
            proof_tree_size: proof_size.0,
            checkpoint_tree_size: checkpoint_size.0,
        });
    }

    // Step 5b: apply the inclusion proof and require it to reconstruct exactly
    // the root the checkpoint commits to.
    proof
        .verify::<Sha256Hasher>(&leaf_hash, checkpoint.root_hash())
        .map_err(|e| map_proof_error(&e))?;

    // Step 6: verify the CA's checkpoint signature (ECDSA P-256, spec §4/§14.1).
    // Any failure — bad signature, wrong/malformed key, algorithm mismatch, or
    // an un-encodable signed input — collapses to "not authentically signed".
    checkpoint
        .verify(&EcdsaP256, ca_pubkey)
        .map_err(|_| VerifyError::BadSignature)?;

    Ok(Verified {
        log_id: checkpoint.log_id().clone(),
        tree_size: checkpoint_size,
        leaf_index: proof.leaf_index(),
        root_hash: *checkpoint.root_hash(),
    })
}

/// Maps a [`ProofError`] from inclusion-proof application (step 5) to the
/// relying-party [`VerifyError`] reason taxonomy.
///
/// `NonMonotonicSizes` and `TreeTooSmall` are generation-time faults that
/// [`InclusionProof::verify`] cannot raise; they are folded into
/// [`VerifyError::MalformedProof`] defensively so the mapping is total and
/// panic-free.
const fn map_proof_error(error: &ProofError) -> VerifyError {
    match error {
        ProofError::IndexOutOfRange { index, tree_size } => VerifyError::IndexOutOfRange {
            index: *index,
            tree_size: *tree_size,
        },
        ProofError::MalformedPath { expected, actual } => VerifyError::MalformedProof {
            expected: *expected,
            actual: *actual,
        },
        ProofError::RootMismatch => VerifyError::WrongRoot,
        ProofError::NonMonotonicSizes { .. } | ProofError::TreeTooSmall { .. } => {
            VerifyError::MalformedProof {
                expected: 0,
                actual: 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{verify_inclusion, VerifyError};
    use mtc::{
        Checkpoint, CheckpointBuilder, Claim, DnsName, EcdsaP256, HashOutput, InclusionProof,
        Index, LogEntry, LogId, MerkleTree, Sha256Hasher, Signed, SignedAt, SubjectInfoHash,
        SubjectType, TbsCertificateLogEntry, TlsSerialize, TreeSize, VerifyingKey,
    };

    type Tree = MerkleTree<Sha256Hasher>;

    fn entry(i: u8) -> LogEntry {
        LogEntry::certificate(
            TbsCertificateLogEntry::builder()
                .subject_type(SubjectType::Tls)
                .subject_info_hash(SubjectInfoHash::from_hash(HashOutput([i; 32])))
                .claim(Claim::dns(vec![DnsName::new(b"example.com".to_vec()).unwrap()]).unwrap())
                .build(),
        )
    }

    /// Builds a tree of `n` certificate entries, returns the entries and tree.
    fn log_of(n: u8) -> (Vec<LogEntry>, Tree) {
        let entries: Vec<LogEntry> = (0..n).map(entry).collect();
        let mut tree = Tree::new();
        for e in &entries {
            tree.append(&e.tls_serialize_to_vec().unwrap());
        }
        (entries, tree)
    }

    /// Signs a checkpoint over `tree`'s current root under a fresh CA keypair.
    fn signed_checkpoint(tree: &Tree) -> (Checkpoint<Signed>, VerifyingKey) {
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let cp = CheckpointBuilder::new(LogId::new("test-log").unwrap())
            .root_hash(tree.root())
            .tree_size(tree.len())
            .signed_at(SignedAt(0))
            .build()
            .sign(&EcdsaP256, &signing)
            .unwrap();
        (cp, verifying)
    }

    #[test]
    fn valid_proof_verifies_and_reports_committed_facts() {
        let (entries, tree) = log_of(9);
        let (cp, pubkey) = signed_checkpoint(&tree);
        for (i, e) in entries.iter().enumerate() {
            let proof = InclusionProof::generate(&tree, Index(i as u64)).unwrap();
            let verified = verify_inclusion(e, &proof, &cp, &pubkey).unwrap();
            assert_eq!(verified.leaf_index(), Index(i as u64));
            assert_eq!(verified.tree_size(), TreeSize(9));
            assert_eq!(verified.root_hash(), tree.root());
            // Verified exposes the checkpoint's log_id for the caller's binding.
            assert_eq!(verified.log_id(), &LogId::new("test-log").unwrap());
        }
    }

    #[test]
    fn log_id_is_not_bound_but_is_surfaced_for_the_caller() {
        // Crypto F2 boundary (deliberate, documented): verify_inclusion binds
        // the entry to a (tree_size, root_hash) THIS CA KEY signed, but does NOT
        // check the checkpoint's log_id. A same-key checkpoint for a DIFFERENT
        // log over the SAME root therefore verifies — the caller distinguishes
        // logs via Verified::log_id(). Binding the expected log is read-verify-cert.
        let (entries, tree) = log_of(8);
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let cp_for = |log: &str| {
            CheckpointBuilder::new(LogId::new(log).unwrap())
                .root_hash(tree.root())
                .tree_size(tree.len())
                .signed_at(SignedAt(0))
                .build()
                .sign(&EcdsaP256, &signing)
                .unwrap()
        };
        let cp_a = cp_for("log-A");
        let cp_b = cp_for("log-B"); // different log, same key, same root
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();

        // Both verify (log_id is not bound here)...
        let va = verify_inclusion(&entries[3], &proof, &cp_a, &verifying).unwrap();
        let vb = verify_inclusion(&entries[3], &proof, &cp_b, &verifying).unwrap();
        // ...but Verified reports each checkpoint's own log_id, so a caller that
        // expected "log-A" can reject the "log-B" checkpoint.
        assert_eq!(va.log_id().as_str(), "log-A");
        assert_eq!(vb.log_id().as_str(), "log-B");
        assert_ne!(va.log_id(), vb.log_id());
    }

    #[test]
    fn single_leaf_tree_verifies() {
        // Empty audit path; the root is the single leaf hash.
        let (entries, tree) = log_of(1);
        let (cp, pubkey) = signed_checkpoint(&tree);
        let proof = InclusionProof::generate(&tree, Index(0)).unwrap();
        assert!(proof.audit_path().is_empty());
        assert!(verify_inclusion(&entries[0], &proof, &cp, &pubkey).is_ok());
    }

    #[test]
    fn wrong_entry_at_index_is_wrong_root() {
        // Presenting entry 4 for a proof of index 3: the leaf hash differs, so
        // the reconstructed root does not match the checkpoint.
        let (entries, tree) = log_of(8);
        let (cp, pubkey) = signed_checkpoint(&tree);
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();
        assert_eq!(
            verify_inclusion(&entries[4], &proof, &cp, &pubkey).unwrap_err(),
            VerifyError::WrongRoot,
        );
    }

    #[test]
    fn tampered_sibling_is_wrong_root() {
        let (entries, tree) = log_of(8);
        let (cp, pubkey) = signed_checkpoint(&tree);
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();
        let mut path = proof.audit_path().to_vec();
        path[0].0[0] ^= 0x01;
        let tampered = InclusionProof::from_parts(proof.tree_size(), proof.leaf_index(), path);
        assert_eq!(
            verify_inclusion(&entries[3], &tampered, &cp, &pubkey).unwrap_err(),
            VerifyError::WrongRoot,
        );
    }

    #[test]
    fn truncated_audit_path_is_malformed_proof() {
        let (entries, tree) = log_of(8);
        let (cp, pubkey) = signed_checkpoint(&tree);
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();
        let full = proof.audit_path().len();
        let mut short = proof.audit_path().to_vec();
        short.pop();
        let truncated = InclusionProof::from_parts(proof.tree_size(), proof.leaf_index(), short);
        assert_eq!(
            verify_inclusion(&entries[3], &truncated, &cp, &pubkey).unwrap_err(),
            VerifyError::MalformedProof {
                expected: full,
                actual: full - 1,
            },
        );
    }

    #[test]
    fn overlong_audit_path_is_malformed_proof() {
        let (entries, tree) = log_of(8);
        let (cp, pubkey) = signed_checkpoint(&tree);
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();
        let full = proof.audit_path().len();
        let mut long = proof.audit_path().to_vec();
        long.push(HashOutput([0u8; 32]));
        let padded = InclusionProof::from_parts(proof.tree_size(), proof.leaf_index(), long);
        assert_eq!(
            verify_inclusion(&entries[3], &padded, &cp, &pubkey).unwrap_err(),
            VerifyError::MalformedProof {
                expected: full,
                actual: full + 1,
            },
        );
    }

    #[test]
    fn out_of_range_leaf_index_is_typed_not_panic() {
        let (entries, tree) = log_of(5);
        let (cp, pubkey) = signed_checkpoint(&tree);
        // A proof declaring an index beyond the tree size.
        let bad = InclusionProof::from_parts(TreeSize(5), Index(99), Vec::new());
        assert_eq!(
            verify_inclusion(&entries[0], &bad, &cp, &pubkey).unwrap_err(),
            VerifyError::IndexOutOfRange {
                index: 99,
                tree_size: 5,
            },
        );
    }

    #[test]
    fn size_mismatch_between_proof_and_checkpoint() {
        // A proof for a 4-entry tree presented against an 8-entry checkpoint.
        let (_e4, tree4) = log_of(4);
        let (entries8, tree8) = log_of(8);
        let (cp8, pubkey) = signed_checkpoint(&tree8);
        let proof4 = InclusionProof::generate(&tree4, Index(1)).unwrap();
        assert_eq!(
            verify_inclusion(&entries8[1], &proof4, &cp8, &pubkey).unwrap_err(),
            VerifyError::SizeMismatch {
                proof_tree_size: 4,
                checkpoint_tree_size: 8,
            },
        );
    }

    #[test]
    fn wrong_ca_key_is_bad_signature() {
        let (entries, tree) = log_of(8);
        let (cp, _pubkey) = signed_checkpoint(&tree);
        let (_other_signing, other_pubkey) = EcdsaP256::generate_keypair();
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();
        assert_eq!(
            verify_inclusion(&entries[3], &proof, &cp, &other_pubkey).unwrap_err(),
            VerifyError::BadSignature,
        );
    }

    #[test]
    fn tampered_checkpoint_root_is_wrong_root_before_signature() {
        // Grafting a different root onto the (still validly-shaped) checkpoint:
        // step 5 fails first because the proof cannot reconstruct the new root.
        let (entries, tree) = log_of(8);
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let cp = CheckpointBuilder::new(LogId::new("test-log").unwrap())
            .root_hash(HashOutput([0xAB; 32])) // not the real root
            .tree_size(tree.len())
            .signed_at(SignedAt(0))
            .build()
            .sign(&EcdsaP256, &signing)
            .unwrap();
        let proof = InclusionProof::generate(&tree, Index(3)).unwrap();
        assert_eq!(
            verify_inclusion(&entries[3], &proof, &cp, &verifying).unwrap_err(),
            VerifyError::WrongRoot,
        );
    }

    #[test]
    fn malformed_key_is_bad_signature_not_panic() {
        // An un-decodable SPKI blob tagged as ECDSA P-256 must not panic.
        let (entries, tree) = log_of(4);
        let (cp, _pubkey) = signed_checkpoint(&tree);
        let junk = VerifyingKey::from_spki_der(
            mtc::SignatureAlgorithm::EcdsaP256Sha256,
            vec![0xDE, 0xAD, 0xBE, 0xEF],
        );
        let proof = InclusionProof::generate(&tree, Index(1)).unwrap();
        assert_eq!(
            verify_inclusion(&entries[1], &proof, &cp, &junk).unwrap_err(),
            VerifyError::BadSignature,
        );
    }

    #[test]
    fn reason_labels_are_stable_and_distinct() {
        use std::collections::BTreeSet;
        let reasons: [&str; 6] = [
            VerifyError::MalformedEntry.reason(),
            VerifyError::SizeMismatch {
                proof_tree_size: 1,
                checkpoint_tree_size: 2,
            }
            .reason(),
            VerifyError::IndexOutOfRange {
                index: 1,
                tree_size: 1,
            }
            .reason(),
            VerifyError::MalformedProof {
                expected: 1,
                actual: 2,
            }
            .reason(),
            VerifyError::WrongRoot.reason(),
            VerifyError::BadSignature.reason(),
        ];
        // All distinct: telemetry buckets never collide.
        let unique: BTreeSet<&str> = reasons.iter().copied().collect();
        assert_eq!(unique.len(), reasons.len());
        assert_eq!(VerifyError::WrongRoot.reason(), "wrong_root");
        assert_eq!(VerifyError::BadSignature.reason(), "bad_signature");
    }
}
