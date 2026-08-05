//! Runs the checked-in `parse_pruning_checkpoint` fuzz corpus through the
//! parse path under plain `cargo test`, so the "no panic on arbitrary bytes"
//! property (spec §19.3) is exercised on every PR even where nightly/
//! cargo-fuzz is unavailable (corpus inputs double as regression fixtures).
//!
//! Mirrors `crates/acme-core/tests/fuzz_corpus.rs` and
//! `crates/mtc/fuzz/fuzz_targets/parse_pruning_checkpoint.rs`.

use std::fs;
use std::path::PathBuf;

use mtc::{PruningCheckpoint, TlsParse};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz")
        .join("corpus")
        .join("parse_pruning_checkpoint")
}

/// Mirror of `fuzz/fuzz_targets/parse_pruning_checkpoint.rs`.
fn exercise(data: &[u8]) {
    let _ = PruningCheckpoint::tls_parse_exact(data);
}

#[test]
fn corpus_inputs_never_panic() {
    let dir = corpus_dir();
    let entries: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("corpus dir {}: {e}", dir.display()))
        .collect::<Result<_, _>>()
        .expect("readable corpus entries");
    assert!(!entries.is_empty(), "fuzz corpus must be checked in");
    for entry in entries {
        let bytes = fs::read(entry.path())
            .unwrap_or_else(|e| panic!("read {}: {e}", entry.path().display()));
        exercise(&bytes);
    }
}
