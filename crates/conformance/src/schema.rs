//! The JSON+hex test-vector schema (spec §19.4).
//!
//! One vector is one JSON file: `{ "kind": ..., "id": ..., "wire_hex": ...,
//! "parse": { "outcome": "accept" | "reject", ... }, "verify": { ... } }`.
//! `kind` selects which spec structure the vector exercises and, via serde's
//! internally-tagged enum, which of [`CheckpointVector`],
//! [`InclusionProofVector`], or [`LogEntryVector`] the rest of the file must
//! match — so a vector with `"kind": "checkpoint"` but an inclusion-proof
//! `fields` shape is a schema error at load time, not a silent
//! misinterpretation at run time.
//!
//! See `conformance/vectors/README.md` for the human-facing format
//! documentation and worked examples; this module is the machine-checked
//! mirror of that document.

use serde::{Deserialize, Serialize};

/// Whether a vector expects its operation (parse, or verify) to succeed or
/// fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The operation must succeed.
    Accept,
    /// The operation must fail (a "must-reject" vector, spec §19.4 AC).
    Reject,
}

/// What a vector expects from parsing `wire_hex`.
///
/// On [`Outcome::Accept`], `fields` gives the structural values the parsed
/// value must equal (checked field-by-field — see [`crate::runner`]'s
/// per-kind `diff_*_fields` functions). On [`Outcome::Reject`],
/// `error_class` names the expected error: the runner asserts this string
/// appears in the `{:?}` (Debug) rendering of the actual error, so
/// `"TrailingBytes"` matches both a bare `WireError::TrailingBytes` and one
/// nested inside a composite error such as
/// `CheckpointParseError::Wire(WireError::TrailingBytes { .. })`. This is a
/// substring match, not full structural equality, so it stays stable across
/// the offset/length payload a nested error carries — see the vectors
/// README's "`error_class` matching" section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseExpectation<F> {
    /// Accept or reject.
    pub outcome: Outcome,
    /// Required when `outcome` is `accept`; the expected parsed fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<F>,
    /// Required when `outcome` is `reject`; a substring of the expected
    /// error's `Debug` output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
}

/// What a vector expects from a subsequent semantic verification step (run
/// only when [`ParseExpectation::outcome`] is [`Outcome::Accept`] — an input
/// that fails to parse has nothing to verify).
///
/// `material` is the kind-specific data verification needs beyond the parsed
/// value itself (e.g. a verifying key for a checkpoint, or a leaf hash and
/// expected root for an inclusion proof) — it is flattened into the same JSON
/// object as `outcome`/`error_class`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyExpectation<M> {
    /// Accept or reject.
    pub outcome: Outcome,
    /// The kind-specific verification inputs.
    #[serde(flatten)]
    pub material: M,
    /// Required when `outcome` is `reject`; a substring of the expected
    /// error's `Debug` output (same convention as
    /// [`ParseExpectation::error_class`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
}

/// A test vector, tagged by which spec structure it exercises.
///
/// Serde's internally-tagged representation (`#[serde(tag = "kind")]`) reads
/// the JSON file's `"kind"` field and dispatches to the matching variant's own
/// fields — so `Vector` is both the wire schema and the type the runner
/// pattern-matches on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Vector {
    /// A [`mtc::Checkpoint`] vector.
    Checkpoint(CheckpointVector),
    /// A [`mtc::InclusionProof`] vector.
    InclusionProof(InclusionProofVector),
    /// A [`mtc::LogEntry`] vector.
    LogEntry(LogEntryVector),
}

impl Vector {
    /// The vector's `id` field, regardless of kind — used to identify the
    /// vector in failure output (spec §19.4 AC: "Failure output identifies
    /// the vector ID").
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Checkpoint(v) => &v.id,
            Self::InclusionProof(v) => &v.id,
            Self::LogEntry(v) => &v.id,
        }
    }

    /// A stable, human-readable name for the vector's kind (for reporting).
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Checkpoint(_) => "checkpoint",
            Self::InclusionProof(_) => "inclusion_proof",
            Self::LogEntry(_) => "log_entry",
        }
    }

    /// The vector's `description` field, regardless of kind.
    #[must_use]
    pub fn description(&self) -> &str {
        match self {
            Self::Checkpoint(v) => &v.description,
            Self::InclusionProof(v) => &v.description,
            Self::LogEntry(v) => &v.description,
        }
    }

    /// The vector's `parse.outcome`, regardless of kind — whether this is a
    /// happy-path or a must-reject vector.
    #[must_use]
    pub const fn parse_outcome(&self) -> Outcome {
        match self {
            Self::Checkpoint(v) => v.parse.outcome,
            Self::InclusionProof(v) => v.parse.outcome,
            Self::LogEntry(v) => v.parse.outcome,
        }
    }
}

/// A [`mtc::Checkpoint`] test vector: `Checkpoint<Signed>::parse_tls_presentation`
/// on `wire_hex`, then optionally `Checkpoint::verify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointVector {
    /// A unique, stable identifier (also the filename stem by convention).
    pub id: String,
    /// Human-readable summary of what the vector exercises and why.
    pub description: String,
    /// The wire bytes to parse, as lowercase hex with no `0x` prefix or
    /// separators.
    pub wire_hex: String,
    /// The parse expectation.
    pub parse: ParseExpectation<CheckpointFields>,
    /// The optional verify expectation (only meaningful when `parse.outcome`
    /// is `accept`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<VerifyExpectation<CheckpointVerifyMaterial>>,
}

/// Expected field values of a successfully parsed [`mtc::Checkpoint`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointFields {
    /// Expected [`mtc::LogId`] string.
    pub log_id: String,
    /// Expected tree size.
    pub tree_size: u64,
    /// Expected root hash, as 64 lowercase hex characters (32 bytes).
    pub root_hash_hex: String,
    /// Expected `signed_at` (seconds since Unix epoch).
    pub signed_at: u64,
    /// Expected signature bytes, as lowercase hex.
    pub signature_hex: String,
}

/// Verification inputs for a [`mtc::Checkpoint`] vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointVerifyMaterial {
    /// The verifying key's DER `SubjectPublicKeyInfo`, as lowercase hex. The
    /// scheme is always ECDSA P-256 (v1; spec §14.1) — the only algorithm
    /// `mtc::scheme_for` currently resolves.
    pub verifying_key_spki_hex: String,
}

/// A [`mtc::InclusionProof`] test vector: `InclusionProof::tls_parse_exact` on
/// `wire_hex`, then optionally `InclusionProof::verify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InclusionProofVector {
    /// A unique, stable identifier (also the filename stem by convention).
    pub id: String,
    /// Human-readable summary of what the vector exercises and why.
    pub description: String,
    /// The wire bytes to parse, as lowercase hex.
    pub wire_hex: String,
    /// The parse expectation.
    pub parse: ParseExpectation<InclusionProofFields>,
    /// The optional verify expectation (only meaningful when `parse.outcome`
    /// is `accept`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<VerifyExpectation<InclusionProofVerifyMaterial>>,
}

/// Expected field values of a successfully parsed [`mtc::InclusionProof`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InclusionProofFields {
    /// Expected tree size.
    pub tree_size: u64,
    /// Expected leaf index.
    pub leaf_index: u64,
    /// Expected audit path, leaf-to-root, each a 64-character lowercase hex
    /// 32-byte hash.
    pub audit_path_hex: Vec<String>,
}

/// Verification inputs for an [`mtc::InclusionProof`] vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InclusionProofVerifyMaterial {
    /// The leaf hash to verify inclusion of, as lowercase hex (32 bytes) —
    /// i.e. `hash_leaf(entry_bytes)`, not the raw entry bytes.
    pub leaf_hash_hex: String,
    /// The expected root hash to verify against, as lowercase hex (32 bytes).
    pub root_hash_hex: String,
}

/// A [`mtc::LogEntry`] test vector: `LogEntry::tls_parse_exact` on
/// `wire_hex`. Log entries have no self-contained verification step (unlike
/// checkpoints and proofs), so there is no `verify` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntryVector {
    /// A unique, stable identifier (also the filename stem by convention).
    pub id: String,
    /// Human-readable summary of what the vector exercises and why.
    pub description: String,
    /// The wire bytes to parse, as lowercase hex.
    pub wire_hex: String,
    /// The parse expectation.
    pub parse: ParseExpectation<LogEntryFields>,
}

/// Expected field values of a successfully parsed [`mtc::LogEntry`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "variant", rename_all = "snake_case")]
pub enum LogEntryFields {
    /// Expect [`mtc::LogEntry::Null`] (the `null_entry` placeholder).
    Null,
    /// Expect [`mtc::LogEntry::Certificate`] with these field values.
    Certificate {
        /// Expected subject type name (currently always `"tls"`).
        subject_type: String,
        /// Expected `subject_info_hash`, as lowercase hex (32 bytes).
        subject_info_hash_hex: String,
        /// Expected number of claims.
        claim_count: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        CheckpointFields, CheckpointVector, InclusionProofVector, LogEntryFields, LogEntryVector,
        Outcome, ParseExpectation, Vector,
    };

    #[test]
    fn checkpoint_accept_vector_round_trips_through_json() {
        let vector = Vector::Checkpoint(CheckpointVector {
            id: "checkpoint-accept-001".into(),
            description: "example".into(),
            wire_hex: "00".into(),
            parse: ParseExpectation {
                outcome: Outcome::Accept,
                fields: Some(CheckpointFields {
                    log_id: "ca".into(),
                    tree_size: 5,
                    root_hash_hex: "ab".repeat(32),
                    signed_at: 0,
                    signature_hex: "cd".repeat(64),
                }),
                error_class: None,
            },
            verify: None,
        });
        let json = serde_json::to_string_pretty(&vector).unwrap();
        assert!(json.contains("\"kind\": \"checkpoint\""));
        let parsed: Vector = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id(), "checkpoint-accept-001");
        assert_eq!(parsed.kind_name(), "checkpoint");
    }

    #[test]
    fn reject_vector_requires_no_fields() {
        let json = r#"{
            "kind": "inclusion_proof",
            "id": "x",
            "description": "d",
            "wire_hex": "00",
            "parse": { "outcome": "reject", "error_class": "UnexpectedEof" }
        }"#;
        let parsed: Vector = serde_json::from_str(json).unwrap();
        match parsed {
            Vector::InclusionProof(InclusionProofVector { parse, verify, .. }) => {
                assert_eq!(parse.outcome, Outcome::Reject);
                assert!(parse.fields.is_none());
                assert_eq!(parse.error_class.as_deref(), Some("UnexpectedEof"));
                assert!(verify.is_none());
            }
            other => panic!("expected InclusionProof, got {other:?}"),
        }
    }

    #[test]
    fn log_entry_null_variant_round_trips() {
        let json = r#"{
            "kind": "log_entry",
            "id": "log-entry-accept-null",
            "description": "null_entry",
            "wire_hex": "0000",
            "parse": { "outcome": "accept", "fields": { "variant": "null" } }
        }"#;
        let parsed: Vector = serde_json::from_str(json).unwrap();
        match parsed {
            Vector::LogEntry(LogEntryVector { parse, .. }) => {
                assert!(matches!(parse.fields, Some(LogEntryFields::Null)));
            }
            other => panic!("expected LogEntry, got {other:?}"),
        }
    }

    #[test]
    fn unknown_kind_is_a_schema_error_not_a_panic() {
        let json = r#"{"kind": "tile", "id": "x", "description": "d", "wire_hex": "00", "parse": {"outcome": "accept"}}"#;
        assert!(serde_json::from_str::<Vector>(json).is_err());
    }
}
