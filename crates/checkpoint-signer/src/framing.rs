//! The framed §8.1 signed-checkpoint object: its byte layout and its object
//! key. **This is the crown-jewel byte construction of this crate.**
//!
//! # Byte layout (equals mtc's TLS-presentation signed checkpoint)
//!
//! The object body reproduces
//! [`mtc::Checkpoint::<Signed>::serialize_tls_presentation`] exactly (dispatch
//! "Option B") so the read path parses it unchanged:
//!
//! ```text
//! TrustAnchorID log_id       // opaque<1..2^8-1>: u8 length prefix ‖ id bytes
//! uint64        tree_size    // 8 bytes, big-endian
//! HashValue     root_hash    // 32 raw bytes (SHA-256, draft §5.1)
//! uint64        signed_at    // 8 bytes, big-endian
//! opaque        signature    // <0..2^16-1>: u16 length prefix ‖ r‖s bytes
//! ```
//!
//! The 16-byte `mtc-subtree/v1` domain label (ADR-0005) lives in the *signed
//! input*, not in this object body — the signature already commits to it.
//!
//! # Object key: addressed by `tree_size`, never by the signature
//!
//! Production HSM ECDSA is randomized, so a checkpoint's signature bytes are
//! not stable and **must never** be a storage/idempotency key (ADR-0003 B.1).
//! Idempotency of checkpoint publication comes from the object key, which is a
//! function of the committed `tree_size` (ADR-0003 B.2, spec §11.1 step 8):
//! `checkpoints/{tree_size:016}.signed`.
//!
//! # Anti-drift guard
//!
//! [`SignedCheckpointObject::frame`]'s bytes are tied to the read path by the
//! tests below: they must parse through
//! [`mtc::Checkpoint::parse_tls_presentation`], round-trip to the same fields
//! and signature, and re-serialize to the identical bytes (a fixed point of
//! mtc's own serializer ⇒ byte-identical to mtc's writer).

use mtc::{trust_anchor_id_len_byte, Checkpoint, HashOutput, LogId, SignedAt, TreeSize, Unsigned};

use crate::error::CheckpointSignError;
use crate::P1363_SIGNATURE_LEN;

/// The object-key prefix for signed checkpoints in the object store (spec
/// §8.1): every signed checkpoint is stored under `checkpoints/`.
pub const CHECKPOINT_OBJECT_PREFIX: &str = "checkpoints/";

/// Builds the §8.1 object key for the checkpoint committing to `tree_size`:
/// `checkpoints/{tree_size:016}.signed` (16-digit zero-padded decimal).
///
/// Keyed by `tree_size` and **never** by signature bytes, so re-signing the
/// same checkpoint (HSM ECDSA is randomized) targets the same immutable object
/// — the source of checkpoint-publication idempotency (ADR-0003 B.1/B.2, spec
/// §11.1 step 8).
#[must_use]
pub fn checkpoint_object_key(tree_size: TreeSize) -> String {
    format!("{CHECKPOINT_OBJECT_PREFIX}{:016}.signed", tree_size.0)
}

/// A signed checkpoint ready to publish: its §8.1 object key and framed object
/// bytes (the input to the step-8 commit, spec §11.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedCheckpointObject {
    key: String,
    bytes: Vec<u8>,
    tree_size: TreeSize,
}

impl SignedCheckpointObject {
    /// Frames a signed checkpoint object from an unsigned `checkpoint` and its
    /// HSM `signature` bytes (the 64-byte P1363 `r || s`).
    ///
    /// # Errors
    ///
    /// [`CheckpointSignError::Input`] if the log's `TrustAnchorID` is out of the
    /// 1..=255-byte range, or [`CheckpointSignError::MalformedSignature`] if the
    /// signature is longer than the `opaque<0..2^16-1>` field admits (a 64-byte
    /// signature never is — defence in depth).
    pub(crate) fn frame(
        checkpoint: &Checkpoint<Unsigned>,
        signature: &[u8],
    ) -> Result<Self, CheckpointSignError> {
        let bytes = frame_signed_checkpoint(
            checkpoint.log_id(),
            checkpoint.tree_size(),
            checkpoint.root_hash(),
            checkpoint.signed_at(),
            signature,
        )?;
        Ok(Self {
            key: checkpoint_object_key(checkpoint.tree_size()),
            bytes,
            tree_size: checkpoint.tree_size(),
        })
    }

    /// The §8.1 object key: `checkpoints/{tree_size:016}.signed`.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The framed §8.1 object bytes (parseable by
    /// [`mtc::Checkpoint::parse_tls_presentation`]).
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The committed tree size the object is addressed by.
    #[must_use]
    pub const fn tree_size(&self) -> TreeSize {
        self.tree_size
    }

    /// Consumes the object, returning its `(key, bytes)` for the store put.
    #[must_use]
    pub fn into_parts(self) -> (String, Vec<u8>) {
        (self.key, self.bytes)
    }
}

/// Assembles the §8.1 signed-checkpoint object body, byte-identical to
/// [`mtc::Checkpoint::<Signed>::serialize_tls_presentation`].
///
/// Reuses mtc's [`trust_anchor_id_len_byte`] as the single source of truth for
/// the `TrustAnchorID` 1..=255 length rule (draft §5.4.1); the `uint64` and
/// `opaque<0..2^16-1>` framings are the TLS-presentation primitives written
/// inline (mtc keeps its own copies private).
fn frame_signed_checkpoint(
    log_id: &LogId,
    tree_size: TreeSize,
    root_hash: &HashOutput,
    signed_at: SignedAt,
    signature: &[u8],
) -> Result<Vec<u8>, CheckpointSignError> {
    let id = log_id.as_str().as_bytes();
    // Validates the 1..=255 TrustAnchorID bound and yields the u8 length prefix.
    let taid_len = trust_anchor_id_len_byte(id)?;
    // opaque signature<0..2^16-1>: a 2-byte big-endian length prefix. A 64-byte
    // P1363 signature is far within bounds; the check is defence in depth.
    let sig_len =
        u16::try_from(signature.len()).map_err(|_| CheckpointSignError::MalformedSignature {
            expected: P1363_SIGNATURE_LEN,
            actual: signature.len(),
        })?;

    let mut out = Vec::with_capacity(1 + id.len() + 8 + HashOutput::LEN + 8 + 2 + signature.len());
    out.push(taid_len); // TrustAnchorID length prefix
    out.extend_from_slice(id); // TrustAnchorID bytes
    out.extend_from_slice(&tree_size.0.to_be_bytes()); // uint64 tree_size
    out.extend_from_slice(root_hash.as_bytes()); // HashValue[32]
    out.extend_from_slice(&signed_at.0.to_be_bytes()); // uint64 signed_at
    out.extend_from_slice(&sig_len.to_be_bytes()); // opaque<0..2^16-1> length
    out.extend_from_slice(signature); // signature bytes
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{checkpoint_object_key, SignedCheckpointObject};
    use mtc::{Checkpoint, CheckpointBuilder, HashOutput, LogId, Signed, SignedAt, TreeSize};
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;

    fn unsigned(log: &str, tree_size: u64, root: [u8; 32], signed_at: u64) -> Checkpoint {
        CheckpointBuilder::new(LogId::new(log).unwrap())
            .root_hash(HashOutput(root))
            .tree_size(TreeSize(tree_size))
            .signed_at(SignedAt(signed_at))
            .build()
    }

    #[test]
    fn object_key_is_16_digit_zero_padded_and_addressed_by_tree_size() {
        assert_eq!(
            checkpoint_object_key(TreeSize(2)),
            "checkpoints/0000000000000002.signed"
        );
        assert_eq!(
            checkpoint_object_key(TreeSize(0)),
            "checkpoints/0000000000000000.signed"
        );
        assert_eq!(
            checkpoint_object_key(TreeSize(12_345)),
            "checkpoints/0000000000012345.signed"
        );
        // A 16-digit tree size still fits without truncation.
        assert_eq!(
            checkpoint_object_key(TreeSize(9_999_999_999_999_999)),
            "checkpoints/9999999999999999.signed"
        );
    }

    #[test]
    fn frame_byte_layout_is_pinned() {
        // A hand-built known-answer vector for the exact §8.1 object body:
        // log_id "ca", tree_size 2, all-0x11 root, signed_at 1_700_000_000, and
        // a fixed 64-byte 0xAB signature.
        let signature = [0xAB_u8; 64];
        let cp = unsigned("ca", 2, [0x11; 32], 1_700_000_000);
        let object = SignedCheckpointObject::frame(&cp, &signature).unwrap();
        let bytes = object.bytes();

        let mut expected = Vec::new();
        expected.push(0x02); // TrustAnchorID length = 2
        expected.extend_from_slice(b"ca"); // TrustAnchorID bytes
        expected.extend_from_slice(&2u64.to_be_bytes()); // tree_size
        expected.extend_from_slice(&[0x11u8; 32]); // root_hash
        expected.extend_from_slice(&1_700_000_000u64.to_be_bytes()); // signed_at
        expected.extend_from_slice(&64u16.to_be_bytes()); // signature length = 0x0040
        expected.extend_from_slice(&signature); // signature bytes

        assert_eq!(bytes, expected.as_slice());
        // Total: 1 + 2 + 8 + 32 + 8 + 2 + 64 = 117 bytes.
        assert_eq!(bytes.len(), 117);
    }

    #[test]
    fn framed_object_round_trips_through_mtc_read_path() {
        // Anti-drift oracle: the framed bytes must parse via mtc's read path,
        // round-trip to the same fields + signature, and be a fixed point of
        // mtc's own serializer (⇒ byte-identical to the mtc writer). The parser
        // does not verify the signature, so any 64 bytes exercise the framing.
        let signature = vec![0x5A_u8; 64];
        let cp = unsigned("prod-log", 12_345, [0x7f; 32], 1_700_000_000);
        let object = SignedCheckpointObject::frame(&cp, &signature).unwrap();
        assert_eq!(object.key(), "checkpoints/0000000000012345.signed");
        assert_eq!(object.tree_size(), TreeSize(12_345));

        let parsed = Checkpoint::<Signed>::parse_tls_presentation(object.bytes())
            .expect("mtc must parse our framed object");
        assert_eq!(parsed.log_id().as_str(), "prod-log");
        assert_eq!(parsed.tree_size(), TreeSize(12_345));
        assert_eq!(parsed.root_hash(), &HashOutput([0x7f; 32]));
        assert_eq!(parsed.signed_at(), SignedAt(1_700_000_000));
        assert_eq!(parsed.signature().as_bytes(), signature.as_slice());

        // Fixed point of mtc's serializer ⇒ our framing equals mtc's writer.
        assert_eq!(parsed.serialize_tls_presentation().unwrap(), object.bytes());
    }

    #[test]
    fn frame_rejects_an_out_of_range_trust_anchor_id() {
        // A 300-byte log id cannot be a TrustAnchorID (opaque<1..2^8-1>). The
        // error comes from mtc's single length check, surfaced as `Input`.
        let long = "a".repeat(300);
        let cp = unsigned(&long, 1, [0u8; 32], 0);
        let err = SignedCheckpointObject::frame(&cp, &[0u8; 64]).unwrap_err();
        assert!(matches!(
            err,
            crate::CheckpointSignError::Input(mtc::TrustAnchorIdError { actual: 300 })
        ));
    }

    proptest! {
        /// The framed object equals mtc's TLS-presentation writer for ANY valid
        /// input — not just the fixed KAT vectors above (crypto-review should-fix):
        /// frame → parse → same fields+signature → re-serialize byte-identical,
        /// across all log-id lengths, tree sizes, and signed-at values.
        #[test]
        fn framing_equals_mtc_writer_for_any_valid_input(
            log in "[a-z0-9-]{1,255}",
            tree_size in any::<u64>(),
            root in proptest::array::uniform32(any::<u8>()),
            signed_at in any::<u64>(),
            signature in proptest::collection::vec(any::<u8>(), 64..=64),
        ) {
            let Ok(log_id) = LogId::new(log.as_str()) else { return Ok(()); };
            let cp = CheckpointBuilder::new(log_id)
                .root_hash(HashOutput(root))
                .tree_size(TreeSize(tree_size))
                .signed_at(SignedAt(signed_at))
                .build();
            let object = SignedCheckpointObject::frame(&cp, &signature)
                .expect("a valid <=255-byte log id and 64-byte signature always frame");

            let parsed = Checkpoint::<Signed>::parse_tls_presentation(object.bytes())
                .expect("mtc must parse our framed object");
            prop_assert_eq!(parsed.log_id().as_str(), log.as_str());
            prop_assert_eq!(parsed.tree_size(), TreeSize(tree_size));
            prop_assert_eq!(parsed.root_hash(), &HashOutput(root));
            prop_assert_eq!(parsed.signed_at(), SignedAt(signed_at));
            prop_assert_eq!(parsed.signature().as_bytes(), signature.as_slice());
            prop_assert_eq!(
                parsed.serialize_tls_presentation().expect("mtc re-serializes"),
                object.bytes()
            );
        }
    }
}
