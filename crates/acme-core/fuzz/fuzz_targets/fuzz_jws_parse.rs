//! Fuzz target: JWS/request-body parsing must be total (spec §19.3).
//!
//! Feeds arbitrary bytes through the exact code path `new-account` uses on
//! untrusted input: JWS envelope parse, protected-header checks, key
//! decoding, thumbprinting, and payload deserialization. Asserts nothing
//! panics; errors are the expected outcome.

#![no_main]

use acme_core::account::NewAccountRequest;
use acme_core::jws::{AccountKeySource, Jws};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(jws) = Jws::parse(data) else {
        return;
    };
    // Post-parse checks on attacker-controlled structure.
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
});
