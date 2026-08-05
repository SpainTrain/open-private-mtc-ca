//! Regenerates the clean-room seed vectors under `conformance/vectors/` from
//! the real `mtc` crate serializers.
//!
//! ```sh
//! cargo run -p mtc-conformance --example generate_vectors
//! ```
//!
//! Ticket `test-conformance-runner` requires vectors "generate[d] ... from
//! the real Checkpoint/InclusionProof/LogEntry serializers, then lock[ed] as
//! fixtures" rather than hand-invented byte layouts. This program is that
//! generation step, kept as a runnable, auditable tool rather than a
//! one-off script: extend it (add a `write_*_vectors` call or a new case in
//! an existing one) when adding another clean-room vector — see
//! `conformance/vectors/README.md` "Adding a vector".
//!
//! # What is, and is not, generator output
//!
//! Every accept vector's `wire_hex` is the direct output of a real
//! `sign`/`generate`/`tls_serialize_to_vec` call below. Most reject vectors
//! are derived from that real output by a small, explicit, in-code mutation
//! (append a stray byte, drop the last byte, flip a bit) — so the "shape" is
//! still real serializer output, only deliberately corrupted, and the
//! corruption is itself code, not hand-transcribed hex. Two checkpoint reject
//! vectors are the exception: `checkpoint-reject-empty-trust-anchor-id` and
//! `checkpoint-reject-non-utf8-log-id` are too short to come from any real
//! `Checkpoint` (a valid `TrustAnchorID` alone is at least 2 bytes). Their
//! bytes are hand-written here, but they are not invented: they are the exact
//! byte sequences `crates/mtc/src/checkpoint/mod.rs`'s own
//! `parse_rejects_empty_trust_anchor_id` / `parse_rejects_non_utf8_log_id`
//! unit tests already assert are correct — reused, not guessed.
//!
//! # Reproducibility
//!
//! Signing uses the fixed ECDSA P-256 key from RFC 6979 Appendix A.2.5 (the
//! same published test key `crates/mtc/src/signing/ecdsa_p256.rs`'s own
//! known-answer tests use) rather than `EcdsaP256::generate_keypair`'s `OsRng`,
//! and ECDSA P-256 signing is RFC 6979 deterministic — so re-running this
//! program reproduces byte-identical vectors.
//!
//! # Error handling
//!
//! This is a bin-shaped dev tool (rule `thiserror-for-libs-eyre-for-bins`):
//! every fallible step propagates via `?` into `eyre::Result`, never
//! `.expect()`/`.unwrap()` (`clippy::expect_used`/`unwrap_used` are denied
//! outside `#[test]`/`#[cfg(test)]`, and `examples/` is not exempt).

use std::fs;
use std::path::{Path, PathBuf};

use eyre::Context;
use mtc::{
    hash_leaf, Checkpoint, CheckpointBuilder, Claim, DnsName, EcdsaP256, HashOutput,
    InclusionProof, Index, LogEntry, LogId, MerkleTree, Sha256Hasher, Signed, SignedAt, SigningKey,
    SubjectInfoHash, SubjectType, TbsCertificateLogEntry, TlsParse, TlsSerialize, TreeSize,
    VerifyingKey,
};
use mtc_conformance::hex;
use mtc_conformance::schema::{
    CheckpointFields, CheckpointVector, CheckpointVerifyMaterial, InclusionProofFields,
    InclusionProofVector, InclusionProofVerifyMaterial, LogEntryFields, LogEntryVector, Outcome,
    ParseExpectation, Vector, VerifyExpectation,
};

/// RFC 6979 Appendix A.2.5 P-256 private key (big-endian scalar) — published
/// standards text, the same constant `ecdsa_p256.rs`'s own KAT test uses.
const SIGNING_KEY_HEX: &str = "C9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721";

/// The SEC1-encoded public point matching [`SIGNING_KEY_HEX`] (RFC 6979
/// Appendix A.2.5), uncompressed form `0x04 || Ux || Uy`.
const VERIFYING_KEY_SEC1_HEX: &str = concat!(
    "04",
    "60FED4BA255A9D31C961EB74C6356D68C049B8923B61FA6CE669622E60F29FB6",
    "7903FE1008B8BC99A41AE9E95628BC64F2F1B20C2D7E9F5177A3C294D4462299",
);

fn main() -> eyre::Result<()> {
    let root = vectors_root();
    write_checkpoint_vectors(&root.join("checkpoint"))?;
    write_inclusion_proof_vectors(&root.join("inclusion-proof"))?;
    write_log_entry_vectors(&root.join("log-entry"))?;
    println!("wrote clean-room seed vectors under {}", root.display());
    Ok(())
}

/// `conformance/vectors/`, resolved relative to this crate's manifest so the
/// tool works from any invoking directory.
fn vectors_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/vectors")
}

/// The fixed KAT keypair used to sign every checkpoint vector.
fn signing_keypair() -> eyre::Result<(SigningKey, VerifyingKey)> {
    let secret = hex::decode(SIGNING_KEY_HEX)?;
    let signing = EcdsaP256::signing_key_from_bytes(&secret)?;
    let public = hex::decode(VERIFYING_KEY_SEC1_HEX)?;
    let verifying = EcdsaP256::verifying_key_from_sec1(&public)?;
    Ok((signing, verifying))
}

/// Serializes `vector` as pretty JSON to `dir/file_name`, creating `dir` if
/// needed.
fn write_vector(dir: &Path, file_name: &str, vector: &Vector) -> eyre::Result<()> {
    fs::create_dir_all(dir).wrap_err_with(|| format!("failed to create {}", dir.display()))?;
    let json = serde_json::to_string_pretty(vector).wrap_err("vector failed to serialize")?;
    let path = dir.join(file_name);
    fs::write(&path, format!("{json}\n"))
        .wrap_err_with(|| format!("failed to write {}", path.display()))?;
    println!("  wrote {}", path.display());
    Ok(())
}

// --- checkpoint --------------------------------------------------------

/// The one real, signed [`Checkpoint`] every checkpoint vector below is
/// built from — either directly (the accept vector) or by mutating
/// [`Self::wire`] (the reject vectors that need real bytes to corrupt).
struct CheckpointFixture {
    wire: Vec<u8>,
    fields: CheckpointFields,
    verifying_key_spki_hex: String,
}

fn build_checkpoint_fixture() -> eyre::Result<CheckpointFixture> {
    let (signing, verifying) = signing_keypair()?;
    let scheme = EcdsaP256;

    let log_id = LogId::new("prod-log-1")?;
    let root_hash = HashOutput([0x7f; 32]);
    let tree_size = TreeSize(12345);
    let signed_at = SignedAt(1_700_000_000);

    let signed: Checkpoint<Signed> = CheckpointBuilder::new(log_id.clone())
        .root_hash(root_hash)
        .tree_size(tree_size)
        .signed_at(signed_at)
        .build()
        .sign(&scheme, &signing)?;
    let wire = signed.serialize_tls_presentation()?;

    Ok(CheckpointFixture {
        wire,
        fields: CheckpointFields {
            log_id: log_id.into_string(),
            tree_size: tree_size.0,
            root_hash_hex: hex::encode(root_hash.as_bytes()),
            signed_at: signed_at.0,
            signature_hex: hex::encode(signed.signature().as_bytes()),
        },
        verifying_key_spki_hex: hex::encode(verifying.spki_der()),
    })
}

fn checkpoint_accept_vector(fixture: &CheckpointFixture) -> Vector {
    Vector::Checkpoint(CheckpointVector {
        id: "checkpoint-accept-001".to_string(),
        description: "Happy path (spec §19.4 AC). A signed Checkpoint built via \
            CheckpointBuilder, signed with ECDSA P-256 under the RFC 6979 Appendix \
            A.2.5 test key, and serialized with \
            Checkpoint::serialize_tls_presentation. Exercises parse accept and \
            verify accept."
            .to_string(),
        wire_hex: hex::encode(&fixture.wire),
        parse: ParseExpectation {
            outcome: Outcome::Accept,
            fields: Some(fixture.fields.clone()),
            error_class: None,
        },
        verify: Some(VerifyExpectation {
            outcome: Outcome::Accept,
            material: CheckpointVerifyMaterial {
                verifying_key_spki_hex: fixture.verifying_key_spki_hex.clone(),
            },
            error_class: None,
        }),
    })
}

fn checkpoint_reject_empty_trust_anchor_id_vector() -> Vector {
    Vector::Checkpoint(CheckpointVector {
        id: "checkpoint-reject-empty-trust-anchor-id".to_string(),
        description: "Must-reject. A single 0x00 length-prefix byte for the \
            TrustAnchorID (log id) field: below the draft's opaque<1..2^8-1> \
            minimum of 1 byte. Not derived from a serializer call (no valid \
            Checkpoint is this short) — this is the exact byte sequence \
            crates/mtc/src/checkpoint/mod.rs's own \
            parse_rejects_empty_trust_anchor_id unit test already asserts is \
            correct, reused rather than invented."
            .to_string(),
        wire_hex: hex::encode(&[0x00u8]),
        parse: ParseExpectation {
            outcome: Outcome::Reject,
            fields: None,
            error_class: Some("TrustAnchorIdEmpty".to_string()),
        },
        verify: None,
    })
}

fn checkpoint_reject_non_utf8_log_id_vector() -> Vector {
    Vector::Checkpoint(CheckpointVector {
        id: "checkpoint-reject-non-utf8-log-id".to_string(),
        description: "Must-reject. TrustAnchorID length prefix 0x01 followed by \
            the single byte 0xFF, which is not valid UTF-8. Reuses the exact byte \
            sequence crates/mtc/src/checkpoint/mod.rs's own \
            parse_rejects_non_utf8_log_id unit test asserts is correct."
            .to_string(),
        wire_hex: hex::encode(&[0x01u8, 0xFF]),
        parse: ParseExpectation {
            outcome: Outcome::Reject,
            fields: None,
            error_class: Some("LogIdNotUtf8".to_string()),
        },
        verify: None,
    })
}

fn checkpoint_reject_trailing_bytes_vector(fixture: &CheckpointFixture) -> Vector {
    let mut trailing = fixture.wire.clone();
    trailing.push(0x99);
    Vector::Checkpoint(CheckpointVector {
        id: "checkpoint-reject-trailing-bytes".to_string(),
        description: "Must-reject. checkpoint-accept-001's real serialized bytes \
            with one stray byte (0x99) appended — TlsReader::finish rejects any \
            unconsumed suffix after a structurally complete checkpoint."
            .to_string(),
        wire_hex: hex::encode(&trailing),
        parse: ParseExpectation {
            outcome: Outcome::Reject,
            fields: None,
            error_class: Some("TrailingBytes".to_string()),
        },
        verify: None,
    })
}

fn write_checkpoint_vectors(dir: &Path) -> eyre::Result<()> {
    let fixture = build_checkpoint_fixture()?;
    write_vector(
        dir,
        "checkpoint-accept-001.json",
        &checkpoint_accept_vector(&fixture),
    )?;
    write_vector(
        dir,
        "checkpoint-reject-empty-trust-anchor-id.json",
        &checkpoint_reject_empty_trust_anchor_id_vector(),
    )?;
    write_vector(
        dir,
        "checkpoint-reject-non-utf8-log-id.json",
        &checkpoint_reject_non_utf8_log_id_vector(),
    )?;
    write_vector(
        dir,
        "checkpoint-reject-trailing-bytes.json",
        &checkpoint_reject_trailing_bytes_vector(&fixture),
    )?;
    Ok(())
}

// --- inclusion proof -------------------------------------------------------

/// The one real, generated [`InclusionProof`] every inclusion-proof vector
/// below is built from.
struct InclusionProofFixture {
    wire: Vec<u8>,
    fields: InclusionProofFields,
    leaf_hash_hex: String,
    root_hash_hex: String,
}

/// Builds the fields expectation for an [`InclusionProof`] — shared by the
/// fixture's own happy-path fields and by the root-mismatch vector (whose
/// bytes are tampered but still parse to a structurally well-formed proof).
fn inclusion_proof_fields(proof: &InclusionProof) -> InclusionProofFields {
    InclusionProofFields {
        tree_size: proof.tree_size().0,
        leaf_index: proof.leaf_index().0,
        audit_path_hex: proof
            .audit_path()
            .iter()
            .map(|h| hex::encode(h.as_bytes()))
            .collect(),
    }
}

fn build_inclusion_proof_fixture() -> eyre::Result<InclusionProofFixture> {
    // The same "entry-{i}" leaf convention crates/mtc's own inclusion-proof
    // tests use (crates/mtc/src/proof/inclusion.rs `tree_of`).
    let mut tree = MerkleTree::<Sha256Hasher>::new();
    for i in 0..13u64 {
        tree.append(format!("entry-{i}").as_bytes());
    }
    let proof = InclusionProof::generate(&tree, Index(5))?;
    let leaf_hash = hash_leaf::<Sha256Hasher>(b"entry-5");
    let root = tree.root();
    let wire = proof.tls_serialize_to_vec()?;
    let fields = inclusion_proof_fields(&proof);
    Ok(InclusionProofFixture {
        wire,
        fields,
        leaf_hash_hex: hex::encode(leaf_hash.as_bytes()),
        root_hash_hex: hex::encode(root.as_bytes()),
    })
}

fn inclusion_proof_accept_vector(fixture: &InclusionProofFixture) -> Vector {
    Vector::InclusionProof(InclusionProofVector {
        id: "inclusion-proof-accept-001".to_string(),
        description: "Happy path (spec §19.4 AC). InclusionProof::generate for \
            leaf index 5 of a 13-leaf tree (leaves \"entry-0\"..\"entry-12\", the \
            same convention crates/mtc's own inclusion-proof tests use), \
            serialized with tls_serialize_to_vec. Exercises parse accept and \
            verify accept; leaf_hash_hex is hash_leaf::<Sha256Hasher>(b\"entry-5\")."
            .to_string(),
        wire_hex: hex::encode(&fixture.wire),
        parse: ParseExpectation {
            outcome: Outcome::Accept,
            fields: Some(fixture.fields.clone()),
            error_class: None,
        },
        verify: Some(VerifyExpectation {
            outcome: Outcome::Accept,
            material: InclusionProofVerifyMaterial {
                leaf_hash_hex: fixture.leaf_hash_hex.clone(),
                root_hash_hex: fixture.root_hash_hex.clone(),
            },
            error_class: None,
        }),
    })
}

fn inclusion_proof_reject_truncated_vector(fixture: &InclusionProofFixture) -> Vector {
    let truncated = &fixture.wire[..fixture.wire.len() - 1];
    Vector::InclusionProof(InclusionProofVector {
        id: "inclusion-proof-reject-truncated".to_string(),
        description: "Must-reject. inclusion-proof-accept-001's real serialized \
            bytes with the final byte dropped: the audit_path vector's own u16 \
            length prefix (still the original claimed body length) now exceeds \
            the bytes actually remaining in the truncated input, so the bounded \
            reader rejects it as LengthOverflow before reading any element \
            (caught at the outer length-prefix check, ahead of the per-element \
            32-byte reads)."
            .to_string(),
        wire_hex: hex::encode(truncated),
        parse: ParseExpectation {
            outcome: Outcome::Reject,
            fields: None,
            error_class: Some("LengthOverflow".to_string()),
        },
        verify: None,
    })
}

fn inclusion_proof_reject_root_mismatch_vector(
    fixture: &InclusionProofFixture,
) -> eyre::Result<Vector> {
    let mut tampered = fixture.wire.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    // A single flipped bit inside a 32-byte hash keeps every length prefix
    // intact, so this still parses — only `verify()` catches it.
    let tampered_proof = InclusionProof::tls_parse_exact(&tampered)
        .map_err(|e| eyre::eyre!("tampered inclusion-proof bytes failed to parse: {e}"))?;

    Ok(Vector::InclusionProof(InclusionProofVector {
        id: "inclusion-proof-reject-root-mismatch".to_string(),
        description: "Must-reject (verify, not parse). \
            inclusion-proof-accept-001's real serialized bytes with the last bit \
            of the final audit-path hash flipped: still a structurally \
            well-formed proof (parse accepts), but verify() reconstructs a \
            different root than the true one, so it rejects with RootMismatch."
            .to_string(),
        wire_hex: hex::encode(&tampered),
        parse: ParseExpectation {
            outcome: Outcome::Accept,
            fields: Some(inclusion_proof_fields(&tampered_proof)),
            error_class: None,
        },
        verify: Some(VerifyExpectation {
            outcome: Outcome::Reject,
            material: InclusionProofVerifyMaterial {
                leaf_hash_hex: fixture.leaf_hash_hex.clone(),
                root_hash_hex: fixture.root_hash_hex.clone(),
            },
            error_class: Some("RootMismatch".to_string()),
        }),
    }))
}

fn write_inclusion_proof_vectors(dir: &Path) -> eyre::Result<()> {
    let fixture = build_inclusion_proof_fixture()?;
    write_vector(
        dir,
        "inclusion-proof-accept-001.json",
        &inclusion_proof_accept_vector(&fixture),
    )?;
    write_vector(
        dir,
        "inclusion-proof-reject-truncated.json",
        &inclusion_proof_reject_truncated_vector(&fixture),
    )?;
    write_vector(
        dir,
        "inclusion-proof-reject-root-mismatch.json",
        &inclusion_proof_reject_root_mismatch_vector(&fixture)?,
    )?;
    Ok(())
}

// --- log entry ---------------------------------------------------------

fn log_entry_accept_null_vector() -> eyre::Result<Vector> {
    let wire = LogEntry::null().tls_serialize_to_vec()?;
    Ok(Vector::LogEntry(LogEntryVector {
        id: "log-entry-accept-null".to_string(),
        description: "Happy path. The spec-defined null_entry placeholder \
            (LogEntry::null), serialized with tls_serialize_to_vec — the constant \
            two-byte `00 00` form (type = null_entry(0), Empty body)."
            .to_string(),
        wire_hex: hex::encode(&wire),
        parse: ParseExpectation {
            outcome: Outcome::Accept,
            fields: Some(LogEntryFields::Null),
            error_class: None,
        },
    }))
}

fn log_entry_accept_certificate_vector() -> eyre::Result<Vector> {
    let claim = Claim::dns(vec![DnsName::new(b"example.com".to_vec())?])?;
    let cert_entry = TbsCertificateLogEntry::builder()
        .subject_type(SubjectType::Tls)
        .subject_info_hash(SubjectInfoHash::from_hash(HashOutput([0x11; 32])))
        .claim(claim)
        .build();
    let wire = LogEntry::certificate(cert_entry.clone()).tls_serialize_to_vec()?;

    Ok(Vector::LogEntry(LogEntryVector {
        id: "log-entry-accept-certificate".to_string(),
        description: "Happy path. A TbsCertificateLogEntry (subject_type = tls, \
            a fixed subject_info_hash, one dns claim for \"example.com\") wrapped \
            as LogEntry::Certificate and serialized with tls_serialize_to_vec."
            .to_string(),
        wire_hex: hex::encode(&wire),
        parse: ParseExpectation {
            outcome: Outcome::Accept,
            fields: Some(LogEntryFields::Certificate {
                subject_type: "tls".to_string(),
                subject_info_hash_hex: hex::encode(
                    cert_entry.subject_info_hash().as_hash().as_bytes(),
                ),
                claim_count: cert_entry.claims().len(),
            }),
            error_class: None,
        },
    }))
}

fn log_entry_reject_unknown_type_vector() -> eyre::Result<Vector> {
    // MerkleTreeCertEntryType discriminant 2: neither null_entry(0) nor
    // tbs_cert_entry(1). `2u16.tls_serialize_to_vec()` is the real primitive
    // wire writer LogEntry's own codec builds on, applied to a value its
    // parser's match statement does not recognize.
    let wire = 2u16.tls_serialize_to_vec()?;
    Ok(Vector::LogEntry(LogEntryVector {
        id: "log-entry-reject-unknown-type".to_string(),
        description: "Must-reject. A MerkleTreeCertEntryType discriminant of 2, \
            which is neither null_entry(0) nor tbs_cert_entry(1) — \
            LogEntry::tls_parse's catch-all match arm rejects it as InvalidValue. \
            The two bytes are the real u16 TLS-presentation encoding of 2."
            .to_string(),
        wire_hex: hex::encode(&wire),
        parse: ParseExpectation {
            outcome: Outcome::Reject,
            fields: None,
            error_class: Some("InvalidValue".to_string()),
        },
    }))
}

fn write_log_entry_vectors(dir: &Path) -> eyre::Result<()> {
    write_vector(
        dir,
        "log-entry-accept-null.json",
        &log_entry_accept_null_vector()?,
    )?;
    write_vector(
        dir,
        "log-entry-accept-certificate.json",
        &log_entry_accept_certificate_vector()?,
    )?;
    write_vector(
        dir,
        "log-entry-reject-unknown-type.json",
        &log_entry_reject_unknown_type_vector()?,
    )?;
    Ok(())
}
