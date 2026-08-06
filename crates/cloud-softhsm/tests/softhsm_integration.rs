//! Live `SoftHSM2` integration tests for [`cloud_softhsm::SoftHsm`] (spec §14,
//! §19.1) — the same sign/verify behavior the shared `cloud-test-suite` Hsm
//! suite asserts, plus the concurrency and latency bars from the ticket.
//!
//! Gated behind `--features integration`. They require a provisioned token
//! (`scripts/softhsm-init.sh`) and a host `SoftHSM2` install whose module the
//! `MTC_PKCS11_*` env contract points at. If the module or token is not
//! present the tests **skip** with a printed notice rather than fail (so a
//! developer without `SoftHSM2` still gets a green `--features integration` run);
//! set `MTC_SOFTHSM_REQUIRE=1` to turn a skip into a hard failure (CI uses
//! this in the cloud-softhsm-conformance lane).
//!
//! PKCS#11 `C_Initialize` is process-global per module, so run single-threaded:
//!
//! ```console
//! cargo test -p cloud-softhsm --features integration -- --test-threads=1
//! ```
#![cfg(feature = "integration")]

use std::path::Path;
use std::sync::Arc;

use cloud_softhsm::{Pkcs11Config, SoftHsm};
use cloud_types::{Hsm, KeyHandle, KeySpec};
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey as _;

/// Connects to the configured `SoftHSM2` token, or returns `None` (after
/// printing why) when the environment is not provisioned — unless
/// `MTC_SOFTHSM_REQUIRE=1`, in which case a missing environment panics.
fn connect_or_skip() -> Option<SoftHsm> {
    let require = std::env::var("MTC_SOFTHSM_REQUIRE").is_ok_and(|v| v == "1");

    let config = match Pkcs11Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            handle_unavailable(require, &format!("invalid PKCS#11 config: {error}"));
            return None;
        }
    };

    if !Path::new(config.module_path()).exists() {
        handle_unavailable(
            require,
            &format!("PKCS#11 module {} not found", config.module_path()),
        );
        return None;
    }

    match SoftHsm::connect(config) {
        Ok(hsm) => Some(hsm),
        Err(error) => {
            handle_unavailable(
                require,
                &format!("could not connect to `SoftHSM2` token: {error}"),
            );
            None
        }
    }
}

fn handle_unavailable(require: bool, reason: &str) {
    assert!(
        !require,
        "MTC_SOFTHSM_REQUIRE=1 but `SoftHSM2` is unavailable: {reason}"
    );
    eprintln!("[skip] `SoftHSM2` integration test: {reason} (run scripts/softhsm-init.sh)");
}

/// Verifies `signature` (P1363 r‖s) over `message` under the SPKI DER public
/// key — the exact check the shared conformance suite performs with `RustCrypto`.
// Non-`#[test]` helper: `expect` on freshly-parsed test values is the intended
// failure signal (docs/lint-policy.md — scoped allow for integration helpers).
#[allow(clippy::expect_used)]
fn assert_verifies(spki_der: &[u8], message: &[u8], signature: &[u8]) {
    assert_eq!(signature.len(), 64, "P-256 P1363 signature is 64 bytes");
    let verifying_key =
        VerifyingKey::from_public_key_der(spki_der).expect("exported SPKI DER parses");
    let parsed = Signature::from_slice(signature).expect("64-byte r||s parses");
    verifying_key
        .verify(message, &parsed)
        .expect("SoftHSM signature verifies under the exported public key");
}

#[tokio::test]
async fn generate_sign_verify_round_trip() {
    let Some(hsm) = connect_or_skip() else {
        return;
    };

    let handle = hsm
        .generate_key(KeySpec::EcdsaP256)
        .await
        .expect("generate_key");
    let message = b"mtc checkpoint bytes";
    let signature = hsm.sign(&handle, message).await.expect("sign");
    let public_key = hsm.get_public_key(&handle).await.expect("get_public_key");

    assert_verifies(public_key.spki_der(), message, &signature);
    assert!(
        !hsm.is_fips_validated(),
        "`SoftHSM2` is never FIPS (spec §14.4)"
    );
}

#[tokio::test]
async fn sign_with_unknown_handle_is_not_found_never_panics() {
    let Some(hsm) = connect_or_skip() else {
        return;
    };
    let err = hsm
        .sign(&KeyHandle::new("cloud-softhsm-no-such-key"), b"data")
        .await
        .expect_err("unknown handle must error, not panic");
    assert!(matches!(err, cloud_types::CloudError::NotFound { .. }));
}

#[tokio::test]
async fn get_public_key_unknown_handle_is_not_found() {
    let Some(hsm) = connect_or_skip() else {
        return;
    };
    assert!(matches!(
        hsm.get_public_key(&KeyHandle::new("cloud-softhsm-no-such-key"))
            .await,
        Err(cloud_types::CloudError::NotFound { .. })
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_signing_eight_tasks_all_verify() {
    let Some(hsm) = connect_or_skip() else {
        return;
    };
    let hsm = Arc::new(hsm);
    let handle = hsm
        .generate_key(KeySpec::EcdsaP256)
        .await
        .expect("generate_key");
    let public_key = hsm.get_public_key(&handle).await.expect("get_public_key");
    let spki_der = public_key.spki_der().to_vec();

    // Eight tasks share one HSM (one PKCS#11 context) and sign in parallel;
    // each opens its own session (sessions are never shared across threads).
    let mut tasks = Vec::new();
    for i in 0u8..8 {
        let hsm = Arc::clone(&hsm);
        let handle = handle.clone();
        tasks.push(tokio::spawn(async move {
            let message = [b"parallel-sign-", &[i][..]].concat();
            let signature = hsm.sign(&handle, &message).await.expect("concurrent sign");
            (message, signature)
        }));
    }

    for task in tasks {
        let (message, signature) = task.await.expect("signing task joins");
        assert_verifies(&spki_der, &message, &signature);
    }
}

#[tokio::test]
async fn single_sign_latency_is_well_under_the_100ms_p99_target() {
    let Some(hsm) = connect_or_skip() else {
        return;
    };
    let handle = hsm
        .generate_key(KeySpec::EcdsaP256)
        .await
        .expect("generate_key");
    // Warm up (module/session first-touch), then time one signature.
    hsm.sign(&handle, b"warmup").await.expect("warmup sign");

    // Scoped allow: a latency micro-measurement genuinely needs monotonic
    // ambient time; the injected `Clock` (rule no-systemtime-now-in-prod) is
    // for production time-dependent logic, not a test stopwatch. This is the
    // sanctioned test exemption pattern (docs/lint-policy.md).
    #[allow(clippy::disallowed_methods)]
    let start = std::time::Instant::now();
    hsm.sign(&handle, b"timed").await.expect("timed sign");
    #[allow(clippy::disallowed_methods)]
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 100,
        "single SoftHSM sign took {elapsed:?}, expected far under the 100ms p99 target (spec §14.3)"
    );
}
