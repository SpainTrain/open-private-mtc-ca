//! Runs the checked-in fuzz corpus through the parse path under plain
//! `cargo test`, so the "no panic on arbitrary bytes" property is exercised
//! on every PR even where nightly/cargo-fuzz is unavailable (spec §19.3:
//! corpus inputs double as regression fixtures).

use std::fs;
use std::path::PathBuf;

use acme_core::account::NewAccountRequest;
use acme_core::jws::{AccountKeySource, Jws};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz")
        .join("corpus")
        .join("fuzz_jws_parse")
}

/// Mirror of `fuzz/fuzz_targets/fuzz_jws_parse.rs`.
fn exercise(data: &[u8]) {
    let Ok(jws) = Jws::parse(data) else {
        return;
    };
    let _ = jws.check_alg();
    let _ = jws.nonce();
    let _ = jws.check_url("https://ca.example/acme/new-account");
    if let Ok(AccountKeySource::Jwk(jwk)) = jws.account_key() {
        let _ = jwk.thumbprint();
        if let Ok(key) = jwk.verifying_key() {
            let _ = jws.verify_signature(&key);
        }
    }
    let _ = serde_json::from_slice::<NewAccountRequest>(jws.payload());
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
