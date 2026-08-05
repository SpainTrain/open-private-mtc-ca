//! The subject side of a log entry: its type, the TLS subject info, and the
//! **public-key hash** the entry commits to instead of the raw key.
//!
//! Spec section 2 (concept table): "`TBSCertificateLogEntry` — per-cert log
//! entry; uses public key hash, not raw key". The mechanism is the draft's
//! *abridged assertion* (`draft-ietf-plants-merkle-tree-certs-03` §5.5.1):
//! where a full assertion carries `opaque subject_info<0..2^16-1>` (the TLS
//! subject info, which contains the end-entity public key), the abridged form
//! that goes into the log carries `opaque subject_info_hash[hash.length]` —
//! `subject_info_hash = HASH(subject_info)`. Only the hash is logged, so the
//! log never stores, and this API never lets an entry hold, a raw public key.
//!
//! This module provides:
//! - [`SubjectType`] — the `SubjectType` enum (draft-03 §4), `tls(0)` for v1.
//! - [`TlsSubjectInfo`] — the pre-hash TLS subject info (`signature` scheme +
//!   `public_key`), the value one hashes to obtain a commitment.
//! - [`SubjectInfoHash`] — the fixed-width hash newtype the entry stores. It
//!   has no constructor from a raw key; you either wrap an already-computed hash
//!   or [hash a `TlsSubjectInfo`](TlsSubjectInfo::subject_info_hash).

use std::io::{self, Write};

use crate::tree::Hasher;
use crate::types::HashOutput;
use crate::wire::{write_opaque_u16, write_u16, TlsParse, TlsReader, TlsSerialize, WireError};

use super::error::EntryError;

/// The subject type of an assertion (`SubjectType` enum,
/// `draft-ietf-plants-merkle-tree-certs-03` §4).
///
/// A closed set (spec §22.3): v1 defines only `tls(0)`. The wire encoding is a
/// `uint16` (the draft's `(2^16-1)` maximum). An unrecognized value parses to
/// [`WireError::InvalidValue`], never a panic (spec §22.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SubjectType {
    /// A TLS subject: the subject info is a [`TlsSubjectInfo`] (draft-03 §4,
    /// `tls(0)`).
    Tls,
}

impl SubjectType {
    /// The on-wire `uint16` codepoint for this subject type.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Tls => 0,
        }
    }

    /// Resolves a `uint16` codepoint to a subject type, or `None` if unknown.
    #[must_use]
    pub const fn from_code(code: u16) -> Option<Self> {
        match code {
            0 => Some(Self::Tls),
            _ => None,
        }
    }
}

impl TlsSerialize for SubjectType {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_u16(writer, self.code())
    }
}

impl TlsParse for SubjectType {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        let code = reader.read_u16()?;
        Self::from_code(code).ok_or_else(|| WireError::InvalidValue {
            offset: reader.position(),
            reason: "unknown SubjectType codepoint",
        })
    }
}

/// A commitment to a subject's public key: `HASH(subject_info)` (spec §2).
///
/// This is the fixed 32-byte `subject_info_hash` an abridged assertion stores in
/// place of the raw key (`draft-ietf-plants-merkle-tree-certs-03` §5.5.1).
///
/// This newtype is the whole point of "uses public key hash, not raw key": a
/// [`TbsCertificateLogEntry`](super::TbsCertificateLogEntry) holds a
/// `SubjectInfoHash`, and there is deliberately **no** API to put a
/// [`TlsSubjectInfo`] (which contains the key) into an entry — you must hash it
/// first. Construct one by [wrapping a precomputed hash](Self::from_hash) or by
/// [hashing a `TlsSubjectInfo`](TlsSubjectInfo::subject_info_hash).
///
/// On the wire it is `opaque subject_info_hash[hash.length]` — a fixed-width
/// field with no length prefix (draft-03 §5.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubjectInfoHash(HashOutput);

impl SubjectInfoHash {
    /// Wraps an already-computed `HASH(subject_info)` digest.
    ///
    /// Use this when the hash was produced elsewhere (e.g. by the issuance
    /// pipeline). To compute it from the subject info directly, use
    /// [`TlsSubjectInfo::subject_info_hash`] or [`Self::of_subject_info`].
    #[must_use]
    pub const fn from_hash(hash: HashOutput) -> Self {
        Self(hash)
    }

    /// Computes `HASH(subject_info)` over already-serialized subject-info bytes.
    ///
    /// `subject_info` is the wire encoding of the assertion's `subject_info`
    /// field (for a TLS subject, a serialized [`TlsSubjectInfo`]). The hash is
    /// the log's raw `HASH` with no tree domain-separation prefix — the draft
    /// defines `subject_info_hash = HASH(subject_info)` plainly (draft-03
    /// §5.5.1); the entry that embeds it is what later gets the leaf prefix.
    #[must_use]
    pub fn of_subject_info<H: Hasher>(subject_info: &[u8]) -> Self {
        Self(H::digest(&[subject_info]))
    }

    /// Borrows the underlying digest.
    #[must_use]
    pub const fn as_hash(&self) -> &HashOutput {
        &self.0
    }
}

impl TlsSerialize for SubjectInfoHash {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // Fixed-width opaque[32]: the raw digest bytes, no length prefix.
        self.0.as_bytes().tls_serialize(writer)
    }
}

impl TlsParse for SubjectInfoHash {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        let bytes = reader.read_array::<{ HashOutput::LEN }>()?;
        Ok(Self(HashOutput(bytes)))
    }
}

/// The TLS subject info (`draft-ietf-plants-merkle-tree-certs-03` §4, `tls(0)`
/// subject): the end-entity's signature scheme and public key.
///
/// This is the *pre-hash* value — it holds the raw `public_key`. It never
/// enters a log entry directly; you hash it into a [`SubjectInfoHash`] via
/// [`Self::subject_info_hash`] and the entry commits only to that hash (spec §2,
/// "uses public key hash, not raw key"). It is modelled here so tests and the
/// issuance pipeline can derive a commitment from a real key and so the
/// subject-info wire form round-trips.
///
/// Wire form (`opaque subject_info` body for a TLS subject):
/// `uint16 signature; opaque public_key<1..2^16-1>;`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TlsSubjectInfo {
    signature_scheme: u16,
    public_key: Vec<u8>,
}

impl TlsSubjectInfo {
    /// Creates TLS subject info from a signature-scheme codepoint and a
    /// non-empty public key.
    ///
    /// `signature_scheme` is a raw IANA TLS `SignatureScheme` codepoint (the
    /// subject may use any TLS scheme, not just this CA's signing algorithms),
    /// so it is not narrowed to a closed enum here.
    ///
    /// # Errors
    ///
    /// [`EntryError::EmptyPublicKey`] if `public_key` is empty — `public_key`
    /// is `opaque<1..2^16-1>` (draft-03 §4), a hand-enforced minimum-length
    /// field (crypto F3 / bead `mtc-qka.3`).
    pub fn new(signature_scheme: u16, public_key: impl Into<Vec<u8>>) -> Result<Self, EntryError> {
        let public_key = public_key.into();
        if public_key.is_empty() {
            return Err(EntryError::EmptyPublicKey);
        }
        Ok(Self {
            signature_scheme,
            public_key,
        })
    }

    /// The subject's IANA TLS `SignatureScheme` codepoint.
    #[must_use]
    pub const fn signature_scheme(&self) -> u16 {
        self.signature_scheme
    }

    /// Borrows the subject's public key bytes.
    #[must_use]
    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }

    /// Computes this subject info's commitment: `HASH(subject_info)`.
    ///
    /// Serializes the subject info and hashes it into the [`SubjectInfoHash`]
    /// the log entry stores (draft-03 §5.5.1). Serialization of a well-formed
    /// [`TlsSubjectInfo`] cannot fail (`public_key` is bounded well within its
    /// `u16` prefix), but the encoder is fallible in general, so the failure is
    /// surfaced rather than hashing truncated bytes (a sign-the-wrong-bytes
    /// hazard; see the wire-module Finding-1 note).
    ///
    /// # Errors
    ///
    /// Propagates an encoding failure from [`TlsSerialize::tls_serialize_to_vec`]
    /// (only if `public_key` overflowed its length prefix, i.e. exceeded
    /// `u16::MAX` bytes).
    pub fn subject_info_hash<H: Hasher>(&self) -> io::Result<SubjectInfoHash> {
        let bytes = self.tls_serialize_to_vec()?;
        Ok(SubjectInfoHash::of_subject_info::<H>(&bytes))
    }
}

impl TlsSerialize for TlsSubjectInfo {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_u16(writer, self.signature_scheme)?;
        write_opaque_u16(writer, &self.public_key)
    }
}

impl TlsParse for TlsSubjectInfo {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        let signature_scheme = reader.read_u16()?;
        let public_key = reader.read_opaque_u16()?;
        // Hand-enforced `public_key<1..2^16-1>` floor (crypto F3): the generic
        // reader accepts a zero-length opaque; the draft does not.
        if public_key.is_empty() {
            return Err(WireError::InvalidValue {
                offset: reader.position(),
                reason: "TLS subject public_key must be at least one byte",
            });
        }
        Ok(Self {
            signature_scheme,
            public_key: public_key.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{SubjectInfoHash, SubjectType, TlsSubjectInfo};
    use crate::tree::Sha256Hasher;
    use crate::wire::{TlsParse, TlsSerialize, WireError};
    use crate::{assert_roundtrip, HashOutput};

    #[test]
    fn subject_type_codepoint_round_trips() {
        assert_eq!(SubjectType::Tls.code(), 0);
        assert_eq!(SubjectType::from_code(0), Some(SubjectType::Tls));
        assert_eq!(SubjectType::from_code(1), None);
        assert_roundtrip!(SubjectType::Tls, [0x00, 0x00]);
    }

    #[test]
    fn unknown_subject_type_is_rejected_not_panicked() {
        let parsed = SubjectType::tls_parse_exact(&[0x00, 0x09]);
        assert!(
            matches!(parsed, Err(WireError::InvalidValue { .. })),
            "{parsed:?}"
        );
    }

    #[test]
    fn subject_info_hash_fixed_width_round_trips() {
        let sih = SubjectInfoHash::from_hash(HashOutput([0x5a; 32]));
        let bytes = assert_roundtrip!(sih);
        // opaque[32]: exactly the digest, no length prefix.
        assert_eq!(bytes.len(), 32);
        assert_eq!(bytes, vec![0x5a; 32]);
    }

    #[test]
    fn subject_info_hash_is_hash_of_subject_info() {
        let si = TlsSubjectInfo::new(0x0403, vec![0x04, 0x01, 0x02, 0x03]).unwrap();
        let serialized = si.tls_serialize_to_vec().unwrap();
        let expected = SubjectInfoHash::of_subject_info::<Sha256Hasher>(&serialized);
        assert_eq!(si.subject_info_hash::<Sha256Hasher>().unwrap(), expected);
        // The commitment is HASH(subject_info): distinct from HASH(public_key).
        let over_key = SubjectInfoHash::of_subject_info::<Sha256Hasher>(si.public_key());
        assert_ne!(expected, over_key);
    }

    #[test]
    fn tls_subject_info_round_trips() {
        let si = TlsSubjectInfo::new(0x0807, vec![0xAA, 0xBB, 0xCC]).unwrap();
        assert_roundtrip!(si);
    }

    #[test]
    fn tls_subject_info_rejects_empty_public_key_on_construction() {
        assert_eq!(
            TlsSubjectInfo::new(0x0403, Vec::new()).unwrap_err(),
            super::EntryError::EmptyPublicKey,
        );
    }

    #[test]
    fn tls_subject_info_rejects_empty_public_key_on_parse() {
        // uint16 signature (0x0403) + u16 length prefix 0x0000 (empty key).
        let parsed = TlsSubjectInfo::tls_parse_exact(&[0x04, 0x03, 0x00, 0x00]);
        assert!(
            matches!(
                parsed,
                Err(WireError::InvalidValue {
                    reason: "TLS subject public_key must be at least one byte",
                    ..
                })
            ),
            "{parsed:?}"
        );
    }
}
