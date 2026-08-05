//! The domain-separated signed payload of a checkpoint signature —
//! `MTCSubtreeSignatureInput` (`draft-ietf-plants-merkle-tree-certs-03`
//! §5.4.1). **This is the crown-jewel byte construction of this ticket.**
//!
//! A cosigner (in this deployment, the CA itself — spec §1 is cosigner-free)
//! signs *subtrees* of the issuance log. The bytes it signs are never the raw
//! `(tree_size, root_hash)`: they are wrapped in a fixed, versioned,
//! domain-separating structure so a signature over a checkpoint can never be
//! confused with a signature over anything else (a different label, a different
//! log, or a different byte string entirely). Getting these bytes exactly right
//! is the whole security value of the signature (crypto invariants, spec
//! §19.6).
//!
//! # The structure (draft §5.4.1), verbatim
//!
//! ```text
//! opaque HashValue[HASH_SIZE];          // 32 bytes for SHA-256 (draft §5.1)
//! opaque TrustAnchorID<1..2^8-1>;       // 1..=255 bytes, u8 length prefix
//!
//! struct {
//!     TrustAnchorID log_id;
//!     uint64 start;
//!     uint64 end;
//!     HashValue hash;
//! } MTCSubtree;
//!
//! struct {
//!     uint8 label[16] = "mtc-subtree/v1\n\0";
//!     TrustAnchorID cosigner_id;
//!     MTCSubtree subtree;
//! } MTCSubtreeSignatureInput;
//! ```
//!
//! A *checkpoint* signature is the special case `start == 0`: it commits to the
//! whole prefix `[0, tree_size)` of the log (draft §5.4.1: "When `start` is
//! zero, the resulting signature describes the checkpoint with tree size `end`
//! and is also known as a checkpoint signature").
//!
//! # Why `mtclib-checkpoint` owns this and `mtclib-signing` does not
//!
//! ADR-0003 (Decision B.3) fixes the boundary: the [`SignatureScheme`] trait
//! signs a raw `message: &[u8]` with **no** context/domain-separation
//! parameter, so "constructing the domain-separated `MTCSubtreeSignatureInput`
//! (the `mtc-subtree/v1` label, cosigner ID, and subtree, per draft §5.4.1) is
//! owned solely by `mtclib-checkpoint` — [the signing] crate signs the
//! already-assembled bytes." These functions *are* that assembly step.
//!
//! [`SignatureScheme`]: crate::signing::SignatureScheme
//!
//! # The `cosigner_id` / `log_id` identity (single-trust-boundary reading)
//!
//! `MTCSubtreeSignatureInput` names two `TrustAnchorID`s: the `cosigner_id` of
//! the entity producing the signature, and the `log_id` of the log inside the
//! `MTCSubtree`. In a general MTC deployment these can differ (an independent
//! cosigner following someone else's log). This CA has **no external
//! cosigners** (spec §1): it signs checkpoints of its *own* log, so the two IDs
//! are the same value — the CA's trust-anchor ID, carried by the
//! [`Checkpoint`](crate::Checkpoint)'s [`LogId`](crate::LogId). Both slots are
//! still emitted (the bytes must match the draft structure exactly); they
//! simply carry the same identifier here.
//!
//! # `TrustAnchorID` and the missing type (`mtclib-trust-anchor-id`)
//!
//! The dedicated `TrustAnchorId` type lands in a separate ticket
//! (`mtclib-trust-anchor-id`) that this ticket does not depend on. Until then a
//! checkpoint carries a [`LogId`](crate::LogId) (a non-empty UTF-8 string, spec
//! §22.5) and its UTF-8 bytes fill the `TrustAnchorID` opaque slot. That slot
//! is `opaque<1..2^8-1>`: 1..=255 bytes. The **minimum length is hand-enforced**
//! here — the generic wire reader admits a zero-length opaque, but the draft
//! forbids it (crypto F3 / bead `mtc-qka.3`), so [`trust_anchor_id_len_byte`]
//! and the checkpoint parser both reject an empty identifier.

use std::io::{self, Write};

use thiserror::Error;

use crate::types::HashOutput;
use crate::wire::write_bytes;

/// The 16-byte domain-separation label of a subtree signature (draft §5.4.1):
/// the ASCII string `mtc-subtree/v1`, a newline (`U+000A`), and a zero byte
/// (`U+0000`).
///
/// Exposed so conformance vectors (`mtclib-conformance-suite`) and the
/// differential oracle (`mtclib-differential-go-oracle`) can assert the exact
/// bytes. The `[u8; 16]` type makes the length a compile-time invariant: were
/// the literal not exactly 16 bytes, this line would not type-check.
pub const SUBTREE_SIGNATURE_LABEL: [u8; 16] = *b"mtc-subtree/v1\n\0";

/// The fixed byte length of a `HashValue` (`HASH_SIZE` for SHA-256, draft §5.1;
/// [`HashOutput::LEN`](crate::HashOutput::LEN)).
const HASH_VALUE_LEN: usize = HashOutput::LEN;

/// A `TrustAnchorID` (`opaque<1..2^8-1>`) was empty or longer than 255 bytes.
///
/// The bound is the draft's (§5.4.1): a trust-anchor identifier is 1..=255
/// bytes. This is the *minimum-length* enforcement the generic wire framework
/// cannot express (its opaque reader admits length 0), hand-enforced here
/// because a zero-length or over-long identifier must never reach the signer as
/// a silently-truncated or empty field (crypto F3 / bead `mtc-qka.3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error(
    "trust-anchor id is {actual} byte(s); a TrustAnchorID must be 1..=255 \
     (draft §5.4.1 opaque<1..2^8-1>)"
)]
pub struct TrustAnchorIdError {
    /// The rejected length.
    pub actual: usize,
}

/// Validates a `TrustAnchorID`'s length (`opaque<1..2^8-1>`, draft §5.4.1) and
/// returns its `u8` length prefix.
///
/// This is the single place the 1..=255 bound is checked on the write/sign
/// path. Returning the prefix as a `u8` (via [`u8::try_from`], never an `as`
/// cast) means the caller writes a length that is provably in range.
///
/// # Errors
///
/// [`TrustAnchorIdError`] if `id` is empty (below the minimum) or longer than
/// 255 bytes (above the maximum).
pub fn trust_anchor_id_len_byte(id: &[u8]) -> Result<u8, TrustAnchorIdError> {
    if id.is_empty() {
        return Err(TrustAnchorIdError { actual: 0 });
    }
    u8::try_from(id.len()).map_err(|_| TrustAnchorIdError { actual: id.len() })
}

/// Writes a `TrustAnchorID` (`opaque<1..2^8-1>`, draft §5.4.1) to `writer`.
///
/// Emits the `u8` length prefix then the identifier bytes, enforcing the
/// 1..=255 bound before writing anything.
///
/// # Errors
///
/// [`io::ErrorKind::InvalidData`] if `id` violates the 1..=255 length bound
/// (the draft minimum/maximum), plus any I/O error from `writer`.
pub fn write_trust_anchor_id<W: Write>(writer: &mut W, id: &[u8]) -> io::Result<()> {
    let len =
        trust_anchor_id_len_byte(id).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_bytes(writer, &[len])?;
    write_bytes(writer, id)
}

/// Assembles the exact bytes of a `MTCSubtreeSignatureInput` (draft §5.4.1) —
/// the message a cosigner signs over a subtree `[start, end)` of `log_id`.
///
/// For a **checkpoint** signature `start` is `0` and `end` is the tree size
/// (draft §5.4.1); [`Checkpoint::signature_input`](crate::Checkpoint::signature_input)
/// is the checkpoint-specialized caller. `cosigner_id` and `log_id` are the
/// producing cosigner's and the signed log's trust-anchor IDs respectively —
/// equal for this cosigner-free CA (see the module docs).
///
/// The layout, in order, is:
/// `label[16] || len(cosigner_id) || cosigner_id || len(log_id) || log_id ||
/// start_be[8] || end_be[8] || hash[32]`.
///
/// # Errors
///
/// [`TrustAnchorIdError`] if either identifier is not 1..=255 bytes
/// (`opaque<1..2^8-1>`).
pub fn subtree_signature_input(
    cosigner_id: &[u8],
    log_id: &[u8],
    start: u64,
    end: u64,
    hash: &HashOutput,
) -> Result<Vec<u8>, TrustAnchorIdError> {
    // Validate both identifiers up front so nothing is written on rejection.
    let cosigner_len = trust_anchor_id_len_byte(cosigner_id)?;
    let log_len = trust_anchor_id_len_byte(log_id)?;

    let mut out = Vec::with_capacity(
        SUBTREE_SIGNATURE_LABEL.len()
            + 1
            + cosigner_id.len()
            + 1
            + log_id.len()
            + 8
            + 8
            + HASH_VALUE_LEN,
    );
    // uint8 label[16] = "mtc-subtree/v1\n\0"
    out.extend_from_slice(&SUBTREE_SIGNATURE_LABEL);
    // TrustAnchorID cosigner_id (opaque<1..2^8-1>)
    out.push(cosigner_len);
    out.extend_from_slice(cosigner_id);
    // MTCSubtree.log_id (opaque<1..2^8-1>)
    out.push(log_len);
    out.extend_from_slice(log_id);
    // MTCSubtree.start / MTCSubtree.end (uint64, big-endian)
    out.extend_from_slice(&start.to_be_bytes());
    out.extend_from_slice(&end.to_be_bytes());
    // MTCSubtree.hash (HashValue, opaque[32])
    out.extend_from_slice(hash.as_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{
        subtree_signature_input, trust_anchor_id_len_byte, TrustAnchorIdError,
        SUBTREE_SIGNATURE_LABEL,
    };
    use crate::types::HashOutput;

    #[test]
    fn label_is_the_exact_16_bytes_of_the_draft() {
        // "mtc-subtree/v1" (14) + '\n' (0x0a) + '\0' (0x00) = 16 bytes.
        assert_eq!(SUBTREE_SIGNATURE_LABEL.len(), 16);
        assert_eq!(
            SUBTREE_SIGNATURE_LABEL,
            [
                b'm', b't', b'c', b'-', b's', b'u', b'b', b't', b'r', b'e', b'e', b'/', b'v', b'1',
                0x0a, 0x00,
            ],
        );
        // The first 14 bytes are the ASCII label; the last two are the
        // separators the draft spells out.
        assert_eq!(&SUBTREE_SIGNATURE_LABEL[..14], b"mtc-subtree/v1");
        assert_eq!(SUBTREE_SIGNATURE_LABEL[14], b'\n');
        assert_eq!(SUBTREE_SIGNATURE_LABEL[15], b'\0');
    }

    #[test]
    fn trust_anchor_id_length_bound_is_hand_enforced() {
        // Minimum: empty is rejected (the generic opaque reader would allow it).
        assert_eq!(
            trust_anchor_id_len_byte(&[]),
            Err(TrustAnchorIdError { actual: 0 }),
        );
        // In range: 1 and 255 bytes both yield their exact length prefix.
        assert_eq!(trust_anchor_id_len_byte(&[0u8; 1]), Ok(1));
        assert_eq!(trust_anchor_id_len_byte(&[0u8; 255]), Ok(255));
        // Maximum: 256 bytes is rejected.
        assert_eq!(
            trust_anchor_id_len_byte(&[0u8; 256]),
            Err(TrustAnchorIdError { actual: 256 }),
        );
    }

    #[test]
    fn subtree_signature_input_byte_layout_is_pinned() {
        // A hand-computed known-answer vector for the exact §5.4.1 bytes. Both
        // identifiers are the ASCII "ca" (0x63 0x61); the subtree is the
        // checkpoint [0, 2) with an all-0x11 root hash.
        let hash = HashOutput([0x11u8; 32]);
        let bytes = subtree_signature_input(b"ca", b"ca", 0, 2, &hash).expect("valid ids");

        let mut expected = Vec::new();
        expected.extend_from_slice(b"mtc-subtree/v1\n\0"); // label[16]
        expected.extend_from_slice(&[0x02, b'c', b'a']); //   cosigner_id
        expected.extend_from_slice(&[0x02, b'c', b'a']); //   log_id
        expected.extend_from_slice(&0u64.to_be_bytes()); //   start = 0
        expected.extend_from_slice(&2u64.to_be_bytes()); //   end   = 2
        expected.extend_from_slice(&[0x11u8; 32]); //         hash

        assert_eq!(bytes, expected);
        // Total: 16 + 3 + 3 + 8 + 8 + 32 = 70 bytes.
        assert_eq!(bytes.len(), 70);
    }

    #[test]
    fn subtree_signature_input_rejects_out_of_range_ids() {
        let hash = HashOutput([0u8; 32]);
        // Empty cosigner_id.
        assert_eq!(
            subtree_signature_input(&[], b"ca", 0, 1, &hash),
            Err(TrustAnchorIdError { actual: 0 }),
        );
        // Over-long log_id.
        assert_eq!(
            subtree_signature_input(b"ca", &[0u8; 256], 0, 1, &hash),
            Err(TrustAnchorIdError { actual: 256 }),
        );
    }

    #[test]
    fn distinct_subtrees_produce_distinct_signed_bytes() {
        let hash = HashOutput([0x11u8; 32]);
        let base = subtree_signature_input(b"ca", b"ca", 0, 2, &hash).unwrap();
        // Changing tree_size (end), the root hash, or the log id each changes
        // the signed bytes — the properties that make tampering detectable.
        assert_ne!(
            base,
            subtree_signature_input(b"ca", b"ca", 0, 3, &hash).unwrap()
        );
        assert_ne!(
            base,
            subtree_signature_input(b"ca", b"ca", 0, 2, &HashOutput([0x22u8; 32])).unwrap(),
        );
        assert_ne!(
            base,
            subtree_signature_input(b"cb", b"cb", 0, 2, &hash).unwrap()
        );
    }
}
