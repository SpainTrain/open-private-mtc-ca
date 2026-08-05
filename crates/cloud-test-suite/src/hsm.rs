//! Shared [`Hsm`] conformance suite (spec §9.7).
//!
//! Ported from `cloud-memory`'s original inline `#[cfg(test)]` module (spec
//! §9.6) -- the assertions are unchanged, generalized behind the
//! factory-closure pattern so any backend can run them. The compile-time
//! "key material zeroizes on drop" assertion stays in `cloud-memory`'s own
//! tests: it inspects `MemoryHsm`'s concrete signing-key type, an
//! implementation detail no [`Hsm`] method exposes, so it cannot be
//! generalized behind the trait.

use std::future::Future;

use cloud_types::{CloudError, Hsm, KeyHandle, KeySpec, PublicKey};
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

/// Deterministic message set every [`Hsm`] backend's sign/verify round trip
/// must handle: empty input, a short ASCII message, and a buffer spanning
/// more than one SHA-256 block, so backends cannot special-case short
/// buffers only.
const DETERMINISTIC_MESSAGES: &[&[u8]] = &[
    b"",
    b"checkpoint bytes",
    b"a message long enough to span more than one SHA-256 block of \
      input so a backend cannot special-case short buffers only, which is \
      exactly the kind of off-by-one a fixed-size test vector would miss",
];

/// Number of additional randomized messages exercised by the property case
/// (spec §19.2's "small randomized message property case").
const RANDOM_MESSAGE_CASES: u32 = 8;

/// Runs the full [`Hsm`] conformance suite against instances built by
/// `factory`.
///
/// `expected_fips` is the backend's documented FIPS posture
/// ([`Hsm::is_fips_validated`]) -- `false` for memory/`SoftHSM2`, `true` only
/// for hardware-backed `CloudHSM` runs (spec §14.4).
///
/// # Panics
///
/// Panics (via `assert!`/`assert_eq!`) on the first behavior that diverges
/// from the contract documented on [`Hsm`].
pub async fn run_hsm_suite<F, Fut, H>(factory: F, expected_fips: bool)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = H> + Send,
    H: Hsm + 'static,
{
    test_generate_sign_verify_round_trip(&factory).await;
    test_distinct_keys_produce_distinct_public_keys(&factory).await;
    test_sign_with_unknown_handle_is_not_found(&factory).await;
    test_get_public_key_with_unknown_handle_is_not_found(&factory).await;
    test_is_fips_validated_matches_expected(&factory, expected_fips).await;
    test_random_message_sign_verify_property(&factory).await;
}

/// Generates a fresh `EcdsaP256` key and exports its public half.
async fn generate_and_export<H: Hsm>(hsm: &H) -> (KeyHandle, PublicKey) {
    let handle = hsm
        .generate_key(KeySpec::EcdsaP256)
        .await
        .unwrap_or_else(|err| panic!("generate_key should succeed: {err}"));
    let public_key = hsm
        .get_public_key(&handle)
        .await
        .unwrap_or_else(|err| panic!("get_public_key should succeed: {err}"));
    (handle, public_key)
}

/// Signs `message` with `handle` and checks the signature verifies under
/// `public_key`'s exported SPKI DER (spec §14.1: fixed-width 64-byte P1363
/// `r || s`, the encoding every `Hsm` backend must produce).
async fn assert_sign_verifies<H: Hsm>(
    hsm: &H,
    handle: &KeyHandle,
    public_key: &PublicKey,
    message: &[u8],
) {
    let verifying_key = VerifyingKey::from_public_key_der(public_key.spki_der())
        .unwrap_or_else(|err| panic!("exported SPKI DER must parse: {err}"));
    let signature_bytes = hsm
        .sign(handle, message)
        .await
        .unwrap_or_else(|err| panic!("sign should succeed: {err}"));
    assert_eq!(
        signature_bytes.len(),
        64,
        "EcdsaP256 signatures are 64-byte P1363 r||s"
    );
    let signature = Signature::from_slice(&signature_bytes)
        .unwrap_or_else(|err| panic!("64-byte signature must parse: {err}"));
    verifying_key
        .verify(message, &signature)
        .unwrap_or_else(|err| panic!("signature must verify under the exported public key: {err}"));
}

async fn test_generate_sign_verify_round_trip<F, Fut, H>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = H> + Send,
    H: Hsm,
{
    let hsm = factory().await;
    let (handle, public_key) = generate_and_export(&hsm).await;
    for message in DETERMINISTIC_MESSAGES {
        assert_sign_verifies(&hsm, &handle, &public_key, message).await;
    }
}

async fn test_distinct_keys_produce_distinct_public_keys<F, Fut, H>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = H> + Send,
    H: Hsm,
{
    let hsm = factory().await;
    let (handle_a, public_key_a) = generate_and_export(&hsm).await;
    let (handle_b, public_key_b) = generate_and_export(&hsm).await;
    assert_ne!(handle_a, handle_b, "handles must be distinct");
    assert_ne!(
        public_key_a.spki_der(),
        public_key_b.spki_der(),
        "distinct keys must export distinct public keys"
    );
}

async fn test_sign_with_unknown_handle_is_not_found<F, Fut, H>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = H> + Send,
    H: Hsm,
{
    let hsm = factory().await;
    let result = hsm
        .sign(&KeyHandle::new("cts/hsm/no-such-key"), b"data")
        .await;
    let Err(err) = result else {
        panic!("sign with an unknown handle must fail, not panic or succeed");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}

async fn test_get_public_key_with_unknown_handle_is_not_found<F, Fut, H>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = H> + Send,
    H: Hsm,
{
    let hsm = factory().await;
    let result = hsm
        .get_public_key(&KeyHandle::new("cts/hsm/no-such-key"))
        .await;
    let Err(err) = result else {
        panic!("get_public_key with an unknown handle must fail");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}

async fn test_is_fips_validated_matches_expected<F, Fut, H>(factory: &F, expected_fips: bool)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = H> + Send,
    H: Hsm,
{
    let hsm = factory().await;
    assert_eq!(hsm.is_fips_validated(), expected_fips);
    // Generating and signing must not change the reported posture.
    let (handle, public_key) = generate_and_export(&hsm).await;
    assert_sign_verifies(&hsm, &handle, &public_key, b"posture-check").await;
    assert_eq!(hsm.is_fips_validated(), expected_fips);
}

async fn test_random_message_sign_verify_property<F, Fut, H>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = H> + Send,
    H: Hsm,
{
    let hsm = factory().await;
    let (handle, public_key) = generate_and_export(&hsm).await;

    let mut runner = TestRunner::default();
    let message_strategy = proptest::collection::vec(proptest::prelude::any::<u8>(), 0..256);
    for _ in 0..RANDOM_MESSAGE_CASES {
        let tree = message_strategy
            .new_tree(&mut runner)
            .unwrap_or_else(|reason| panic!("proptest message generation failed: {reason}"));
        let message = tree.current();
        assert_sign_verifies(&hsm, &handle, &public_key, &message).await;
    }
}
