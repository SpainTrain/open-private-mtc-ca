//! Evaluates one loaded [`Vector`] against the real `mtc` crate types.
//!
//! This is the conformance layer itself (spec §19.4): each `evaluate_*`
//! function runs the vector's `wire_hex` through the matching `mtc` type's
//! real parser (never a hand-rolled reimplementation) and, on an `accept`
//! expectation, through the type's real `verify` when the vector supplies
//! verification material. A mismatch is reported as a **structural diff** —
//! one line per differing field, expected vs. actual — rather than a bare
//! `assert_eq!` failure (spec §19.4 AC: "Failure output ... shows a
//! structural diff, not just an assertion failure"). [`evaluate`] never
//! panics on a malformed or hostile vector body; every failure mode returns
//! `Err(String)` with the diagnostic text `tests/conformance.rs` prints.

use std::fmt::{Debug, Write as _};

use mtc::{
    Checkpoint, EcdsaP256, HashOutput, InclusionProof, LogEntry, Sha256Hasher, SignatureAlgorithm,
    Signed, SubjectType, TlsParse, VerifyingKey,
};

use crate::hex;
use crate::schema::{
    CheckpointFields, CheckpointVector, CheckpointVerifyMaterial, InclusionProofFields,
    InclusionProofVector, InclusionProofVerifyMaterial, LogEntryFields, LogEntryVector, Outcome,
    Vector, VerifyExpectation,
};

/// One field that differed between a vector's expectation and the actual
/// parsed value.
#[derive(Debug, Clone)]
struct FieldDiff {
    field: String,
    expected: String,
    actual: String,
}

/// Evaluates `vector` against the real `mtc` type it names, returning the
/// structural-diff / mismatch report as the `Err` string on failure.
///
/// # Errors
///
/// A formatted, multi-line diagnostic describing exactly which expectation
/// (parse outcome, verify outcome, or which field) did not match. Never
/// panics — a vector whose own JSON is well-formed but whose `wire_hex` or
/// verification material is bad hex fails with a descriptive `Err`, the same
/// as any other conformance mismatch.
pub fn evaluate(vector: &Vector) -> Result<(), String> {
    match vector {
        Vector::Checkpoint(v) => evaluate_checkpoint(v),
        Vector::InclusionProof(v) => evaluate_inclusion_proof(v),
        Vector::LogEntry(v) => evaluate_log_entry(v),
    }
}

// --- checkpoint --------------------------------------------------------

fn evaluate_checkpoint(v: &CheckpointVector) -> Result<(), String> {
    let bytes = decode_field("wire_hex", &v.wire_hex)?;
    let parsed = Checkpoint::<Signed>::parse_tls_presentation(&bytes);
    match (v.parse.outcome, parsed) {
        (Outcome::Reject, Err(err)) => check_error_class(v.parse.error_class.as_deref(), &err),
        (Outcome::Reject, Ok(cp)) => Err(accepted_but_expected_reject(&format!("{cp:?}"))),
        (Outcome::Accept, Err(err)) => Err(rejected_but_expected_accept(&err)),
        (Outcome::Accept, Ok(cp)) => {
            let expected = require_fields(v.parse.fields.as_ref())?;
            let diffs = diff_checkpoint_fields(expected, &cp);
            require_no_diffs("parsed checkpoint fields did not match", &diffs)?;
            if let Some(verify) = &v.verify {
                evaluate_checkpoint_verify(verify, &cp)?;
            }
            Ok(())
        }
    }
}

fn diff_checkpoint_fields(
    expected: &CheckpointFields,
    actual: &Checkpoint<Signed>,
) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();
    diff_eq(
        &mut diffs,
        "log_id",
        &expected.log_id,
        actual.log_id().as_str(),
    );
    diff_eq(
        &mut diffs,
        "tree_size",
        &expected.tree_size.to_string(),
        &actual.tree_size().0.to_string(),
    );
    diff_hex(
        &mut diffs,
        "root_hash_hex",
        &expected.root_hash_hex,
        actual.root_hash().as_bytes(),
    );
    diff_eq(
        &mut diffs,
        "signed_at",
        &expected.signed_at.to_string(),
        &actual.signed_at().0.to_string(),
    );
    diff_hex(
        &mut diffs,
        "signature_hex",
        &expected.signature_hex,
        actual.signature().as_bytes(),
    );
    diffs
}

fn evaluate_checkpoint_verify(
    v: &VerifyExpectation<CheckpointVerifyMaterial>,
    cp: &Checkpoint<Signed>,
) -> Result<(), String> {
    let key_bytes = decode_field(
        "verify.verifying_key_spki_hex",
        &v.material.verifying_key_spki_hex,
    )?;
    let key = VerifyingKey::from_spki_der(SignatureAlgorithm::EcdsaP256Sha256, key_bytes);
    let scheme = EcdsaP256;
    let result = cp.verify(&scheme, &key);
    match (v.outcome, result) {
        (Outcome::Accept, Ok(())) => Ok(()),
        (Outcome::Accept, Err(err)) => Err(rejected_but_expected_accept(&err)),
        (Outcome::Reject, Err(err)) => check_error_class(v.error_class.as_deref(), &err),
        (Outcome::Reject, Ok(())) => {
            Err(accepted_but_expected_reject("(): verification succeeded"))
        }
    }
}

// --- inclusion proof -----------------------------------------------------

fn evaluate_inclusion_proof(v: &InclusionProofVector) -> Result<(), String> {
    let bytes = decode_field("wire_hex", &v.wire_hex)?;
    let parsed = InclusionProof::tls_parse_exact(&bytes);
    match (v.parse.outcome, parsed) {
        (Outcome::Reject, Err(err)) => check_error_class(v.parse.error_class.as_deref(), &err),
        (Outcome::Reject, Ok(proof)) => Err(accepted_but_expected_reject(&format!("{proof:?}"))),
        (Outcome::Accept, Err(err)) => Err(rejected_but_expected_accept(&err)),
        (Outcome::Accept, Ok(proof)) => {
            let expected = require_fields(v.parse.fields.as_ref())?;
            let diffs = diff_inclusion_proof_fields(expected, &proof);
            require_no_diffs("parsed inclusion proof fields did not match", &diffs)?;
            if let Some(verify) = &v.verify {
                evaluate_inclusion_proof_verify(verify, &proof)?;
            }
            Ok(())
        }
    }
}

fn diff_inclusion_proof_fields(
    expected: &InclusionProofFields,
    actual: &InclusionProof,
) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();
    diff_eq(
        &mut diffs,
        "tree_size",
        &expected.tree_size.to_string(),
        &actual.tree_size().0.to_string(),
    );
    diff_eq(
        &mut diffs,
        "leaf_index",
        &expected.leaf_index.to_string(),
        &actual.leaf_index().0.to_string(),
    );
    let actual_path: Vec<String> = actual
        .audit_path()
        .iter()
        .map(|h| hex::encode(h.as_bytes()))
        .collect();
    if expected.audit_path_hex.len() == actual_path.len() {
        for (i, (e, a)) in expected
            .audit_path_hex
            .iter()
            .zip(actual_path.iter())
            .enumerate()
        {
            if !e.eq_ignore_ascii_case(a) {
                diffs.push(FieldDiff {
                    field: format!("audit_path_hex[{i}]"),
                    expected: e.to_lowercase(),
                    actual: a.clone(),
                });
            }
        }
    } else {
        diffs.push(FieldDiff {
            field: "audit_path_hex (length)".to_string(),
            expected: expected.audit_path_hex.len().to_string(),
            actual: actual_path.len().to_string(),
        });
    }
    diffs
}

fn evaluate_inclusion_proof_verify(
    v: &VerifyExpectation<InclusionProofVerifyMaterial>,
    proof: &InclusionProof,
) -> Result<(), String> {
    let leaf_hash = decode_hash_field("verify.leaf_hash_hex", &v.material.leaf_hash_hex)?;
    let root_hash = decode_hash_field("verify.root_hash_hex", &v.material.root_hash_hex)?;
    let result = proof.verify::<Sha256Hasher>(&leaf_hash, &root_hash);
    match (v.outcome, result) {
        (Outcome::Accept, Ok(())) => Ok(()),
        (Outcome::Accept, Err(err)) => Err(rejected_but_expected_accept(&err)),
        (Outcome::Reject, Err(err)) => check_error_class(v.error_class.as_deref(), &err),
        (Outcome::Reject, Ok(())) => {
            Err(accepted_but_expected_reject("(): verification succeeded"))
        }
    }
}

// --- log entry -------------------------------------------------------------

fn evaluate_log_entry(v: &LogEntryVector) -> Result<(), String> {
    let bytes = decode_field("wire_hex", &v.wire_hex)?;
    let parsed = LogEntry::tls_parse_exact(&bytes);
    match (v.parse.outcome, parsed) {
        (Outcome::Reject, Err(err)) => check_error_class(v.parse.error_class.as_deref(), &err),
        (Outcome::Reject, Ok(entry)) => Err(accepted_but_expected_reject(&format!("{entry:?}"))),
        (Outcome::Accept, Err(err)) => Err(rejected_but_expected_accept(&err)),
        (Outcome::Accept, Ok(entry)) => {
            let expected = require_fields(v.parse.fields.as_ref())?;
            let diffs = diff_log_entry_fields(expected, &entry);
            require_no_diffs("parsed log entry fields did not match", &diffs)
        }
    }
}

fn diff_log_entry_fields(expected: &LogEntryFields, actual: &LogEntry) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();
    match (expected, actual) {
        (LogEntryFields::Null, LogEntry::Null) => {}
        (
            LogEntryFields::Certificate {
                subject_type,
                subject_info_hash_hex,
                claim_count,
            },
            LogEntry::Certificate(entry),
        ) => {
            diff_eq(
                &mut diffs,
                "subject_type",
                subject_type,
                subject_type_name(entry.subject_type()),
            );
            diff_hex(
                &mut diffs,
                "subject_info_hash_hex",
                subject_info_hash_hex,
                entry.subject_info_hash().as_hash().as_bytes(),
            );
            diff_eq(
                &mut diffs,
                "claim_count",
                &claim_count.to_string(),
                &entry.claims().len().to_string(),
            );
        }
        (expected, actual) => diffs.push(FieldDiff {
            field: "variant".to_string(),
            expected: format!("{expected:?}"),
            actual: format!("{actual:?}"),
        }),
    }
    diffs
}

/// Stable name for a [`SubjectType`], for [`LogEntryFields::Certificate`]
/// comparison. `SubjectType` is `#[non_exhaustive]` (spec §22.3), so this
/// crate — outside the defining crate — must handle unrecognized future
/// variants; they diff as `"unknown"` rather than failing to compile.
const fn subject_type_name(t: SubjectType) -> &'static str {
    match t {
        SubjectType::Tls => "tls",
        // `SubjectType` is `#[non_exhaustive]`; a match outside its defining
        // crate must handle a future variant. There is exactly one today, so
        // this arm is unreachable in practice but required to compile.
        _ => "unknown",
    }
}

// --- shared helpers --------------------------------------------------------

fn diff_eq(diffs: &mut Vec<FieldDiff>, field: &str, expected: &str, actual: &str) {
    if expected != actual {
        diffs.push(FieldDiff {
            field: field.to_string(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
}

fn diff_hex(diffs: &mut Vec<FieldDiff>, field: &str, expected_hex: &str, actual_bytes: &[u8]) {
    let actual_hex = hex::encode(actual_bytes);
    if !expected_hex.eq_ignore_ascii_case(&actual_hex) {
        diffs.push(FieldDiff {
            field: field.to_string(),
            expected: expected_hex.to_lowercase(),
            actual: actual_hex,
        });
    }
}

fn decode_field(name: &str, value: &str) -> Result<Vec<u8>, String> {
    hex::decode(value).map_err(|e| format!("{name} is not valid hex: {e}"))
}

fn decode_hash_field(name: &str, value: &str) -> Result<HashOutput, String> {
    let bytes = decode_field(name, value)?;
    HashOutput::try_from(bytes.as_slice()).map_err(|e| format!("{name}: {e}"))
}

fn require_fields<F>(fields: Option<&F>) -> Result<&F, String> {
    fields.ok_or_else(|| {
        "vector schema error: parse.outcome is \"accept\" but parse.fields is missing".to_string()
    })
}

fn require_no_diffs(context: &str, diffs: &[FieldDiff]) -> Result<(), String> {
    if diffs.is_empty() {
        Ok(())
    } else {
        Err(format_diffs(context, diffs))
    }
}

fn format_diffs(context: &str, diffs: &[FieldDiff]) -> String {
    let mut out = format!("{context} (structural diff):\n");
    for d in diffs {
        // Infallible for a `String` sink; `write!` avoids the extra
        // per-line `format!` allocation `push_str` would need.
        let _ = writeln!(
            out,
            "  - {}: expected {:?}, actual {:?}",
            d.field, d.expected, d.actual
        );
    }
    out
}

/// Checks that `expected_class` (a vector's `error_class`) is a substring of
/// `err`'s `Debug` rendering — see [`crate::schema::ParseExpectation`] docs
/// for why substring, not full equality.
fn check_error_class<E: Debug>(expected_class: Option<&str>, err: &E) -> Result<(), String> {
    let Some(class) = expected_class else {
        return Err(
            "vector schema error: outcome is \"reject\" but error_class is missing".to_string(),
        );
    };
    let debug = format!("{err:?}");
    if debug.contains(class) {
        Ok(())
    } else {
        Err(format!(
            "expected error class {class:?} not found in the actual error\n  actual error (Debug): {debug}"
        ))
    }
}

fn rejected_but_expected_accept<E: Debug>(err: &E) -> String {
    format!("expected ACCEPT but got a rejection\n  error (Debug): {err:?}")
}

fn accepted_but_expected_reject(actual_debug: &str) -> String {
    format!("expected REJECT but the operation succeeded\n  value (Debug): {actual_debug}")
}
