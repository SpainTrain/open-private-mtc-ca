//! [`MemoryHsm`] — pure in-memory [`Hsm`] (spec §9.3, §14).
//!
//! **Dev-only by definition.** Private keys live in process memory, signed
//! with `RustCrypto` `p256` (spec §14.1) rather than inside any hardware
//! boundary — [`Hsm::is_fips_validated`] always returns `false`, and callers
//! (compliance reporting, the `fips-boundary-preserved` CI gate) must be able
//! to trust that honesty rather than infer it. Never wire this backend into a
//! production `Backend` (spec §14.4).
//!
//! # Key material handling
//!
//! [`p256::ecdsa::SigningKey`] implements `ZeroizeOnDrop` via
//! `elliptic-curve`'s default `"zeroize"` feature: dropping a stored key
//! (e.g. dropping this `MemoryHsm`) zeroizes the private scalar rather than
//! leaving it in freed memory. This crate adds no `unsafe` code of its own to
//! achieve that — it is a property of the `RustCrypto` type we store, pinned
//! by the compile-time assertion in this module's tests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use cloud_types::{CloudError, Hsm, KeyHandle, KeySpec, PublicKey};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use p256::pkcs8::EncodePublicKey;

/// Pure in-memory, process-local [`Hsm`] (spec §9.3, §9.6). See the
/// module docs for the FIPS posture and key-zeroization story.
#[derive(Debug, Default)]
pub struct MemoryHsm {
    keys: Mutex<HashMap<KeyHandle, SigningKey>>,
    // Monotonic counter for handle allocation; a plain u64 (not a domain
    // newtype) since it never escapes this module as anything but part of an
    // opaque KeyHandle string.
    next_id: AtomicU64,
}

impl MemoryHsm {
    /// Creates an empty in-memory HSM with no keys.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock_keys(&self) -> MutexGuard<'_, HashMap<KeyHandle, SigningKey>> {
        self.keys.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Draws a uniformly random P-256 signing key.
///
/// `SigningKey::from_slice` rejects the zero scalar and any value at or
/// above the curve order; both are astronomically unlikely from 32
/// CSPRNG-drawn bytes (the P-256 order is within 2^-32 of 2^256), but the
/// bounded retry loop keeps this function total rather than relying on that
/// probability never being exercised.
fn random_signing_key() -> Result<SigningKey, CloudError> {
    const MAX_ATTEMPTS: u32 = 8;
    for _ in 0..MAX_ATTEMPTS {
        let mut scalar_bytes = [0u8; 32];
        getrandom::fill(&mut scalar_bytes).map_err(|error| CloudError::Transport {
            retryable: true,
            reason: format!("CSPRNG failure: {error}"),
        })?;
        if let Ok(key) = SigningKey::from_slice(&scalar_bytes) {
            return Ok(key);
        }
    }
    Err(CloudError::Transport {
        retryable: false,
        reason: "failed to draw a valid P-256 scalar after repeated attempts".to_string(),
    })
}

#[async_trait]
impl Hsm for MemoryHsm {
    async fn sign(&self, key_handle: &KeyHandle, data: &[u8]) -> Result<Vec<u8>, CloudError> {
        let keys = self.lock_keys();
        let signing_key = keys.get(key_handle).ok_or_else(|| CloudError::NotFound {
            key: key_handle.as_str().to_string(),
        })?;
        // `Signer<Signature>` for `SigningKey` digests `data` with SHA-256
        // internally, matching KeySpec::EcdsaP256 (cloud-types rustdoc).
        let signature: Signature = signing_key.sign(data);
        // Release the lock before the (already-complete) work of returning;
        // nothing below touches the key map (clippy::significant_drop_tightening).
        drop(keys);
        // Fixed-width 64-byte r||s (IEEE P1363) — the encoding every Hsm
        // backend must produce (cloud-types rustdoc on Hsm::sign).
        Ok(signature.to_bytes().to_vec())
    }

    async fn get_public_key(&self, key_handle: &KeyHandle) -> Result<PublicKey, CloudError> {
        let keys = self.lock_keys();
        let signing_key = keys.get(key_handle).ok_or_else(|| CloudError::NotFound {
            key: key_handle.as_str().to_string(),
        })?;
        let verifying_key = VerifyingKey::from(signing_key);
        // DER-encoding does not touch the key map; release the lock before
        // it rather than holding it for unrelated work
        // (clippy::significant_drop_tightening).
        drop(keys);
        let der = verifying_key
            .to_public_key_der()
            .map_err(|error| CloudError::Transport {
                retryable: false,
                reason: format!("SPKI DER encoding failed: {error}"),
            })?;
        Ok(PublicKey::from_spki_der(der.as_bytes().to_vec()))
    }

    async fn generate_key(&self, spec: KeySpec) -> Result<KeyHandle, CloudError> {
        match spec {
            KeySpec::EcdsaP256 => {
                let signing_key = random_signing_key()?;
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                let handle = KeyHandle::new(format!("mem-hsm-{id:016x}"));
                self.lock_keys().insert(handle.clone(), signing_key);
                Ok(handle)
            }
        }
    }

    fn is_fips_validated(&self) -> bool {
        // Dev-only by definition (spec §14.4) — never true for this backend.
        false
    }
}

#[cfg(test)]
mod tests {
    use p256::ecdsa::signature::Verifier as _;
    use p256::pkcs8::DecodePublicKey;
    use pretty_assertions::assert_eq;
    use zeroize::ZeroizeOnDrop;

    use super::*;

    #[tokio::test]
    async fn generate_sign_verify_round_trip() {
        let hsm = MemoryHsm::new();
        let handle = hsm
            .generate_key(KeySpec::EcdsaP256)
            .await
            .expect("generate_key succeeds");

        let message = b"checkpoint bytes";
        let signature_bytes = hsm.sign(&handle, message).await.expect("sign succeeds");
        assert_eq!(signature_bytes.len(), 64, "P1363 r||s encoding for P-256");

        let public_key = hsm
            .get_public_key(&handle)
            .await
            .expect("get_public_key succeeds");
        let verifying_key =
            VerifyingKey::from_public_key_der(public_key.spki_der()).expect("valid SPKI DER");
        let signature = Signature::from_slice(&signature_bytes).expect("64-byte signature parses");
        verifying_key
            .verify(message, &signature)
            .expect("signature verifies under the exported public key");
    }

    #[tokio::test]
    async fn distinct_generated_keys_produce_distinct_public_keys() {
        let hsm = MemoryHsm::new();
        let a = hsm.generate_key(KeySpec::EcdsaP256).await.expect("gen a");
        let b = hsm.generate_key(KeySpec::EcdsaP256).await.expect("gen b");
        assert_ne!(a, b, "handles are distinct");

        let pk_a = hsm.get_public_key(&a).await.expect("pk a");
        let pk_b = hsm.get_public_key(&b).await.expect("pk b");
        assert_ne!(pk_a.spki_der(), pk_b.spki_der());
    }

    #[tokio::test]
    async fn sign_with_unknown_handle_is_not_found_never_panics() {
        let hsm = MemoryHsm::new();
        let err = hsm
            .sign(&KeyHandle::new("no-such-key"), b"data")
            .await
            .expect_err("unknown handle must error, not panic");
        assert!(matches!(err, CloudError::NotFound { .. }));
    }

    #[tokio::test]
    async fn get_public_key_with_unknown_handle_is_not_found() {
        let hsm = MemoryHsm::new();
        assert!(matches!(
            hsm.get_public_key(&KeyHandle::new("no-such-key")).await,
            Err(CloudError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn is_fips_validated_is_always_false() {
        let hsm = MemoryHsm::new();
        assert!(!hsm.is_fips_validated());
        // Signing/generating a key must not change the posture.
        let handle = hsm.generate_key(KeySpec::EcdsaP256).await.expect("gen");
        hsm.sign(&handle, b"x").await.expect("sign");
        assert!(!hsm.is_fips_validated());
    }

    /// Compile-time proof, not a runtime check (inspecting freed memory would
    /// require `unsafe`, forbidden by rule `no-unsafe`): the private key type
    /// this module stores implements `ZeroizeOnDrop`, so key material does
    /// not linger after a key (or the whole `MemoryHsm`) is dropped.
    #[test]
    fn signing_key_material_is_zeroized_on_drop() {
        const fn assert_zeroize_on_drop<T: ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<SigningKey>();
    }
}
