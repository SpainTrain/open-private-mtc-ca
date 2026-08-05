//! The [`Checkpoint`] — a signed `(tree_size, root_hash)` commitment to the
//! issuance log (spec §2, "Checkpoint"; `draft-ietf-plants-merkle-tree-certs-03`
//! §5.4.1).
//!
//! A checkpoint is how the CA publishes "the log has exactly these `tree_size`
//! entries, and their Merkle root is `root_hash`". Relying parties trust it
//! because the CA signs it; every MTC certificate is an entry plus an inclusion
//! proof to a *signed* checkpoint (spec §2, §12.1).
//!
//! # Two typestates, two guarantees (spec §22.2, §22.4)
//!
//! - [`CheckpointBuilder`] (§22.2) proves at compile time that a checkpoint's
//!   four required fields — `log_id`, `root_hash`, `tree_size`, `signed_at` —
//!   are all present before it can be built.
//! - [`Checkpoint<State>`] (§22.4) then tracks *signedness* in the type:
//!   [`build`](CheckpointBuilder::build) yields a [`Checkpoint<Unsigned>`],
//!   which has no signature and no [`verify`](Checkpoint::verify) method;
//!   [`sign`](Checkpoint::sign) consumes it and returns a
//!   [`Checkpoint<Signed>`]. An unsigned checkpoint therefore *cannot* be passed
//!   where a signed one is required — the compiler rejects it (see
//!   `tests/compile_fail/checkpoint_unsigned_not_signed.rs`).
//!
//! # What is signed (crown jewel; see the `signature_input` submodule)
//!
//! Signing does **not** sign the raw fields; it signs the domain-separated
//! `MTCSubtreeSignatureInput` (draft §5.4.1) assembled by the
//! `signature_input` submodule. A checkpoint is the `start == 0` subtree, so
//! the signed bytes commit to `(log_id, tree_size, root_hash)` under the
//! `mtc-subtree/v1` label. Notably, `signed_at` is **not** part of the signed
//! payload (the draft structure carries no timestamp), so it is authenticated
//! metadata of the checkpoint record but not of the signature — see
//! [`Checkpoint::signature_input`] and the tests.
//!
//! # Signing/verification backend (ADR-0003)
//!
//! Sign and verify delegate to the [`SignatureScheme`] abstraction (v1: ECDSA
//! P-256, ADR-0003). This module never re-implements signing; it assembles the
//! bytes and hands them to the scheme. Production checkpoint signing runs on the
//! HSM behind the same [`SignatureScheme`]/[`VerifyingKey`] contract, so these
//! callers are backend-agnostic (spec §14).

mod builder;
mod signature_input;

pub use builder::{
    CheckpointBuilder, NoRootHash, NoSignedAt, NoTreeSize, WithRootHash, WithSignedAt, WithTreeSize,
};
pub use signature_input::{
    subtree_signature_input, trust_anchor_id_len_byte, TrustAnchorIdError, SUBTREE_SIGNATURE_LABEL,
};

use std::io::{self, Write};

use thiserror::Error;

use crate::signing::{
    SignError, Signature, SignatureScheme, SigningKey, VerifyError, VerifyingKey,
};
use crate::types::{HashOutput, LogId, TreeSize};
use crate::wire::{write_bytes, write_opaque_u16, TlsReader, TlsSerialize, WireError};

use signature_input::write_trust_anchor_id;

/// A checkpoint's timestamp: seconds since the Unix epoch (spec §8.2
/// `signed_at`).
///
/// A distinct newtype (rule `use-newtypes`, spec §22.1) so a raw second count
/// cannot be confused with a [`TreeSize`] or any other `u64`. The value is
/// supplied by the caller from the injected `Clock` (rule
/// `no-systemtime-now-in-prod`); this crate never reads the wall clock.
///
/// `signed_at` is checkpoint metadata: it is serialized in the checkpoint
/// record but is **not** covered by the checkpoint signature (draft §5.4.1
/// defines no timestamp field in the signed input).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct SignedAt(pub u64);

/// Typestate marker for a checkpoint that has **not** been signed
/// (spec §22.4).
///
/// A [`Checkpoint<Unsigned>`] is the output of
/// [`CheckpointBuilder::build`]; it carries the committed values but no
/// signature, and exposes [`sign`](Checkpoint::sign) but not
/// [`verify`](Checkpoint::verify).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsigned;

/// Typestate marker for a checkpoint that carries a cosigner signature
/// (spec §22.4).
///
/// A [`Checkpoint<Signed>`] is produced by [`Checkpoint::sign`]; it holds the
/// [`Signature`] over the checkpoint's `MTCSubtreeSignatureInput` and exposes
/// [`verify`](Checkpoint::verify), [`signature`](Checkpoint::signature), and
/// wire serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    signature: Signature,
}

/// A signed (or not-yet-signed) `(tree_size, root_hash)` commitment to the
/// issuance log (spec §2, "Checkpoint").
///
/// The `State` type parameter is either [`Unsigned`] (fresh from the builder)
/// or [`Signed`] (after [`sign`](Self::sign)); it defaults to [`Unsigned`] so
/// the builder's output can be named without spelling the parameter out. All
/// fields are private and set through [`CheckpointBuilder`], upholding the
/// §22.2 "no partial checkpoint" guarantee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint<State = Unsigned> {
    log_id: LogId,
    tree_size: TreeSize,
    root_hash: HashOutput,
    signed_at: SignedAt,
    state: State,
}

impl<State> Checkpoint<State> {
    /// The identifier of the log this checkpoint commits to (spec §2,
    /// "Issuance log").
    #[must_use]
    pub const fn log_id(&self) -> &LogId {
        &self.log_id
    }

    /// The committed tree size — the number of entries the checkpoint commits
    /// to (spec §2, "Checkpoint").
    #[must_use]
    pub const fn tree_size(&self) -> TreeSize {
        self.tree_size
    }

    /// The committed Merkle root hash (spec §2, "Checkpoint").
    #[must_use]
    pub const fn root_hash(&self) -> &HashOutput {
        &self.root_hash
    }

    /// The checkpoint's timestamp (spec §8.2). Not covered by the signature —
    /// see [`Self::signature_input`].
    #[must_use]
    pub const fn signed_at(&self) -> SignedAt {
        self.signed_at
    }

    /// Assembles the exact `MTCSubtreeSignatureInput` bytes this checkpoint is
    /// (or would be) signed over (draft §5.4.1) — the crown-jewel signed
    /// payload.
    ///
    /// A checkpoint is the subtree `[0, tree_size)`, so this is
    /// `subtree_signature_input(id, id, 0, tree_size, root_hash)` where `id` is
    /// the log's trust-anchor ID (the [`LogId`] UTF-8 bytes; `cosigner_id ==
    /// log_id` for this cosigner-free CA — spec §1). It is available in both
    /// typestates because it depends only on the committed fields, so a caller
    /// can inspect the to-be-signed bytes before signing.
    ///
    /// # Errors
    ///
    /// [`TrustAnchorIdError`] if the log's trust-anchor ID is not 1..=255 bytes
    /// (`opaque<1..2^8-1>`, draft §5.4.1) — e.g. a [`LogId`] longer than 255
    /// bytes cannot be encoded.
    pub fn signature_input(&self) -> Result<Vec<u8>, TrustAnchorIdError> {
        let id = self.log_id.as_str().as_bytes();
        subtree_signature_input(id, id, 0, self.tree_size.0, &self.root_hash)
    }
}

impl Checkpoint<Unsigned> {
    /// Signs this checkpoint with `scheme` and `key`, transitioning it to
    /// [`Checkpoint<Signed>`] (spec §22.4; draft §5.4.1).
    ///
    /// The signature is computed over the [`signature_input`](Self::signature_input)
    /// bytes (the domain-separated `MTCSubtreeSignatureInput`), never over the
    /// raw fields. Any [`SignatureScheme`] works (v1 is ECDSA P-256); the same
    /// scheme and the matching [`VerifyingKey`] verify it later.
    ///
    /// # Errors
    ///
    /// - [`CheckpointSignError::Input`] if the signature input cannot be
    ///   assembled (trust-anchor ID out of the 1..=255-byte range).
    /// - [`CheckpointSignError::Sign`] if the scheme rejects the key or fails to
    ///   sign (e.g. algorithm mismatch, malformed secret — never a panic).
    pub fn sign(
        self,
        scheme: &dyn SignatureScheme,
        key: &SigningKey,
    ) -> Result<Checkpoint<Signed>, CheckpointSignError> {
        let input = self.signature_input()?;
        let signature = scheme.sign(key, &input)?;
        Ok(Checkpoint {
            log_id: self.log_id,
            tree_size: self.tree_size,
            root_hash: self.root_hash,
            signed_at: self.signed_at,
            state: Signed { signature },
        })
    }
}

impl Checkpoint<Signed> {
    /// Borrows the checkpoint's signature.
    ///
    /// Per ADR-0003 (Decision B.1), these bytes are malleable and **must never**
    /// be used as an identifier, deduplication key, idempotency key, or cache
    /// key.
    #[must_use]
    pub const fn signature(&self) -> &Signature {
        &self.state.signature
    }

    /// Verifies the checkpoint's signature under `key` using `scheme`
    /// (draft §5.4.1).
    ///
    /// Reconstructs the [`signature_input`](Self::signature_input) bytes and
    /// checks the signature against them, so a mutated `tree_size`, mutated
    /// `root_hash`, mutated `log_id`, wrong signature, or wrong key all fail.
    /// (A mutated `signed_at` does **not** fail — it is not part of the signed
    /// input; draft §5.4.1.)
    ///
    /// # Errors
    ///
    /// - [`CheckpointVerifyError::Input`] if the signature input cannot be
    ///   assembled (trust-anchor ID out of range).
    /// - [`CheckpointVerifyError::Verify`] if the signature does not verify, or
    ///   the key/signature bytes are malformed or algorithm-mismatched. Verifying
    ///   never panics (spec §19.6).
    #[must_use = "an unchecked signature verification result is a security bug (spec §22.10)"]
    pub fn verify(
        &self,
        scheme: &dyn SignatureScheme,
        key: &VerifyingKey,
    ) -> Result<(), CheckpointVerifyError> {
        let input = self.signature_input()?;
        scheme
            .verify(key, &input, &self.state.signature)
            .map_err(CheckpointVerifyError::Verify)
    }

    /// Serializes the signed checkpoint to its TLS-presentation wire bytes
    /// (spec §19.3).
    ///
    /// The layout is `TrustAnchorID log_id || uint64 tree_size ||
    /// HashValue root_hash || uint64 signed_at || opaque signature<0..2^16-1>`.
    /// Round-trips with [`Self::parse_tls_presentation`].
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidData`] if the [`LogId`] exceeds the 255-byte
    /// `TrustAnchorID` bound and so cannot be length-prefixed (defence in depth;
    /// real log IDs are short).
    pub fn serialize_tls_presentation(&self) -> io::Result<Vec<u8>> {
        self.tls_serialize_to_vec()
    }

    /// Parses a signed checkpoint from TLS-presentation wire bytes, requiring
    /// every byte to be consumed (spec §19.3).
    ///
    /// This is the untrusted-input boundary: it never panics, bounds every
    /// length against the remaining input, and hand-enforces the `TrustAnchorID`
    /// minimum length (the generic reader admits a zero-length opaque; the draft
    /// forbids it — crypto F3 / bead `mtc-qka.3`).
    ///
    /// # Errors
    ///
    /// [`CheckpointParseError`]: a [`WireError`](CheckpointParseError::Wire) for
    /// truncated/oversized/trailing bytes, [`TrustAnchorIdEmpty`] for a
    /// zero-length log-id field, or [`LogIdNotUtf8`] for non-UTF-8 log-id bytes.
    ///
    /// [`TrustAnchorIdEmpty`]: CheckpointParseError::TrustAnchorIdEmpty
    /// [`LogIdNotUtf8`]: CheckpointParseError::LogIdNotUtf8
    pub fn parse_tls_presentation(bytes: &[u8]) -> Result<Self, CheckpointParseError> {
        let mut reader = TlsReader::new(bytes);
        let checkpoint = Self::parse_from(&mut reader)?;
        reader.finish()?;
        Ok(checkpoint)
    }

    /// Reads the fields of a signed checkpoint from a bounded reader.
    fn parse_from(reader: &mut TlsReader<'_>) -> Result<Self, CheckpointParseError> {
        let log_id = read_log_id(reader)?;
        let tree_size = TreeSize(read_u64(reader)?);
        let root_hash = HashOutput(reader.read_array::<{ HashOutput::LEN }>()?);
        let signed_at = SignedAt(read_u64(reader)?);
        let signature = Signature::from_bytes(reader.read_opaque_u16()?.to_vec());
        Ok(Self {
            log_id,
            tree_size,
            root_hash,
            signed_at,
            state: Signed { signature },
        })
    }
}

impl TlsSerialize for Checkpoint<Signed> {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_trust_anchor_id(writer, self.log_id.as_str().as_bytes())?;
        write_u64(writer, self.tree_size.0)?;
        write_bytes(writer, self.root_hash.as_bytes())?;
        write_u64(writer, self.signed_at.0)?;
        write_opaque_u16(writer, self.state.signature.as_bytes())
    }
}

/// Writes a `uint64` in big-endian TLS-presentation form (8 bytes).
///
/// The wire framework ships `u8`/`u16`/`u24`/`u32` primitives but not `u64`,
/// which `MTCSubtree` and `tree_size` need; this is the local helper until the
/// framework grows a shared `u64`.
fn write_u64<W: Write>(writer: &mut W, value: u64) -> io::Result<()> {
    write_bytes(writer, &value.to_be_bytes())
}

/// Reads a big-endian `uint64` (8 bytes) through the bounded reader.
fn read_u64(reader: &mut TlsReader<'_>) -> Result<u64, WireError> {
    Ok(u64::from_be_bytes(reader.read_array::<8>()?))
}

/// Reads a `TrustAnchorID` (`opaque<1..2^8-1>`) and decodes it as a [`LogId`].
///
/// Hand-enforces the draft minimum length (1 byte): the generic
/// [`TlsReader::read_opaque_u8`] admits a zero-length opaque, but a
/// `TrustAnchorID` may not be empty (crypto F3 / bead `mtc-qka.3`). The bytes
/// must also be valid UTF-8, because this CA's trust-anchor ID is carried by a
/// UTF-8 [`LogId`] until the dedicated `TrustAnchorId` type lands
/// (`mtclib-trust-anchor-id`).
fn read_log_id(reader: &mut TlsReader<'_>) -> Result<LogId, CheckpointParseError> {
    let offset = reader.position();
    let bytes = reader.read_opaque_u8()?;
    if bytes.is_empty() {
        return Err(CheckpointParseError::TrustAnchorIdEmpty { offset });
    }
    let text =
        core::str::from_utf8(bytes).map_err(|_| CheckpointParseError::LogIdNotUtf8 { offset })?;
    // `text` is non-empty (checked above), so `LogId::new` cannot fail with
    // `Empty`; map the unreachable error to the same empty-id class rather than
    // unwrapping (rule `no-unwrap-in-prod`).
    LogId::new(text).map_err(|_| CheckpointParseError::TrustAnchorIdEmpty { offset })
}

/// Assembling a checkpoint signature (the [`sign`](Checkpoint::sign) path)
/// failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CheckpointSignError {
    /// The `MTCSubtreeSignatureInput` could not be assembled.
    #[error("cannot assemble the checkpoint signature input: {0}")]
    Input(#[from] TrustAnchorIdError),
    /// The signature scheme rejected the key or failed to sign.
    #[error("signing the checkpoint failed: {0}")]
    Sign(#[from] SignError),
}

/// Verifying a checkpoint signature (the [`verify`](Checkpoint::verify) path)
/// failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CheckpointVerifyError {
    /// The `MTCSubtreeSignatureInput` could not be assembled.
    #[error("cannot assemble the checkpoint signature input: {0}")]
    Input(#[from] TrustAnchorIdError),
    /// The signature did not verify, or the key/signature was malformed.
    #[error("checkpoint signature did not verify: {0}")]
    Verify(#[from] VerifyError),
}

/// Parsing a signed checkpoint from wire bytes failed.
///
/// Wraps the generic [`WireError`] and adds the two checkpoint-specific
/// semantic checks the generic framework cannot express: the `TrustAnchorID`
/// minimum-length rule and the UTF-8 requirement on the log id. Carries the
/// byte `offset` of the fault for fixtures and differential harnesses (spec
/// §19.3), mirroring [`WireError`]'s convention.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CheckpointParseError {
    /// A generic wire-decoding fault (truncation, impossible length, trailing
    /// bytes, …).
    #[error(transparent)]
    Wire(#[from] WireError),
    /// The `TrustAnchorID` (log id) field had length zero, below the draft
    /// minimum of 1 (`opaque<1..2^8-1>`, draft §5.4.1).
    #[error("trust-anchor id (log id) at offset {offset} is empty; must be 1..=255 bytes (draft §5.4.1)")]
    TrustAnchorIdEmpty {
        /// Offset of the zero-length length prefix.
        offset: usize,
    },
    /// The log-id bytes were not valid UTF-8 (this CA carries its trust-anchor
    /// ID as a UTF-8 [`LogId`]).
    #[error("log id at offset {offset} is not valid UTF-8")]
    LogIdNotUtf8 {
        /// Offset of the log-id field.
        offset: usize,
    },
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        subtree_signature_input, Checkpoint, CheckpointBuilder, CheckpointParseError,
        CheckpointVerifyError, Signature, Signed, SignedAt, Unsigned,
    };
    use crate::signing::{EcdsaP256, VerifyError};
    use crate::tree::SHA256_EMPTY_ROOT;
    use crate::types::{HashOutput, LogId, TreeSize};

    /// Builds an unsigned checkpoint with the given committed fields.
    fn unsigned(log: &str, tree_size: u64, root: [u8; 32], signed_at: u64) -> Checkpoint<Unsigned> {
        CheckpointBuilder::new(LogId::new(log).unwrap())
            .root_hash(HashOutput(root))
            .tree_size(TreeSize(tree_size))
            .signed_at(SignedAt(signed_at))
            .build()
    }

    /// A signed checkpoint with `tree_size` replaced but the original signature
    /// kept — an in-memory tamper the type system otherwise forbids.
    fn with_tree_size(cp: &Checkpoint<Signed>, tree_size: TreeSize) -> Checkpoint<Signed> {
        Checkpoint {
            log_id: cp.log_id.clone(),
            tree_size,
            root_hash: cp.root_hash,
            signed_at: cp.signed_at,
            state: Signed {
                signature: cp.state.signature.clone(),
            },
        }
    }

    /// A signed checkpoint with `root_hash` replaced but the signature kept.
    fn with_root_hash(cp: &Checkpoint<Signed>, root_hash: HashOutput) -> Checkpoint<Signed> {
        Checkpoint {
            log_id: cp.log_id.clone(),
            tree_size: cp.tree_size,
            root_hash,
            signed_at: cp.signed_at,
            state: Signed {
                signature: cp.state.signature.clone(),
            },
        }
    }

    /// A signed checkpoint with `signed_at` replaced but the signature kept.
    fn with_signed_at(cp: &Checkpoint<Signed>, signed_at: SignedAt) -> Checkpoint<Signed> {
        Checkpoint {
            log_id: cp.log_id.clone(),
            tree_size: cp.tree_size,
            root_hash: cp.root_hash,
            signed_at,
            state: Signed {
                signature: cp.state.signature.clone(),
            },
        }
    }

    #[test]
    fn build_populates_every_field_and_accessors_read_back() {
        let cp = unsigned("prod-log", 42, [0xab; 32], 1_700_000_000);
        assert_eq!(cp.log_id().as_str(), "prod-log");
        assert_eq!(cp.tree_size(), TreeSize(42));
        assert_eq!(cp.root_hash(), &HashOutput([0xab; 32]));
        assert_eq!(cp.signed_at(), SignedAt(1_700_000_000));
    }

    #[test]
    fn checkpoint_signature_input_is_the_zero_start_subtree() {
        let cp = unsigned("ca", 7, [0x11; 32], 0);
        let input = cp.signature_input().unwrap();
        let expected =
            subtree_signature_input(b"ca", b"ca", 0, 7, &HashOutput([0x11; 32])).unwrap();
        assert_eq!(input, expected);
    }

    #[test]
    fn empty_tree_checkpoint_is_well_defined_and_constant_rooted() {
        // Spec §19.6: the empty-tree checkpoint is well-defined and its root is
        // the constant empty-tree root. It signs and verifies.
        let cp = CheckpointBuilder::new(LogId::new("ca").unwrap())
            .root_hash(SHA256_EMPTY_ROOT)
            .tree_size(TreeSize(0))
            .signed_at(SignedAt(0))
            .build();
        assert_eq!(cp.root_hash(), &SHA256_EMPTY_ROOT);
        let input = cp.signature_input().unwrap();
        assert_eq!(
            input,
            subtree_signature_input(b"ca", b"ca", 0, 0, &SHA256_EMPTY_ROOT).unwrap(),
        );

        let scheme = EcdsaP256;
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let signed = cp
            .sign(&scheme, &signing)
            .expect("sign empty-tree checkpoint");
        signed
            .verify(&scheme, &verifying)
            .expect("empty-tree checkpoint verifies");
    }

    #[test]
    fn sign_then_verify_round_trips() {
        let scheme = EcdsaP256;
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let signed = unsigned("ca", 1000, [0x42; 32], 1)
            .sign(&scheme, &signing)
            .expect("sign");
        signed.verify(&scheme, &verifying).expect("verify");
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let scheme = EcdsaP256;
        let (signing, _verifying) = EcdsaP256::generate_keypair();
        let (_other, other_verifying) = EcdsaP256::generate_keypair();
        let signed = unsigned("ca", 5, [0x01; 32], 0)
            .sign(&scheme, &signing)
            .unwrap();
        assert_eq!(
            signed.verify(&scheme, &other_verifying),
            Err(CheckpointVerifyError::Verify(VerifyError::BadSignature)),
        );
    }

    #[test]
    fn verify_rejects_mutated_tree_size_and_root_hash() {
        let scheme = EcdsaP256;
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let signed = unsigned("ca", 5, [0x01; 32], 0)
            .sign(&scheme, &signing)
            .unwrap();

        // Mutated tree_size: the signed input's `end` changes -> rejected.
        assert_eq!(
            with_tree_size(&signed, TreeSize(6)).verify(&scheme, &verifying),
            Err(CheckpointVerifyError::Verify(VerifyError::BadSignature)),
        );
        // Mutated root_hash: the signed input's `hash` changes -> rejected.
        assert_eq!(
            with_root_hash(&signed, HashOutput([0x02; 32])).verify(&scheme, &verifying),
            Err(CheckpointVerifyError::Verify(VerifyError::BadSignature)),
        );
    }

    #[test]
    fn mutated_signed_at_does_not_break_the_signature() {
        // Draft §5.4.1: `signed_at` is not part of MTCSubtreeSignatureInput, so
        // changing it must NOT invalidate the signature. This documents a
        // deliberate spec property (not a bug): the signature commits to
        // (log_id, tree_size, root_hash), not the timestamp.
        let scheme = EcdsaP256;
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let signed = unsigned("ca", 5, [0x01; 32], 100)
            .sign(&scheme, &signing)
            .unwrap();
        with_signed_at(&signed, SignedAt(999))
            .verify(&scheme, &verifying)
            .expect("signed_at is not covered by the signature");
    }

    #[test]
    fn wire_round_trip_is_byte_exact() {
        let scheme = EcdsaP256;
        let (signing, _verifying) = EcdsaP256::generate_keypair();
        let signed = unsigned("prod-log-1", 12345, [0x7f; 32], 1_700_000_000)
            .sign(&scheme, &signing)
            .unwrap();
        let bytes = signed.serialize_tls_presentation().unwrap();
        let parsed = Checkpoint::<Signed>::parse_tls_presentation(&bytes).unwrap();
        assert_eq!(signed, parsed);
    }

    #[test]
    fn parse_rejects_empty_trust_anchor_id() {
        // Length prefix 0x00 for the log-id field: below the TrustAnchorID
        // minimum of 1 (hand-enforced; the generic reader would accept it).
        let bytes = [0x00u8];
        assert_eq!(
            Checkpoint::<Signed>::parse_tls_presentation(&bytes),
            Err(CheckpointParseError::TrustAnchorIdEmpty { offset: 0 }),
        );
    }

    #[test]
    fn parse_rejects_non_utf8_log_id() {
        // log-id length 1, byte 0xFF is not valid UTF-8.
        let bytes = [0x01u8, 0xFF];
        assert_eq!(
            Checkpoint::<Signed>::parse_tls_presentation(&bytes),
            Err(CheckpointParseError::LogIdNotUtf8 { offset: 0 }),
        );
    }

    #[test]
    fn parse_rejects_truncated_and_trailing_bytes() {
        // A well-formed signed checkpoint, then perturbations.
        let scheme = EcdsaP256;
        let (signing, _v) = EcdsaP256::generate_keypair();
        let signed = unsigned("ca", 1, [0u8; 32], 0)
            .sign(&scheme, &signing)
            .unwrap();
        let good = signed.serialize_tls_presentation().unwrap();

        // Truncated: drop the last byte -> a wire error, never a panic.
        let truncated = &good[..good.len() - 1];
        assert!(matches!(
            Checkpoint::<Signed>::parse_tls_presentation(truncated),
            Err(CheckpointParseError::Wire(_)),
        ));

        // Trailing: append a stray byte -> TrailingBytes.
        let mut trailing = good.clone();
        trailing.push(0x99);
        assert!(matches!(
            Checkpoint::<Signed>::parse_tls_presentation(&trailing),
            Err(CheckpointParseError::Wire(
                crate::wire::WireError::TrailingBytes { .. }
            )),
        ));
    }

    proptest! {
        // Spec §19.3 Layer 1: the `checkpoint_roundtrip` property. Arbitrary
        // signed checkpoints (with arbitrary signature bytes) round-trip
        // through the wire codec.
        #[test]
        fn checkpoint_roundtrip(
            log in "[ -~]{1,64}",
            tree_size in any::<u64>(),
            root in any::<[u8; 32]>(),
            signed_at in any::<u64>(),
            sig in prop::collection::vec(any::<u8>(), 0..300),
        ) {
            let cp = Checkpoint {
                log_id: LogId::new(log).unwrap(),
                tree_size: TreeSize(tree_size),
                root_hash: HashOutput(root),
                signed_at: SignedAt(signed_at),
                state: Signed { signature: Signature::from_bytes(sig) },
            };
            let bytes = cp.serialize_tls_presentation().unwrap();
            let parsed = Checkpoint::<Signed>::parse_tls_presentation(&bytes).unwrap();
            prop_assert_eq!(cp, parsed);
        }

        // Spec §19.3: parsing arbitrary bytes returns Ok or Err, never panics.
        #[test]
        fn parse_arbitrary_bytes_never_panics(
            bytes in prop::collection::vec(any::<u8>(), 0..512),
        ) {
            let _ = Checkpoint::<Signed>::parse_tls_presentation(&bytes);
        }

        // Spec §19.2/§19.6: sign/verify round-trips for any committed values;
        // any tampering or a wrong key is rejected.
        #[test]
        fn sign_verify_and_tamper(
            tree_size in any::<u64>(),
            root in any::<[u8; 32]>(),
            signed_at in any::<u64>(),
        ) {
            let scheme = EcdsaP256;
            let (signing, verifying) = EcdsaP256::generate_keypair();
            let signed = CheckpointBuilder::new(LogId::new("ca").unwrap())
                .root_hash(HashOutput(root))
                .tree_size(TreeSize(tree_size))
                .signed_at(SignedAt(signed_at))
                .build()
                .sign(&scheme, &signing)
                .unwrap();

            prop_assert!(signed.verify(&scheme, &verifying).is_ok());

            // Flipping the low bit of tree_size always changes it -> reject.
            let tampered = with_tree_size(&signed, TreeSize(tree_size ^ 1));
            prop_assert!(tampered.verify(&scheme, &verifying).is_err());

            // A different key -> reject.
            let (_s2, other) = EcdsaP256::generate_keypair();
            prop_assert!(signed.verify(&scheme, &other).is_err());
        }
    }
}
