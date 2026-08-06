//! The source-agnostic submission envelope: [`LogEntry`], [`SourceType`], and
//! [`SourceId`] (spec §10.2).
//!
//! These are the Stage-1/Stage-2 boundary types: whatever an adapter is (the
//! native ACME endpoint today; AWS Private CA, Keyfactor, or Cloudflare's PCA
//! as pure additions later, spec §10.1, §10.4), it hands the CA exactly one
//! [`LogEntry`] per issuance event. Nothing about *how* the entry was
//! produced crosses this boundary — see the [`EntryIntake`](crate::EntryIntake)
//! trait docs for the adapter contract.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Which adapter produced a [`LogEntry`] (spec §10.2, §10.4).
///
/// The spec's own example values — `"native-acme"`, `"aws-pca-adapter"`,
/// `"etc."` — describe an *open* set of provenance tags, not a fixed list:
/// §10.4's whole point is that "future adapters are pure additions". A closed
/// Rust enum naming every adapter (`AwsPca`, `Keyfactor`, `CloudflarePca`,
/// ...) would force every future non-ACME adapter ticket to modify this seam
/// crate just to add a variant — exactly the "small refactor" §10.4 says v1
/// should avoid paying twice. So only the v1 adapter gets a first-class
/// variant; everything else is data, carried by [`SourceType::Adapter`].
///
/// No ACME-specific *type* appears here (or anywhere in this crate) — only
/// the [`SourceType::NativeAcme`] label, which is data, not a dependency on
/// `acme-core`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceType {
    /// The v1 native ACME endpoint (spec §10.1) — the only adapter that
    /// exists today. Corresponds to the spec pseudocode's `"native-acme"`.
    NativeAcme,
    /// Any adapter other than the native ACME endpoint, identified by its own
    /// type tag (e.g. `"aws-pca-adapter"`, `"keyfactor-adapter"` — spec
    /// §10.1's examples). New adapters are additions at the call site; this
    /// enum never needs to grow to admit one (spec §10.4).
    Adapter(String),
}

/// An adapter's external reference for one issuance event (spec §10.2).
///
/// Opaque from the CA's point of view — an ACME order URL, a webhook
/// delivery ID, a source CA's certificate serial, whatever the adapter's own
/// system uses to name the event. Newtyped (rule `use-newtypes`) so it can
/// never be passed where some other `String`-shaped field is expected as
/// [`SourceType`], [`LogEntry`], and future batch-state code accumulate more
/// string-valued fields.
///
/// Deliberately infallible to construct, unlike `mtc::Id`'s non-empty check:
/// validating (or not) an external reference is a source-specific policy call
/// that belongs to the adapter (spec §10.3 point 2), not to this
/// source-agnostic seam.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceId(pub String);

impl SourceId {
    /// Borrows the reference as a plain string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SourceId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SourceId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// A source-agnostic entry submitted to the issuance log intake queue (spec
/// §10.2).
///
/// This is the value every adapter — native ACME included — hands to
/// [`EntryIntake::submit_entry`](crate::EntryIntake::submit_entry). It is
/// *not* the Merkle-tree leaf itself: `tbs_cert` carries the leaf's
/// not-yet-admitted, already-serialized bytes (what the spec pseudocode types
/// as `TBSCert []byte // serialized TBSCertificateLogEntry`); turning those
/// bytes into a tree leaf, allocating an [`Index`](mtc::Index), and hashing
/// them into the tree is Stage 2 (the batch builder, tree updater — spec
/// §10, out of scope for this seam).
///
/// `source_type` and `source_id` are not consumed here — they flow through
/// unexamined so the batch state item can persist them (spec §8.2's
/// `batch#{batchId}` item carries `source_type`/`source_id` attributes),
/// giving audit traceability back to the originating issuance event (spec
/// §10.2, §10.3 point 5's "idempotency").
///
/// `submitted_at` is supplied by the caller (typically read from an injected
/// `Arc<dyn Clock>` — rule `no-systemtime-now-in-prod`); this crate never
/// reads ambient time itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Serialized `TbsCertificateLogEntry` bytes (spec §10.2's `TBSCert
    /// []byte`), constructed by the adapter (spec §10.3 point 3). Opaque to
    /// this crate: framing and hashing into the tree are Stage 2's job.
    pub tbs_cert: Vec<u8>,
    /// Which adapter produced this entry.
    pub source_type: SourceType,
    /// The adapter's external reference for this issuance event.
    pub source_id: SourceId,
    /// When the entry was submitted, per the caller's injected clock.
    pub submitted_at: SystemTime,
}

impl LogEntry {
    /// Builds a [`LogEntry`] from its four constituent fields (spec §10.2).
    ///
    /// A thin constructor — `LogEntry`'s fields are `pub` and struct-literal
    /// construction works too — provided so call sites read as one
    /// expression instead of a field-by-field literal.
    #[must_use]
    pub const fn new(
        tbs_cert: Vec<u8>,
        source_type: SourceType,
        source_id: SourceId,
        submitted_at: SystemTime,
    ) -> Self {
        Self {
            tbs_cert,
            source_type,
            source_id,
            submitted_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::{LogEntry, SourceId, SourceType};

    #[test]
    fn source_id_from_str_and_string_agree() {
        assert_eq!(
            SourceId::from("order-1"),
            SourceId::from("order-1".to_string())
        );
        assert_eq!(SourceId::from("order-1").as_str(), "order-1");
    }

    #[test]
    fn native_acme_and_adapter_variants_are_distinct() {
        // Two different adapters (or an adapter vs. native ACME) with the
        // same-shaped payload must not compare equal — SourceType is part of
        // the audit trail (spec §10.2), so conflating sources would corrupt
        // it.
        assert_ne!(
            SourceType::NativeAcme,
            SourceType::Adapter("native-acme".to_string())
        );
        assert_ne!(
            SourceType::Adapter("aws-pca-adapter".to_string()),
            SourceType::Adapter("keyfactor-adapter".to_string())
        );
    }

    #[test]
    fn new_constructs_expected_fields() {
        let entry = LogEntry::new(
            vec![1, 2, 3],
            SourceType::NativeAcme,
            SourceId::from("order-42"),
            UNIX_EPOCH,
        );
        assert_eq!(entry.tbs_cert, vec![1, 2, 3]);
        assert_eq!(entry.source_type, SourceType::NativeAcme);
        assert_eq!(entry.source_id, SourceId::from("order-42"));
        assert_eq!(entry.submitted_at, UNIX_EPOCH);
    }
}
