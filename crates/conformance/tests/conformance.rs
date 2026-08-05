//! The conformance suite itself (spec §19.4): discovers every vector under
//! `conformance/vectors/` and asserts it against the real `mtc` types.
//!
//! This is what `cargo test -p mtc-conformance` — and by extension `cargo
//! test --workspace --all-features`, the repository's `test` required CI
//! check (spec §22.13) — runs on every PR. `make test-conformance` (see
//! `mk/test.mk`) runs just this test with `--nocapture` so the per-vector
//! pass/fail lines and the total-count summary always print, matching the
//! ticket's demo command.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use mtc_conformance::schema::Outcome;
use mtc_conformance::{discover_vector_files, evaluate, load_vector_file, Vector};

/// `conformance/vectors/`, resolved relative to this crate's manifest
/// directory (`crates/conformance/`) so the test passes regardless of the
/// invoking working directory (repo root, this crate's directory, or CI).
fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/vectors")
}

/// Loads every vector under [`vectors_dir`]. Shared by both tests below so
/// discovery/parsing failures point at one place.
fn load_all_vectors() -> Vec<Vector> {
    let dir = vectors_dir();
    let files = discover_vector_files(&dir)
        .unwrap_or_else(|e| panic!("failed to walk vector directory {}: {e}", dir.display()));
    assert!(
        !files.is_empty(),
        "no vector files found under {} — the conformance suite has nothing to gate on",
        dir.display(),
    );
    files
        .iter()
        .map(|path| {
            load_vector_file(path).unwrap_or_else(|e| {
                panic!(
                    "vector file {} does not match the schema: {e}",
                    path.display()
                )
            })
        })
        .collect()
}

/// The conformance layer itself: every vector's `wire_hex` is parsed (and,
/// where supplied, verified) through the real `mtc` types and checked against
/// its `expect`ation. A mismatch prints the vector's `id` and a structural
/// diff (spec §19.4 AC), then fails the test after every vector has run (so
/// one bad vector does not hide a second, unrelated failure).
#[test]
fn conformance_suite() {
    let vectors = load_all_vectors();

    let mut failures: Vec<(String, String)> = Vec::new();
    let mut passed = 0usize;

    for vector in &vectors {
        let id = vector.id().to_string();
        let kind = vector.kind_name();
        match evaluate(vector) {
            Ok(()) => {
                println!("PASS [{kind}] {id}");
                passed += 1;
            }
            Err(detail) => {
                println!("FAIL [{kind}] {id}");
                failures.push((format!("[{kind}] {id}"), detail));
            }
        }
    }

    println!(
        "\nconformance suite: {passed} passed, {} failed, {} total",
        failures.len(),
        vectors.len(),
    );

    if !failures.is_empty() {
        let mut report = String::new();
        for (id, detail) in &failures {
            let _ = write!(report, "\n--- {id} ---\n{detail}\n");
        }
        panic!("{} conformance vector(s) failed:{report}", failures.len());
    }
}

/// Guards the ticket's seed-data acceptance criterion directly: a checkpoint
/// happy path, an inclusion-proof happy path, and at least three must-reject
/// vectors must always be present, so a future edit cannot silently thin the
/// seed set below what spec §19.4's AC requires.
#[test]
fn seed_vectors_satisfy_the_ac_minimums() {
    let vectors = load_all_vectors();

    let checkpoint_happy_path = vectors
        .iter()
        .any(|v| matches!(v, Vector::Checkpoint(_)) && v.parse_outcome() == Outcome::Accept);
    let inclusion_proof_happy_path = vectors
        .iter()
        .any(|v| matches!(v, Vector::InclusionProof(_)) && v.parse_outcome() == Outcome::Accept);
    let reject_count = vectors
        .iter()
        .filter(|v| v.parse_outcome() == Outcome::Reject)
        .count();

    assert!(
        checkpoint_happy_path,
        "spec §19.4 AC requires a checkpoint happy-path vector"
    );
    assert!(
        inclusion_proof_happy_path,
        "spec §19.4 AC requires an inclusion-proof happy-path vector"
    );
    assert!(
        reject_count >= 3,
        "spec §19.4 AC requires >= 3 must-reject vectors, found {reject_count}"
    );
}
