//! The per-certificate log entry and its typestate builder.
//!
//! [`TbsCertificateLogEntry`] is the spec's `TBSCertificateLogEntry` (spec §2
//! concept table) — the "to-be-signed" content a certificate contributes to the
//! issuance log. It is the draft's *abridged assertion*
//! (`draft-ietf-plants-merkle-tree-certs-03` §5.5.1):
//!
//! ```text
//! struct {
//!     SubjectType subject_type;              // uint16
//!     opaque subject_info_hash[hash.length]; // = HASH(subject_info)
//!     Claim claims<0..2^16-1>;
//! }
//! ```
//!
//! The field that matters for the "uses public key hash, not raw key" property
//! (spec §2) is `subject_info_hash`: a [`SubjectInfoHash`], which is a hash of
//! the subject info, **not** the key. There is no field, method, or builder step
//! that accepts a raw [`TlsSubjectInfo`](super::TlsSubjectInfo) — storing a raw
//! key in an entry is unrepresentable in this API.
//!
//! Construction goes through [`TbsCertificateLogEntryBuilder`], a typestate
//! builder (spec §22.2): `.build()` does not exist until both required fields —
//! `subject_type` and `subject_info_hash` — are set. Calling it early is a
//! compile error, proven by `tests/compile_fail/tbs_entry_incomplete_build.rs`.

use std::io::{self, Write};

use crate::wire::{write_vector_u16, TlsParse, TlsReader, TlsSerialize, WireError};

use super::claim::Claim;
use super::subject::{SubjectInfoHash, SubjectType};

/// The per-certificate log entry: the draft's abridged assertion committing to
/// a subject-info **hash** and a set of claims (spec §2; draft-03 §5.5.1).
///
/// Immutable once built. Construct via [`Self::builder`]; the fields are read
/// back through the accessors. Its wire form round-trips through the
/// mtc-serialization framework, and it becomes a Merkle-tree leaf inside a
/// [`LogEntry`](super::LogEntry) (which prepends the entry-type discriminant
/// before the domain-separated leaf hash is taken).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TbsCertificateLogEntry {
    subject_type: SubjectType,
    subject_info_hash: SubjectInfoHash,
    claims: Vec<Claim>,
}

impl TbsCertificateLogEntry {
    /// Starts a typestate builder with no fields set (spec §22.2).
    #[must_use]
    pub const fn builder() -> TbsCertificateLogEntryBuilder {
        TbsCertificateLogEntryBuilder::new()
    }

    /// The subject type (v1: always [`SubjectType::Tls`]).
    #[must_use]
    pub const fn subject_type(&self) -> SubjectType {
        self.subject_type
    }

    /// The commitment to the subject's public key: `HASH(subject_info)`.
    #[must_use]
    pub const fn subject_info_hash(&self) -> &SubjectInfoHash {
        &self.subject_info_hash
    }

    /// The claims this entry asserts.
    #[must_use]
    pub fn claims(&self) -> &[Claim] {
        &self.claims
    }
}

impl TlsSerialize for TbsCertificateLogEntry {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.subject_type.tls_serialize(writer)?;
        self.subject_info_hash.tls_serialize(writer)?;
        write_vector_u16(writer, &self.claims)
    }
}

impl TlsParse for TbsCertificateLogEntry {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        let subject_type = SubjectType::tls_parse(reader)?;
        let subject_info_hash = SubjectInfoHash::tls_parse(reader)?;
        // `claims<0..2^16-1>` has a zero floor, so an empty list is valid here;
        // each individual claim still enforces its own non-empty payload.
        let claims = reader.read_vector_u16::<Claim>()?;
        Ok(Self {
            subject_type,
            subject_info_hash,
            claims,
        })
    }
}

/// Typestate marker: the builder's `subject_type` field is unset.
#[derive(Debug)]
pub struct NoSubjectType;

/// Typestate marker: the builder's `subject_info_hash` field is unset.
#[derive(Debug)]
pub struct NoSubjectInfoHash;

/// A typestate builder for [`TbsCertificateLogEntry`] (spec §22.2).
///
/// The two type parameters track whether the required fields are set. In the
/// initial state `TbsCertificateLogEntryBuilder<NoSubjectType,
/// NoSubjectInfoHash>` there is no `.build()`; each setter transitions its
/// marker to the concrete value type, and only
/// `TbsCertificateLogEntryBuilder<SubjectType, SubjectInfoHash>` — both fields
/// present — exposes [`build`](Self::build). Constructing an incomplete entry is
/// therefore a compile error, not a runtime one.
///
/// Claims are optional (the draft allows `claims<0..2^16-1>`, i.e. none) and can
/// be added in any state via [`claim`](Self::claim) / [`claims`](Self::claims).
#[derive(Debug)]
pub struct TbsCertificateLogEntryBuilder<St = NoSubjectType, Sh = NoSubjectInfoHash> {
    subject_type: St,
    subject_info_hash: Sh,
    claims: Vec<Claim>,
}

impl TbsCertificateLogEntryBuilder {
    /// Creates a builder with no fields set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            subject_type: NoSubjectType,
            subject_info_hash: NoSubjectInfoHash,
            claims: Vec::new(),
        }
    }
}

impl Default for TbsCertificateLogEntryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl<St, Sh> TbsCertificateLogEntryBuilder<St, Sh> {
    /// Appends one claim. Available in any builder state.
    #[must_use]
    pub fn claim(mut self, claim: Claim) -> Self {
        self.claims.push(claim);
        self
    }

    /// Appends every claim from an iterator. Available in any builder state.
    #[must_use]
    pub fn claims(mut self, claims: impl IntoIterator<Item = Claim>) -> Self {
        self.claims.extend(claims);
        self
    }
}

impl<Sh> TbsCertificateLogEntryBuilder<NoSubjectType, Sh> {
    /// Sets the subject type (required). Consumes the `NoSubjectType` state.
    #[must_use]
    pub fn subject_type(
        self,
        subject_type: SubjectType,
    ) -> TbsCertificateLogEntryBuilder<SubjectType, Sh> {
        TbsCertificateLogEntryBuilder {
            subject_type,
            subject_info_hash: self.subject_info_hash,
            claims: self.claims,
        }
    }
}

impl<St> TbsCertificateLogEntryBuilder<St, NoSubjectInfoHash> {
    /// Sets the subject-info hash (required). Consumes the `NoSubjectInfoHash`
    /// state. This takes a [`SubjectInfoHash`], never a raw key — the entry can
    /// only ever commit to the hash (spec §2).
    #[must_use]
    pub fn subject_info_hash(
        self,
        subject_info_hash: SubjectInfoHash,
    ) -> TbsCertificateLogEntryBuilder<St, SubjectInfoHash> {
        TbsCertificateLogEntryBuilder {
            subject_type: self.subject_type,
            subject_info_hash,
            claims: self.claims,
        }
    }
}

impl TbsCertificateLogEntryBuilder<SubjectType, SubjectInfoHash> {
    /// Finalizes the entry. Reachable only once both required fields are set —
    /// the typestate guarantee (spec §22.2).
    #[must_use]
    pub fn build(self) -> TbsCertificateLogEntry {
        TbsCertificateLogEntry {
            subject_type: self.subject_type,
            subject_info_hash: self.subject_info_hash,
            claims: self.claims,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SubjectType, TbsCertificateLogEntry};
    use crate::assert_roundtrip;
    use crate::log_entry::{Claim, DnsName, SubjectInfoHash};
    use crate::wire::TlsParse;
    use crate::HashOutput;

    fn sample_hash() -> SubjectInfoHash {
        SubjectInfoHash::from_hash(HashOutput([0x11; 32]))
    }

    #[test]
    fn builder_produces_entry_with_all_fields() {
        let claim = Claim::dns(vec![DnsName::new(b"example.com".to_vec()).unwrap()]).unwrap();
        let entry = TbsCertificateLogEntry::builder()
            .subject_type(SubjectType::Tls)
            .subject_info_hash(sample_hash())
            .claim(claim.clone())
            .build();
        assert_eq!(entry.subject_type(), SubjectType::Tls);
        assert_eq!(entry.subject_info_hash(), &sample_hash());
        assert_eq!(entry.claims(), &[claim]);
    }

    #[test]
    fn builder_field_order_is_independent() {
        // Setting subject_info_hash before subject_type must also reach build().
        let entry = TbsCertificateLogEntry::builder()
            .subject_info_hash(sample_hash())
            .subject_type(SubjectType::Tls)
            .build();
        assert!(entry.claims().is_empty());
    }

    #[test]
    fn entry_round_trips_with_no_claims() {
        let entry = TbsCertificateLogEntry::builder()
            .subject_type(SubjectType::Tls)
            .subject_info_hash(SubjectInfoHash::from_hash(HashOutput([0xAB; 32])))
            .build();
        // subject_type(0x0000) + 32-byte hash + claims vector length (0x0000).
        let bytes = assert_roundtrip!(entry);
        assert_eq!(bytes.len(), 2 + 32 + 2);
        assert_eq!(&bytes[0..2], &[0x00, 0x00]);
        assert_eq!(&bytes[34..36], &[0x00, 0x00]);
    }

    #[test]
    fn entry_round_trips_with_mixed_claims() {
        use std::net::Ipv4Addr;
        let entry = TbsCertificateLogEntry::builder()
            .subject_type(SubjectType::Tls)
            .subject_info_hash(sample_hash())
            .claim(Claim::dns(vec![DnsName::new(b"a.example".to_vec()).unwrap()]).unwrap())
            .claim(Claim::ipv4(vec![Ipv4Addr::new(203, 0, 113, 5)]).unwrap())
            .build();
        assert_roundtrip!(entry);
    }

    #[test]
    fn truncated_entry_bytes_error_not_panic() {
        // subject_type + a 20-byte (short) hash: the fixed 32-byte read fails.
        let mut bytes = vec![0x00, 0x00];
        bytes.extend_from_slice(&[0u8; 20]);
        assert!(TbsCertificateLogEntry::tls_parse_exact(&bytes).is_err());
    }
}
