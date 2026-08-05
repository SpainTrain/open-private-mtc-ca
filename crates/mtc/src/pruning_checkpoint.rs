//! [`PruningCheckpoint`] — a declaration that leaf indices
//! `[pruned_start, pruned_end)` are pruned as of a log of `tree_size` entries.
//!
//! Spec §15.1 ("Pruning is recorded as a signed pruning checkpoint — never
//! silent") and §15.2 ("Pruning checkpoint format: signed declaration of
//! `pruned_range = [start, end) at tree_size T`").
//!
//! # What this type is (and is not)
//!
//! This is the checkpoint's *content* only — the four fields spec §15.2
//! names: the pruned leaf-index range, the tree size the declaration commits
//! to, when it was declared, and which key is meant to sign it. It is
//! deliberately **not** the full signed artifact:
//!
//! - No [`Signature`](crate::Signature) field, and no [`Signed`]/[`Unsigned`]
//!   typestate the way [`Checkpoint`](crate::Checkpoint) has one. Producing a
//!   signature over this content is out of scope (`prune-checkpoint-signer`);
//!   a `PruningCheckpoint` is always the to-be-signed content.
//! - No persistence or commit sequencing (`prune-commit-protocol`).
//!
//! [`signing_key_id`](PruningCheckpoint::signing_key_id) only *names* the key
//! a future signer will use — a phantom-typed identifier (spec §22.5), not a
//! key handle. It is modeled, carried through the wire codec, and otherwise
//! untouched here.
//!
//! [`Signed`]: crate::checkpoint::Signed
//! [`Unsigned`]: crate::checkpoint::Unsigned
//!
//! # Invariants (spec §15.2)
//!
//! [`PruningCheckpoint::try_new`] is the *only* constructor (every field is
//! private), and it enforces both range invariants an untrusted declaration
//! must satisfy:
//!
//! - `pruned_start <= pruned_end` — the half-open range is not inverted;
//! - `pruned_end <= tree_size` — the declaration cannot prune entries the
//!   committed tree does not yet contain.
//!
//! A `PruningCheckpoint` violating either is **unconstructible**: there is no
//! code path that builds one without going through `try_new`. Parsing applies
//! the identical checks to untrusted wire bytes via
//! [`TlsParse::tls_parse`], rejecting a violation as
//! [`WireError::InvalidValue`] rather than ever constructing the invalid value
//! or panicking (spec §19.3).
//!
//! # Wire format (spec §19.3 TLS-presentation style)
//!
//! ```text
//! struct {
//!     uint64 pruned_start;
//!     uint64 pruned_end;
//!     uint64 tree_size;
//!     uint64 pruned_at;
//!     opaque signing_key_id<1..2^16-1>;
//! } PruningCheckpoint;
//! ```
//!
//! `signing_key_id` hand-enforces a non-empty floor (crypto F3 / bead
//! `mtc-qka.3`, mirroring [`Checkpoint`](crate::Checkpoint)'s `TrustAnchorID`
//! handling): the generic [`TlsReader::read_opaque_u16`] admits a zero-length
//! opaque, but an empty id could never name a real signing key, so it is
//! rejected at parse rather than accepted as an ambiguity to resolve later.

use std::io::{self, Write};

use thiserror::Error;

use crate::types::{Id, Index, TreeSize};
use crate::wire::{write_bytes, write_opaque_u16, TlsParse, TlsReader, TlsSerialize, WireError};

/// Phantom tag for [`SigningKeyId`] (spec §22.5 phantom-typed identifiers).
///
/// Never instantiated; exists only as the type parameter of [`SigningKeyId`],
/// which keeps it a distinct compile-time type from [`LogId`](crate::LogId)
/// and [`BatchId`](crate::BatchId) even though all three are `String`-backed
/// at runtime.
pub struct SigningKeyTag;

/// Identifier of the key meant to sign a [`PruningCheckpoint`] (spec §15.2).
///
/// An opaque name — e.g. an HSM key alias or ARN — resolved to an actual key
/// by the future `prune-checkpoint-signer` bead. This ticket carries the
/// field through the domain type and wire codec only: no signature is
/// produced, checked, or even representable on [`PruningCheckpoint`].
pub type SigningKeyId = Id<SigningKeyTag>;

/// A pruning checkpoint's declaration timestamp: seconds since the Unix epoch
/// (mirrors [`SignedAt`](crate::checkpoint::SignedAt); spec §15.1, "Pruning is
/// recorded ... never silent").
///
/// A distinct newtype (rule `use-newtypes`, spec §22.1): a raw `u64` cannot be
/// confused with a [`TreeSize`] or a leaf [`Index`]. Supplied by the caller
/// from the injected `Clock` (rule `no-systemtime-now-in-prod`); this crate
/// never reads the wall clock.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct PrunedAt(pub u64);

/// Errors constructing a [`PruningCheckpoint`] through its checked constructor
/// ([`PruningCheckpoint::try_new`]).
///
/// `try_new` is the *only* way to build a `PruningCheckpoint` (every field is
/// private), so a value violating either variant here is unconstructible in
/// memory, not merely discouraged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PruningCheckpointError {
    /// `pruned_start > pruned_end`: the half-open range `[start, end)` is
    /// inverted and has no valid length (spec §15.2).
    #[error("inverted pruned range: start {start} > end {end}")]
    InvertedRange {
        /// The (larger) inclusive lower bound that was supplied.
        start: u64,
        /// The (smaller) exclusive upper bound that was supplied.
        end: u64,
    },
    /// `pruned_end > tree_size`: the declaration claims to prune entries the
    /// committed tree does not yet contain (spec §15.2 `pruned_range =
    /// [start, end) at tree_size T`; a valid declaration always has `end <=
    /// T`).
    #[error("pruned range end {end} exceeds tree_size {tree_size}")]
    RangeExceedsTreeSize {
        /// The exclusive upper bound of the pruned range.
        end: u64,
        /// The committed tree size the declaration is made at.
        tree_size: u64,
    },
}

/// Validates spec §15.2's two range invariants.
///
/// Shared by [`PruningCheckpoint::try_new`] (the in-memory constructor) and
/// [`TlsParse::tls_parse`] (the untrusted wire boundary), so there is exactly
/// one place either check is expressed.
const fn validate_range(
    pruned_start: Index,
    pruned_end: Index,
    tree_size: TreeSize,
) -> Result<(), PruningCheckpointError> {
    if pruned_start.0 > pruned_end.0 {
        return Err(PruningCheckpointError::InvertedRange {
            start: pruned_start.0,
            end: pruned_end.0,
        });
    }
    if pruned_end.0 > tree_size.0 {
        return Err(PruningCheckpointError::RangeExceedsTreeSize {
            end: pruned_end.0,
            tree_size: tree_size.0,
        });
    }
    Ok(())
}

/// A declaration that leaf indices `[pruned_start, pruned_end)` are pruned as
/// of a log of `tree_size` entries (spec §15.1, §15.2). See the module docs
/// for what this type does and does not cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruningCheckpoint {
    pruned_start: Index,
    pruned_end: Index,
    tree_size: TreeSize,
    pruned_at: PrunedAt,
    signing_key_id: SigningKeyId,
}

impl PruningCheckpoint {
    /// Builds a pruning checkpoint declaration, enforcing spec §15.2's range
    /// invariants.
    ///
    /// # Errors
    ///
    /// - [`PruningCheckpointError::InvertedRange`] if `pruned_start >
    ///   pruned_end`.
    /// - [`PruningCheckpointError::RangeExceedsTreeSize`] if `pruned_end >
    ///   tree_size`.
    pub fn try_new(
        pruned_start: Index,
        pruned_end: Index,
        tree_size: TreeSize,
        pruned_at: PrunedAt,
        signing_key_id: SigningKeyId,
    ) -> Result<Self, PruningCheckpointError> {
        validate_range(pruned_start, pruned_end, tree_size)?;
        Ok(Self {
            pruned_start,
            pruned_end,
            tree_size,
            pruned_at,
            signing_key_id,
        })
    }

    /// The inclusive lower bound of the pruned leaf-index range.
    #[must_use]
    pub const fn pruned_start(&self) -> Index {
        self.pruned_start
    }

    /// The exclusive upper bound of the pruned leaf-index range.
    #[must_use]
    pub const fn pruned_end(&self) -> Index {
        self.pruned_end
    }

    /// The number of leaf indices the declaration covers (`pruned_end -
    /// pruned_start`).
    ///
    /// Never wraps: [`try_new`](Self::try_new) proves `pruned_start <=
    /// pruned_end` before a `PruningCheckpoint` can exist.
    #[must_use]
    pub const fn pruned_len(&self) -> u64 {
        self.pruned_end.0 - self.pruned_start.0
    }

    /// The committed tree size this declaration is made at (spec §15.2 `T`).
    #[must_use]
    pub const fn tree_size(&self) -> TreeSize {
        self.tree_size
    }

    /// When this declaration was made (spec §15.1: pruning is never silent).
    #[must_use]
    pub const fn pruned_at(&self) -> PrunedAt {
        self.pruned_at
    }

    /// The identifier of the key meant to sign this declaration.
    ///
    /// Not verified, consumed, or otherwise interpreted here — see the module
    /// docs.
    #[must_use]
    pub const fn signing_key_id(&self) -> &SigningKeyId {
        &self.signing_key_id
    }
}

impl TlsSerialize for PruningCheckpoint {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_u64(writer, self.pruned_start.0)?;
        write_u64(writer, self.pruned_end.0)?;
        write_u64(writer, self.tree_size.0)?;
        write_u64(writer, self.pruned_at.0)?;
        write_opaque_u16(writer, self.signing_key_id.as_str().as_bytes())
    }
}

impl TlsParse for PruningCheckpoint {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        let pruned_start = Index(read_u64(reader)?);
        let pruned_end = Index(read_u64(reader)?);
        let tree_size = TreeSize(read_u64(reader)?);
        // Every field needed to check the range invariants has now been read;
        // fail fast here (before parsing `pruned_at`/`signing_key_id`) rather
        // than build a value only to reject it after the fact.
        let range_offset = reader.position();
        validate_range(pruned_start, pruned_end, tree_size).map_err(|err| {
            WireError::InvalidValue {
                offset: range_offset,
                reason: match err {
                    PruningCheckpointError::InvertedRange { .. } => {
                        "pruned range is inverted: start > end"
                    }
                    PruningCheckpointError::RangeExceedsTreeSize { .. } => {
                        "pruned range end exceeds tree_size"
                    }
                },
            }
        })?;
        let pruned_at = PrunedAt(read_u64(reader)?);
        let signing_key_id = read_signing_key_id(reader)?;
        Ok(Self {
            pruned_start,
            pruned_end,
            tree_size,
            pruned_at,
            signing_key_id,
        })
    }
}

/// Writes a `uint64` in big-endian TLS-presentation form (8 bytes).
///
/// The wire framework ships `u8`/`u16`/`u24`/`u32` primitives but not `u64`,
/// which `tree_size` and the leaf-index fields need. This is the same local
/// helper [`Checkpoint`](crate::checkpoint)'s codec defines (see that
/// module's identical comment) — duplicated here until the framework grows a
/// shared `u64`.
fn write_u64<W: Write>(writer: &mut W, value: u64) -> io::Result<()> {
    write_bytes(writer, &value.to_be_bytes())
}

/// Reads a big-endian `uint64` (8 bytes) through the bounded reader.
fn read_u64(reader: &mut TlsReader<'_>) -> Result<u64, WireError> {
    Ok(u64::from_be_bytes(reader.read_array::<8>()?))
}

/// Reads the `signing_key_id` field (`opaque<1..2^16-1>`), hand-enforcing the
/// non-empty floor the generic [`TlsReader::read_opaque_u16`] does not police
/// (crypto F3 / bead `mtc-qka.3`, mirroring
/// [`Checkpoint`](crate::checkpoint)'s `TrustAnchorID` handling) and decoding
/// the bytes as UTF-8.
fn read_signing_key_id(reader: &mut TlsReader<'_>) -> Result<SigningKeyId, WireError> {
    let offset = reader.position();
    let bytes = reader.read_opaque_u16()?;
    if bytes.is_empty() {
        return Err(WireError::InvalidValue {
            offset,
            reason: "signing_key_id must be 1..=65535 bytes",
        });
    }
    let text = core::str::from_utf8(bytes).map_err(|_| WireError::InvalidValue {
        offset,
        reason: "signing_key_id is not valid UTF-8",
    })?;
    // `text` is non-empty (checked above), so `SigningKeyId::new` cannot fail
    // with `Empty`; map the unreachable error to the same class rather than
    // unwrapping (rule `no-unwrap-in-prod`).
    SigningKeyId::new(text).map_err(|_| WireError::InvalidValue {
        offset,
        reason: "signing_key_id is not valid UTF-8",
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{PrunedAt, PruningCheckpoint, PruningCheckpointError, SigningKeyId};
    use crate::types::{Index, TreeSize};
    use crate::wire::{TlsParse, TlsSerialize, WireError};

    fn key(id: &str) -> SigningKeyId {
        SigningKeyId::new(id).unwrap()
    }

    fn sample() -> PruningCheckpoint {
        PruningCheckpoint::try_new(
            Index(2),
            Index(5),
            TreeSize(10),
            PrunedAt(1_700_000_000),
            key("prune-key-1"),
        )
        .unwrap()
    }

    // --- construction ---------------------------------------------------

    #[test]
    fn try_new_populates_every_field_and_accessors_read_back() {
        let cp = sample();
        assert_eq!(cp.pruned_start(), Index(2));
        assert_eq!(cp.pruned_end(), Index(5));
        assert_eq!(cp.pruned_len(), 3);
        assert_eq!(cp.tree_size(), TreeSize(10));
        assert_eq!(cp.pruned_at(), PrunedAt(1_700_000_000));
        assert_eq!(cp.signing_key_id().as_str(), "prune-key-1");
    }

    #[test]
    fn try_new_accepts_boundary_ranges() {
        // pruned_end == tree_size: pruning the entire committed tree.
        assert!(PruningCheckpoint::try_new(
            Index(0),
            Index(10),
            TreeSize(10),
            PrunedAt(0),
            key("k"),
        )
        .is_ok());
        // pruned_start == pruned_end: an empty (no-op) declaration.
        assert!(PruningCheckpoint::try_new(
            Index(4),
            Index(4),
            TreeSize(10),
            PrunedAt(0),
            key("k")
        )
        .is_ok());
    }

    #[test]
    fn try_new_rejects_inverted_range() {
        assert_eq!(
            PruningCheckpoint::try_new(Index(9), Index(4), TreeSize(20), PrunedAt(0), key("k"))
                .unwrap_err(),
            PruningCheckpointError::InvertedRange { start: 9, end: 4 },
        );
    }

    #[test]
    fn try_new_rejects_end_past_tree_size() {
        assert_eq!(
            PruningCheckpoint::try_new(Index(0), Index(11), TreeSize(10), PrunedAt(0), key("k"))
                .unwrap_err(),
            PruningCheckpointError::RangeExceedsTreeSize {
                end: 11,
                tree_size: 10,
            },
        );
    }

    // --- wire codec -------------------------------------------------------

    #[test]
    fn wire_layout_is_byte_exact_and_round_trips() {
        let cp = PruningCheckpoint::try_new(
            Index(1),
            Index(3),
            TreeSize(5),
            PrunedAt(9),
            key("ca-key-1"),
        )
        .unwrap();
        let bytes = cp.tls_serialize_to_vec().unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&1u64.to_be_bytes());
        expected.extend_from_slice(&3u64.to_be_bytes());
        expected.extend_from_slice(&5u64.to_be_bytes());
        expected.extend_from_slice(&9u64.to_be_bytes());
        expected.extend_from_slice(&[0x00, 0x08]);
        expected.extend_from_slice(b"ca-key-1");
        assert_eq!(bytes, expected);

        let parsed = PruningCheckpoint::tls_parse_exact(&bytes).unwrap();
        assert_eq!(cp, parsed);
    }

    #[test]
    fn parse_rejects_inverted_range() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&9u64.to_be_bytes()); // pruned_start
        bytes.extend_from_slice(&4u64.to_be_bytes()); // pruned_end
        bytes.extend_from_slice(&20u64.to_be_bytes()); // tree_size
        bytes.extend_from_slice(&0u64.to_be_bytes()); // pruned_at
        bytes.extend_from_slice(&[0x00, 0x01, b'k']); // signing_key_id "k"
        assert_eq!(
            PruningCheckpoint::tls_parse_exact(&bytes),
            Err(WireError::InvalidValue {
                offset: 24,
                reason: "pruned range is inverted: start > end",
            }),
        );
    }

    #[test]
    fn parse_rejects_range_past_tree_size() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u64.to_be_bytes()); // pruned_start
        bytes.extend_from_slice(&11u64.to_be_bytes()); // pruned_end
        bytes.extend_from_slice(&10u64.to_be_bytes()); // tree_size
        bytes.extend_from_slice(&0u64.to_be_bytes()); // pruned_at
        bytes.extend_from_slice(&[0x00, 0x01, b'k']);
        assert_eq!(
            PruningCheckpoint::tls_parse_exact(&bytes),
            Err(WireError::InvalidValue {
                offset: 24,
                reason: "pruned range end exceeds tree_size",
            }),
        );
    }

    #[test]
    fn parse_rejects_empty_signing_key_id() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&[0x00, 0x00]); // zero-length signing_key_id
        assert_eq!(
            PruningCheckpoint::tls_parse_exact(&bytes),
            Err(WireError::InvalidValue {
                offset: 32,
                reason: "signing_key_id must be 1..=65535 bytes",
            }),
        );
    }

    #[test]
    fn parse_rejects_non_utf8_signing_key_id() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&[0x00, 0x01, 0xFF]); // 1 byte, invalid UTF-8
        assert_eq!(
            PruningCheckpoint::tls_parse_exact(&bytes),
            Err(WireError::InvalidValue {
                offset: 32,
                reason: "signing_key_id is not valid UTF-8",
            }),
        );
    }

    #[test]
    fn parse_rejects_truncated_and_trailing_bytes() {
        let good = sample().tls_serialize_to_vec().unwrap();

        // Truncated deep inside the fixed-width header: a plain short read,
        // never a panic.
        let truncated = &good[..5];
        assert!(matches!(
            PruningCheckpoint::tls_parse_exact(truncated),
            Err(WireError::UnexpectedEof { .. }),
        ));

        // Truncated inside the trailing variable-length `signing_key_id`
        // body: the already-read length prefix now claims more than remains,
        // so this is a `LengthOverflow`, not an `UnexpectedEof` — still a
        // structured error, never a panic.
        let short_body = &good[..good.len() - 1];
        assert!(matches!(
            PruningCheckpoint::tls_parse_exact(short_body),
            Err(WireError::LengthOverflow { .. }),
        ));

        // Trailing: append a stray byte -> TrailingBytes.
        let mut trailing = good.clone();
        trailing.push(0x99);
        assert!(matches!(
            PruningCheckpoint::tls_parse_exact(&trailing),
            Err(WireError::TrailingBytes { .. }),
        ));
    }

    proptest! {
        // Spec §19.2: parse(serialize(x)) == x for every well-formed value.
        #[test]
        fn pruning_checkpoint_roundtrip(
            start in any::<u64>(),
            extra in any::<u64>(),
            headroom in any::<u64>(),
            pruned_at in any::<u64>(),
            key_id in "[ -~]{1,64}",
        ) {
            // Derive a well-formed start <= end <= tree_size from three
            // independent arbitrary values via saturating arithmetic, so
            // every generated case is a valid PruningCheckpoint.
            let end = start.saturating_add(extra);
            let tree_size = end.saturating_add(headroom);
            let cp = PruningCheckpoint::try_new(
                Index(start),
                Index(end),
                TreeSize(tree_size),
                PrunedAt(pruned_at),
                SigningKeyId::new(key_id).unwrap(),
            )
            .unwrap();
            let bytes = cp.tls_serialize_to_vec().unwrap();
            let parsed = PruningCheckpoint::tls_parse_exact(&bytes).unwrap();
            prop_assert_eq!(cp, parsed);
        }

        // Spec §19.3: parsing arbitrary bytes returns Ok or Err, never panics.
        #[test]
        fn parse_arbitrary_bytes_never_panics(
            bytes in prop::collection::vec(any::<u8>(), 0..256),
        ) {
            let _ = PruningCheckpoint::tls_parse_exact(&bytes);
        }

        // AC: try_new never admits an invalid range, for any inputs.
        #[test]
        fn try_new_never_admits_an_invalid_range(
            start in any::<u64>(),
            end in any::<u64>(),
            tree_size in any::<u64>(),
        ) {
            let result = PruningCheckpoint::try_new(
                Index(start),
                Index(end),
                TreeSize(tree_size),
                PrunedAt(0),
                SigningKeyId::new("k").unwrap(),
            );
            if start > end || end > tree_size {
                prop_assert!(result.is_err());
            } else {
                prop_assert!(result.is_ok());
            }
        }
    }
}
