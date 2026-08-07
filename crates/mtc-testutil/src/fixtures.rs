//! Fixture builders for checkpoints, log entries, and tree-leaf sequences.
//!
//! Driven by [`crate::rng::seeded_rng`] rather than `proptest`'s `Strategy`
//! machinery (spec §19.2, §19.3 Layer 1) — for tests, benchmarks, and demos
//! that want one concrete, reproducible value rather than a generator (see
//! [`crate::strategy`] for the generator-shaped counterpart).
//!
//! # Error handling in a test-support crate (rule `no-unwrap-in-prod`)
//!
//! This crate's own source is ordinary library code, not gated by
//! `#[cfg(test)]` — the workspace `unwrap_used` / `expect_used` denials apply
//! to it exactly as they do to `mtc` itself, even though every caller reaches
//! these functions from a `[dev-dependencies]` edge. Several builders here
//! call fallible `mtc` constructors (`DnsName::new`, `Claim::dns`,
//! `Checkpoint::sign`, `LogEntry::leaf_bytes`) on inputs this crate fully
//! controls and has chosen to keep well within their valid bounds — so the
//! failures are unreachable in practice. "Unreachable in practice" is not
//! something the type system can prove, though, so — matching the house
//! convention (`mtc::checkpoint::read_log_id` maps its own provably-unreachable
//! error rather than unwrapping it) — every such call propagates through
//! [`FixtureError`] instead of being unwrapped.

use mtc::{
    Checkpoint, CheckpointBuilder, Claim, DnsName, EcdsaP256, HashOutput, Hasher, LogEntry, LogId,
    MerkleTree, Sha256Hasher, Signed, SignedAt, SubjectInfoHash, SubjectType,
    TbsCertificateLogEntry, TreeSize, Unsigned, VerifyingKey,
};
use rand::RngCore;

/// Errors from building a fixture value.
///
/// See the [module docs](self) for why fixture builders return this instead
/// of unwrapping: every variant here is unreachable for the bounded inputs
/// this crate's own builders generate, but the underlying `mtc` constructors
/// are fallible APIs, so failures are propagated rather than assumed away.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FixtureError {
    /// A generated claim or DNS name violated a spec `<min..max>` bound.
    #[error(transparent)]
    Entry(#[from] mtc::EntryError),
    /// Signing a fixture checkpoint failed.
    #[error(transparent)]
    Sign(#[from] mtc::CheckpointSignError),
    /// Encoding a fixture log entry to its leaf bytes failed.
    #[error("encoding fixture leaf bytes failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Builds an unsigned checkpoint over `log_id` with content drawn from `rng`.
///
/// `root_hash`, `tree_size`, and `signed_at` are independent random values —
/// this checkpoint does not commit to any real tree. Use
/// [`checkpoint_for_tree`] when the checkpoint must be consistent with an
/// actual [`MerkleTree`]. Infallible: every field is either caller-supplied
/// or a direct newtype wrap of `rng` output.
#[must_use]
pub fn checkpoint(rng: &mut impl RngCore, log_id: LogId) -> Checkpoint<Unsigned> {
    let mut root = [0u8; 32];
    rng.fill_bytes(&mut root);
    CheckpointBuilder::new(log_id)
        .root_hash(HashOutput(root))
        .tree_size(TreeSize(rng.next_u64()))
        .signed_at(SignedAt(rng.next_u64()))
        .build()
}

/// Builds an unsigned checkpoint that commits to `tree`'s actual root and
/// size — the checkpoint a real write path would produce over that tree.
#[must_use]
pub fn checkpoint_for_tree<H: Hasher>(
    tree: &MerkleTree<H>,
    log_id: LogId,
    signed_at: SignedAt,
) -> Checkpoint<Unsigned> {
    CheckpointBuilder::new(log_id)
        .root_hash(tree.root())
        .tree_size(tree.len())
        .signed_at(signed_at)
        .build()
}

/// Builds and signs a checkpoint with a freshly generated ECDSA P-256
/// keypair, returning the signed checkpoint and its matching verifying key.
///
/// The checkpoint content is deterministic from `rng` (see [`checkpoint`]);
/// the keypair is not — `EcdsaP256::generate_keypair` draws from the OS RNG
/// (see the [`crate::rng`] module docs).
///
/// # Errors
///
/// [`FixtureError::Sign`] if `log_id` exceeds the 255-byte `TrustAnchorID`
/// bound (draft §5.4.1) — unreachable for any realistic log id, but not
/// something this function can prove about a caller-supplied [`LogId`].
pub fn signed_checkpoint(
    rng: &mut impl RngCore,
    log_id: LogId,
) -> Result<(Checkpoint<Signed>, VerifyingKey), FixtureError> {
    let cp = checkpoint(rng, log_id);
    let (signing, verifying) = EcdsaP256::generate_keypair();
    let signed = cp.sign(&EcdsaP256, &signing)?;
    Ok((signed, verifying))
}

/// Builds one certificate log entry with a single DNS claim
/// (`host-<n>.example.test`, `n` drawn from `rng`) and a random subject-info
/// hash.
///
/// # Errors
///
/// [`FixtureError::Entry`] — unreachable in practice (the generated DNS name
/// is always a non-empty, well-under-255-byte ASCII string), but see the
/// [module docs](self) for why this propagates rather than unwraps.
pub fn certificate_entry(rng: &mut impl RngCore) -> Result<TbsCertificateLogEntry, FixtureError> {
    let mut hash = [0u8; 32];
    rng.fill_bytes(&mut hash);
    let suffix = rng.next_u32();
    let name = DnsName::new(format!("host-{suffix}.example.test").into_bytes())?;
    let claim = Claim::dns(vec![name])?;
    Ok(TbsCertificateLogEntry::builder()
        .subject_type(SubjectType::Tls)
        .subject_info_hash(SubjectInfoHash::from_hash(HashOutput(hash)))
        .claim(claim)
        .build())
}

/// Builds one log entry: usually a [`certificate_entry`], sometimes a
/// [`LogEntry::Null`] gap.
///
/// A null entry is produced roughly one call in `null_rate` (e.g. `null_rate
/// = 8` gives ~12.5% null entries, mirroring the abandoned-batch gaps a real
/// log accumulates — spec §2, §13.3); `null_rate = 0` never produces one.
///
/// # Errors
///
/// See [`certificate_entry`].
pub fn log_entry(rng: &mut impl RngCore, null_rate: u32) -> Result<LogEntry, FixtureError> {
    if null_rate > 0 && rng.next_u32().is_multiple_of(null_rate) {
        return Ok(LogEntry::null());
    }
    Ok(LogEntry::certificate(certificate_entry(rng)?))
}

/// Builds a sequence of `count` deterministic-from-`rng` log entries, roughly
/// one in eight a [`null_entry`](mtc::null_entry) gap (see [`log_entry`]).
///
/// This is the "tree-leaf sequences" fixture (spec §19.2/§19.3 Layer 1): feed
/// the result to [`tree`], or append each entry's
/// [`leaf_bytes`](LogEntry::leaf_bytes) into a [`MerkleTree`] directly.
///
/// # Errors
///
/// See [`certificate_entry`].
pub fn leaf_sequence(rng: &mut impl RngCore, count: usize) -> Result<Vec<LogEntry>, FixtureError> {
    (0..count).map(|_| log_entry(rng, 8)).collect()
}

/// Builds a [`MerkleTree`] over `count` deterministic-from-`rng` log entries
/// (see [`leaf_sequence`]).
///
/// # Errors
///
/// [`FixtureError::Entry`] from generating an entry, or [`FixtureError::Io`]
/// if an entry fails to encode to its leaf bytes (a claim body overflowing
/// its `u16` length prefix; unreachable for these small generated entries,
/// but not something this function can prove — spec §22.6).
pub fn tree(
    rng: &mut impl RngCore,
    count: usize,
) -> Result<MerkleTree<Sha256Hasher>, FixtureError> {
    let mut t = MerkleTree::with_capacity(count);
    for entry in leaf_sequence(rng, count)? {
        t.append(&entry.leaf_bytes()?);
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use mtc::{EcdsaP256, LogId, SignedAt, TlsParse, TlsSerialize};

    use super::{
        certificate_entry, checkpoint, checkpoint_for_tree, leaf_sequence, log_entry,
        signed_checkpoint, tree,
    };
    use crate::rng::seeded_rng;

    fn log_id() -> LogId {
        LogId::new("fixture-log").unwrap()
    }

    #[test]
    fn checkpoint_fixture_carries_the_supplied_log_id_and_assembles_its_signature_input() {
        let mut rng = seeded_rng(1);
        let cp = checkpoint(&mut rng, log_id());
        assert_eq!(cp.log_id().as_str(), "fixture-log");
        assert!(cp.signature_input().is_ok());
    }

    #[test]
    fn checkpoint_fixture_is_deterministic_for_the_same_seed() {
        let a = checkpoint(&mut seeded_rng(7), log_id());
        let b = checkpoint(&mut seeded_rng(7), log_id());
        assert_eq!(a, b);
    }

    #[test]
    fn checkpoint_fixture_diverges_for_different_seeds() {
        let a = checkpoint(&mut seeded_rng(7), log_id());
        let b = checkpoint(&mut seeded_rng(8), log_id());
        assert_ne!(a, b);
    }

    #[test]
    fn checkpoint_for_tree_commits_to_the_real_root_and_size() {
        let t = tree(&mut seeded_rng(2), 5).unwrap();
        let cp = checkpoint_for_tree(&t, log_id(), SignedAt(0));
        assert_eq!(cp.root_hash(), &t.root());
        assert_eq!(cp.tree_size(), t.len());
    }

    #[test]
    fn signed_checkpoint_fixture_verifies_under_its_own_returned_key() {
        let (signed, verifying) = signed_checkpoint(&mut seeded_rng(3), log_id()).unwrap();
        assert!(signed.verify(&EcdsaP256, &verifying).is_ok());
    }

    #[test]
    fn certificate_entry_fixture_is_a_valid_domain_value_that_round_trips() {
        let entry = certificate_entry(&mut seeded_rng(4)).unwrap();
        let bytes = entry.tls_serialize_to_vec().unwrap();
        let parsed = mtc::TbsCertificateLogEntry::tls_parse_exact(&bytes).unwrap();
        assert_eq!(entry, parsed);
    }

    #[test]
    fn log_entry_fixture_null_rate_controls_null_vs_certificate() {
        // null_rate = 1: `x % 1 == 0` for every x, so every call is null.
        let forced_null = log_entry(&mut seeded_rng(5), 1).unwrap();
        assert!(forced_null.is_null());

        // null_rate = 0: the null branch is never taken.
        let forced_certificate = log_entry(&mut seeded_rng(5), 0).unwrap();
        assert!(!forced_certificate.is_null());
    }

    #[test]
    fn leaf_sequence_fixture_produces_exactly_the_requested_count() {
        let entries = leaf_sequence(&mut seeded_rng(6), 12).unwrap();
        assert_eq!(entries.len(), 12);
    }

    #[test]
    fn leaf_sequence_fixture_is_deterministic_for_the_same_seed() {
        let a = leaf_sequence(&mut seeded_rng(6), 12).unwrap();
        let b = leaf_sequence(&mut seeded_rng(6), 12).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn tree_fixture_len_matches_the_requested_count_and_root_is_deterministic() {
        let t1 = tree(&mut seeded_rng(9), 20).unwrap();
        let t2 = tree(&mut seeded_rng(9), 20).unwrap();
        assert_eq!(t1.len(), mtc::TreeSize(20));
        assert_eq!(t1.root(), t2.root());
    }

    #[test]
    fn tree_fixture_handles_the_empty_case() {
        let t = tree(&mut seeded_rng(10), 0).unwrap();
        assert!(t.is_empty());
        assert_eq!(t.root(), mtc::SHA256_EMPTY_ROOT);
    }
}
