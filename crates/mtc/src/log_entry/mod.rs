//! Issuance-log entries: the per-certificate entry, its claims, the
//! [`null_entry`] placeholder, and entry-to-leaf hashing.
//!
//! Models spec §2 and `draft-ietf-plants-merkle-tree-certs-03` §4, §5.3, §5.5.1.
//!
//! # What a leaf commits to
//!
//! The Merkle tree of the issuance log (spec §2, "Issuance log") has one leaf
//! per index. The leaf's content is a [`LogEntry`] — the draft's
//! `MerkleTreeCertEntry` (draft-03 §5.3), a discriminated union of exactly two
//! shapes:
//!
//! ```text
//! enum { null_entry(0), tbs_cert_entry(1), (2^16-1) } MerkleTreeCertEntryType;
//! struct {
//!     MerkleTreeCertEntryType type;
//!     select (type) {
//!         case null_entry:     Empty;                 // {}
//!         case tbs_cert_entry: TbsCertificateLogEntry; // the per-cert entry
//!     }
//! }
//! ```
//!
//! - [`LogEntry::Certificate`] wraps a [`TbsCertificateLogEntry`], which commits
//!   to a **hash of the subject's public key** (a [`SubjectInfoHash`]), never
//!   the raw key (spec §2). See [`entry`].
//! - [`LogEntry::Null`] is the [`null_entry`]: the spec-defined placeholder that
//!   fills the index gaps left by abandoned batches (spec §2, §13.3; write-path
//!   §11.2, "abandoned indices become permanent gaps filled with `null_entry`").
//!
//! # Leaf hashing and null-entry unforgeability (security-relevant)
//!
//! [`LogEntry::leaf_hash`] serializes the whole entry — **type discriminant
//! first** — and feeds it to the domain-separated [`hash_leaf`](crate::hash_leaf)
//! from the tree layer: `leaf = HASH(0x00 || type || body)`.
//!
//! The discriminant is a second, entry-level domain separation on top of the
//! tree's `0x00` leaf prefix. A `null_entry` serializes to exactly `00 00`
//! (`type = null_entry(0)`, `Empty` body); every real certificate entry
//! serializes to `00 01 …` (`type = tbs_cert_entry(1)`, non-empty body). The two
//! preimages differ in their second byte for *all* inputs, so:
//!
//! - a `null_entry` leaf hash can never equal a certificate leaf hash (absent a
//!   SHA-256 collision), and
//! - no certificate entry can be crafted to serialize as `00 00` — the
//!   discriminant is fixed by the variant, not attacker-chosen.
//!
//! A `null_entry` therefore cannot forge inclusion of a real certificate, and a
//! real certificate cannot masquerade as a gap. This is asserted below by the
//! `null_entry_*` unit tests (which lock the exact bytes and leaf hash) and the
//! `null_entry_never_collides_with_certificate` property test.

mod claim;
mod entry;
mod error;
mod subject;

use std::io::{self, Write};

use crate::leaf::LeafBytes;
use crate::tree::{hash_leaf, Hasher};
use crate::types::HashOutput;
use crate::wire::{write_u16, TlsParse, TlsReader, TlsSerialize, WireError};

pub use claim::{Claim, ClaimType, DnsName};
pub use entry::{
    NoSubjectInfoHash, NoSubjectType, TbsCertificateLogEntry, TbsCertificateLogEntryBuilder,
};
pub use error::EntryError;
pub use subject::{SubjectInfoHash, SubjectType, TlsSubjectInfo};

/// The on-wire `MerkleTreeCertEntryType` discriminant for a `null_entry`
/// (draft-03 §5.3).
const NULL_ENTRY_TYPE: u16 = 0;

/// The on-wire `MerkleTreeCertEntryType` discriminant for a `tbs_cert_entry`
/// (draft-03 §5.3).
const TBS_CERT_ENTRY_TYPE: u16 = 1;

/// The content of one issuance-log leaf: the draft's `MerkleTreeCertEntry`
/// (draft-03 §5.3), either a certificate entry or the [`null_entry`]
/// placeholder.
///
/// This is the value that gets [leaf-hashed](Self::leaf_hash) into the Merkle
/// tree. Serializing it emits the type discriminant followed by the variant
/// body (nothing for `Null`), so the two variants are wire-distinguishable —
/// the property `null_entry` relies on to be unforgeable (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogEntry {
    /// The `null_entry` placeholder (`type = null_entry(0)`, `Empty` body).
    Null,
    /// A per-certificate entry (`type = tbs_cert_entry(1)`).
    Certificate(TbsCertificateLogEntry),
}

impl LogEntry {
    /// Returns the spec-defined `null_entry` placeholder. See [`null_entry`].
    #[must_use]
    pub const fn null() -> Self {
        Self::Null
    }

    /// Wraps a per-certificate entry as a log leaf.
    #[must_use]
    pub const fn certificate(entry: TbsCertificateLogEntry) -> Self {
        Self::Certificate(entry)
    }

    /// Whether this is the [`null_entry`] placeholder.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The exact bytes this entry commits as a Merkle-tree leaf: its
    /// serialization with the entry-type discriminant first (`00 00` for
    /// `null_entry`, `00 01…` for a certificate entry).
    ///
    /// This is the sanctioned way to obtain a [`LeafBytes`] and the *only*
    /// public producer of one: hand the result to
    /// [`MerkleTree::append`](crate::MerkleTree::append) and the bytes the CA
    /// commits are framed by the **same** serialization a relying party
    /// reconstructs and verifies against. There is deliberately no way to
    /// append un-framed bytes (crypto audit 2026-08-05, Finding 2).
    ///
    /// [`leaf_hash`](Self::leaf_hash) is defined as `HASH(0x00 || leaf_bytes)`,
    /// so the write-path preimage and the read-path leaf hash can never drift.
    ///
    /// # Errors
    ///
    /// Propagates an encoding failure from [`TlsSerialize::tls_serialize_to_vec`]
    /// (a claim body overflowing its `u16` length prefix). For
    /// [`LogEntry::Null`] this is infallible.
    pub fn leaf_bytes(&self) -> io::Result<LeafBytes> {
        Ok(LeafBytes::from_framed(self.tls_serialize_to_vec()?))
    }

    /// Computes this entry's Merkle-tree leaf hash: `HASH(0x00 || entry)`.
    ///
    /// The entry is serialized (type discriminant first, via
    /// [`leaf_bytes`](Self::leaf_bytes)) and hashed with the domain-separated
    /// [`hash_leaf`](crate::hash_leaf) from the tree layer (spec §19.2
    /// leaf/interior separation). It shares one serialization call with
    /// `leaf_bytes`, so the hash a relying party checks is taken over exactly
    /// the bytes the CA appends. Returning a `Result` rather than hashing
    /// eagerly upholds the wire-module contract: an entry that fails to encode
    /// (a claim body overflowing its `u16` prefix) must surface the error, never
    /// be leaf-hashed from truncated bytes — a CA that hashed the wrong bytes
    /// would commit to a certificate it did not mean to (wire-module Finding-1
    /// note).
    ///
    /// # Errors
    ///
    /// Propagates an encoding failure from [`TlsSerialize::tls_serialize_to_vec`].
    /// For [`LogEntry::Null`] this is infallible.
    pub fn leaf_hash<H: Hasher>(&self) -> io::Result<HashOutput> {
        Ok(hash_leaf::<H>(self.leaf_bytes()?.as_bytes()))
    }
}

impl TlsSerialize for LogEntry {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            // null_entry: type = null_entry(0), Empty body (no further bytes).
            Self::Null => write_u16(writer, NULL_ENTRY_TYPE),
            // tbs_cert_entry: type = tbs_cert_entry(1) then the entry body.
            Self::Certificate(entry) => {
                write_u16(writer, TBS_CERT_ENTRY_TYPE)?;
                entry.tls_serialize(writer)
            }
        }
    }
}

impl TlsParse for LogEntry {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        let entry_type = reader.read_u16()?;
        match entry_type {
            NULL_ENTRY_TYPE => Ok(Self::Null),
            TBS_CERT_ENTRY_TYPE => Ok(Self::Certificate(TbsCertificateLogEntry::tls_parse(
                reader,
            )?)),
            _ => Err(WireError::InvalidValue {
                offset: reader.position(),
                reason: "unknown MerkleTreeCertEntry type",
            }),
        }
    }
}

/// The spec-defined `null_entry` placeholder (spec §2, §13.3; draft-03 §5.3).
///
/// `null_entry` fills the index gaps left by abandoned batches so the issuance
/// log's counter never has to decrease (write-path §11.2). It is a well-defined
/// constant: it serializes to exactly the two bytes `00 00` and its leaf hash is
/// deterministic. It cannot be confused with, or forged from, a real
/// certificate entry — see the module docs for the argument and the
/// `null_entry_*` tests for the locked values.
#[must_use]
pub const fn null_entry() -> LogEntry {
    LogEntry::Null
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use proptest::prelude::*;

    use super::{null_entry, Claim, DnsName, LogEntry, SubjectInfoHash, SubjectType};
    use crate::log_entry::entry::TbsCertificateLogEntry;
    use crate::tree::{hash_leaf, Sha256Hasher};
    use crate::wire::{TlsParse, TlsSerialize};
    use crate::{assert_roundtrip, HashOutput};

    // --- null_entry: locked bytes and leaf hash ------------------------------

    #[test]
    fn null_entry_serializes_to_two_zero_bytes() {
        // Spec AC: null_entry is a well-defined constant whose serialization
        // matches the draft; this locks its bytes (draft-03 §5.3: type =
        // null_entry(0), Empty body).
        assert_roundtrip!(null_entry(), [0x00, 0x00]);
        assert!(null_entry().is_null());
    }

    #[test]
    fn null_entry_leaf_hash_is_locked() {
        // Deterministic leaf hash = HASH(0x00 || 0x00 0x00), locked as a KAT.
        let leaf = null_entry().leaf_hash::<Sha256Hasher>().unwrap();
        let expected = hash_leaf::<Sha256Hasher>(&[0x00, 0x00]);
        assert_eq!(leaf, expected);
        assert_eq!(
            format!("{leaf:?}"),
            "HashOutput(709e80c88487a2411e1ee4dfb9f22a861492d20c4765150c0c794abd70f8147c)",
        );
    }

    // --- distinguishability / unforgeability ---------------------------------

    fn sample_certificate() -> LogEntry {
        LogEntry::certificate(
            TbsCertificateLogEntry::builder()
                .subject_type(SubjectType::Tls)
                .subject_info_hash(SubjectInfoHash::from_hash(HashOutput([0x11; 32])))
                .claim(Claim::dns(vec![DnsName::new(b"example.com".to_vec()).unwrap()]).unwrap())
                .build(),
        )
    }

    #[test]
    fn certificate_entry_has_distinct_discriminant() {
        // Real cert entries start with tbs_cert_entry(1); null_entry with (0).
        let cert_bytes = sample_certificate().tls_serialize_to_vec().unwrap();
        assert_eq!(&cert_bytes[0..2], &[0x00, 0x01]);
        assert_eq!(
            null_entry().tls_serialize_to_vec().unwrap(),
            vec![0x00, 0x00]
        );
    }

    #[test]
    fn null_entry_leaf_hash_differs_from_a_certificate() {
        let null_leaf = null_entry().leaf_hash::<Sha256Hasher>().unwrap();
        let cert_leaf = sample_certificate().leaf_hash::<Sha256Hasher>().unwrap();
        assert_ne!(null_leaf, cert_leaf);
    }

    // --- property tests ------------------------------------------------------

    fn arb_dns_name() -> impl Strategy<Value = DnsName> {
        proptest::collection::vec(any::<u8>(), 1..=32)
            .prop_map(|b| DnsName::new(b).expect("1..=32 bytes is within DNSName<1..255>"))
    }

    fn arb_claim() -> impl Strategy<Value = Claim> {
        prop_oneof![
            proptest::collection::vec(arb_dns_name(), 1..4)
                .prop_map(|n| Claim::dns(n).expect("non-empty")),
            proptest::collection::vec(arb_dns_name(), 1..4)
                .prop_map(|n| Claim::dns_wildcard(n).expect("non-empty")),
            proptest::collection::vec(any::<[u8; 4]>(), 1..4).prop_map(|a| Claim::ipv4(
                a.into_iter().map(Ipv4Addr::from).collect::<Vec<_>>()
            )
            .expect("non-empty")),
            proptest::collection::vec(any::<[u8; 16]>(), 1..4).prop_map(|a| Claim::ipv6(
                a.into_iter().map(Ipv6Addr::from).collect::<Vec<_>>()
            )
            .expect("non-empty")),
        ]
    }

    fn arb_certificate_entry() -> impl Strategy<Value = TbsCertificateLogEntry> {
        (
            any::<[u8; 32]>(),
            proptest::collection::vec(arb_claim(), 0..4),
        )
            .prop_map(|(hash, claims)| {
                TbsCertificateLogEntry::builder()
                    .subject_type(SubjectType::Tls)
                    .subject_info_hash(SubjectInfoHash::from_hash(HashOutput(hash)))
                    .claims(claims)
                    .build()
            })
    }

    proptest! {
        // Spec §19.2: parse(serialize(x)) == x for every entry and claim.
        #[test]
        fn claim_round_trips(claim in arb_claim()) {
            let bytes = claim.tls_serialize_to_vec().expect("claim encodes");
            let parsed = Claim::tls_parse_exact(&bytes);
            prop_assert_eq!(parsed.as_ref(), Ok(&claim));
        }

        #[test]
        fn log_entry_round_trips(entry in arb_certificate_entry()) {
            let leaf = LogEntry::certificate(entry);
            let bytes = leaf.tls_serialize_to_vec().expect("entry encodes");
            let parsed = LogEntry::tls_parse_exact(&bytes);
            prop_assert_eq!(parsed.as_ref(), Ok(&leaf));
        }

        // Security AC: a null_entry can never forge inclusion of a real cert.
        // For ANY certificate entry, the serializations differ (discriminant)
        // and therefore the leaf hashes differ (absent a SHA-256 collision).
        #[test]
        fn null_entry_never_collides_with_certificate(entry in arb_certificate_entry()) {
            let cert = LogEntry::certificate(entry);
            let cert_bytes = cert.tls_serialize_to_vec().expect("entry encodes");
            let null_bytes = null_entry().tls_serialize_to_vec().expect("null encodes");
            prop_assert_ne!(&cert_bytes, &null_bytes);
            prop_assert_eq!(&cert_bytes[0..2], &[0x00, 0x01]);
            prop_assert_ne!(
                null_entry().leaf_hash::<Sha256Hasher>().unwrap(),
                cert.leaf_hash::<Sha256Hasher>().unwrap(),
            );
        }

        // Spec §19.3: arbitrary bytes never panic the parser; they parse or
        // return a structured error.
        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = LogEntry::tls_parse_exact(&bytes);
            let _ = Claim::tls_parse_exact(&bytes);
            let _ = TbsCertificateLogEntry::tls_parse_exact(&bytes);
        }
    }
}
