//! Live `SoftHSM2` integration test for the checkpoint signer (spec §14, §19.1),
//! gated behind `--features integration`.
//!
//! It signs a real checkpoint on a provisioned `SoftHSM2` token through the
//! [`CheckpointSigner`] and proves the framed §8.1 object parses via mtc's read
//! path and verifies under the token's exported public key. It mirrors
//! cloud-softhsm's own connect-or-skip harness: with no provisioned token the
//! test **skips** with a notice (set `MTC_SOFTHSM_REQUIRE=1` to make a skip a
//! hard failure, as CI's integration lane does).
//!
//! PKCS#11 `C_Initialize` is process-global per module, so run single-threaded:
//!
//! ```console
//! cargo test -p checkpoint-signer --features integration -- --test-threads=1
//! ```
#![cfg(feature = "integration")]

use std::path::Path;
use std::sync::Arc;

use checkpoint_signer::{CheckpointSigner, RetryPolicy};
use clock::tokio::AsyncClock;
use clock::FakeClock;
use cloud_softhsm::{Pkcs11Config, SoftHsm};
use cloud_types::{Hsm, KeySpec};
use mtc::{
    Checkpoint, EcdsaP256, HashOutput, LogId, SignatureAlgorithm, Signed, SignedAt, TreeSize,
    VerifyingKey,
};

/// Connects to the configured `SoftHSM2` token, or returns `None` (after
/// printing why) when the environment is not provisioned — unless
/// `MTC_SOFTHSM_REQUIRE=1`, in which case a missing environment panics.
// Non-`#[test]` helper in an integration-test file: `expect` on freshly-parsed
// values is the intended failure signal (clippy.toml scoped-allow pattern).
#[allow(clippy::expect_used)]
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
                &format!("could not connect to SoftHSM2 token: {error}"),
            );
            None
        }
    }
}

fn handle_unavailable(require: bool, reason: &str) {
    assert!(
        !require,
        "MTC_SOFTHSM_REQUIRE=1 but SoftHSM2 is unavailable: {reason}"
    );
    eprintln!(
        "[skip] checkpoint-signer SoftHSM2 integration: {reason} (run scripts/softhsm-init.sh)"
    );
}

#[tokio::test]
async fn signs_a_checkpoint_on_softhsm_and_the_object_verifies() {
    let Some(hsm) = connect_or_skip() else {
        return;
    };
    let hsm: Arc<dyn Hsm> = Arc::new(hsm);
    let handle = hsm
        .generate_key(KeySpec::EcdsaP256)
        .await
        .expect("generate key");
    let public_key = hsm
        .get_public_key(&handle)
        .await
        .expect("export public key");

    let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
    let signer = CheckpointSigner::new(Arc::clone(&hsm), clock, RetryPolicy::default());

    let object = signer
        .sign_checkpoint(
            &handle,
            LogId::new("ca").expect("log id"),
            TreeSize(2),
            HashOutput([0x11; 32]),
            SignedAt(1_700_000_000),
        )
        .await
        .expect("sign the checkpoint on SoftHSM2");

    assert_eq!(object.key(), "checkpoints/0000000000000002.signed");

    // Anti-drift + crypto: the framed object parses via mtc, and the SoftHSM
    // signature verifies under the exported SPKI public key.
    let parsed = Checkpoint::<Signed>::parse_tls_presentation(object.bytes())
        .expect("mtc parses the framed SoftHSM object");
    let verifying_key = VerifyingKey::from_spki_der(
        SignatureAlgorithm::EcdsaP256Sha256,
        public_key.spki_der().to_vec(),
    );
    parsed
        .verify(&EcdsaP256, &verifying_key)
        .expect("SoftHSM checkpoint signature verifies");
}
