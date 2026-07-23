//! [`Hsm`] — hardware-backed signing (spec §9.1, §14).
//!
//! Backends: AWS `CloudHSM` / GCP Cloud HSM / Azure Managed HSM / on-prem
//! PKCS#11 / `SoftHSM2` (dev) / pure memory (tests).
//!
//! Private keys live inside the HSM and never cross this boundary: the trait
//! exposes handles, public keys, and signatures only. The §9.5 capabilities
//! anchoring these contracts are **HSM signing** (FIPS 140-2 Level 3 or
//! equivalent key protection) and **HSM cross-region key access** (the key is
//! available wherever the primary is — spec §14.2).

use async_trait::async_trait;

use crate::errors::CloudError;

/// An opaque reference to a private key held inside the HSM (newtype —
/// .claude/rules/use-newtypes).
///
/// The wrapped string is backend-defined (a `CloudHSM` key label, a PKCS#11
/// token label/ID pair, a map key for the memory backend) and stable across
/// process restarts for durable backends. It carries no key material.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyHandle(String);

impl KeyHandle {
    /// Wraps a backend-defined handle identifier.
    #[must_use]
    pub fn new(handle: impl Into<String>) -> Self {
        Self(handle.into())
    }

    /// Borrows the handle identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the handle, returning the identifier string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for KeyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A public key exported from the HSM, DER-encoded as an X.509
/// `SubjectPublicKeyInfo` (RFC 5280) — the interoperable exchange format
/// every backend must produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey(Vec<u8>);

impl PublicKey {
    /// Wraps DER-encoded `SubjectPublicKeyInfo` bytes.
    #[must_use]
    pub const fn from_spki_der(der: Vec<u8>) -> Self {
        Self(der)
    }

    /// Borrows the DER-encoded `SubjectPublicKeyInfo` bytes.
    #[must_use]
    pub fn spki_der(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the key, returning the DER-encoded `SubjectPublicKeyInfo`
    /// bytes.
    #[must_use]
    pub fn into_spki_der(self) -> Vec<u8> {
        self.0
    }
}

/// The algorithm and parameters of a key to generate (spec §14.1).
///
/// v1 signs everything with ECDSA P-256. The v2 ML-DSA-65 variant arrives
/// behind the post-quantum feature work (spec §14.1, roadmap §24) and will be
/// added here as a new variant — an intentionally breaking change every
/// backend must then handle (spec §22.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeySpec {
    /// ECDSA on NIST P-256 with SHA-256 digests (v1 algorithm for
    /// checkpoint, pruning, revocation, and reporting keys — spec §14.1).
    EcdsaP256,
}

/// Hardware-backed key generation and signing.
///
/// Object-safe and shared as `Arc<dyn Hsm>` from the `Backend` factory (spec
/// §9.4); `Send + Sync` supertraits allow concurrent signing from many tasks
/// (backends manage session pooling internally).
#[async_trait]
pub trait Hsm: Send + Sync {
    /// Signs `data` with the private key referenced by `key_handle`.
    ///
    /// `data` is the raw message; the backend performs digesting as required
    /// by the key's algorithm (SHA-256 for [`KeySpec::EcdsaP256`]).
    ///
    /// # Signature encoding
    ///
    /// For [`KeySpec::EcdsaP256`] the returned signature is the fixed-width
    /// 64-byte `r || s` encoding (IEEE P1363, PKCS#11 native output) — not
    /// ASN.1 DER. Every backend must produce this encoding so the shared
    /// conformance suite (spec §9.7) can verify signatures uniformly;
    /// consumers needing DER convert at their own edge.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// HSM signing: the private key is generated in and never leaves a FIPS
    /// 140-2 Level 3 (or equivalent) boundary in production backends; dev
    /// backends must report their true posture via
    /// [`Hsm::is_fips_validated`]. Performance target: <100ms p99 per
    /// signature (spec §14.3).
    ///
    /// # Errors
    ///
    /// - [`CloudError::NotFound`] — `key_handle` does not reference a key
    ///   (never a panic — spec §22.6).
    /// - [`CloudError::Transport`] — HSM/transport failure (see `retryable`).
    async fn sign(&self, key_handle: &KeyHandle, data: &[u8]) -> Result<Vec<u8>, CloudError>;

    /// Exports the public half of the key referenced by `key_handle` as
    /// DER-encoded `SubjectPublicKeyInfo`.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// HSM signing: only public material crosses the HSM boundary; the
    /// export must round-trip with standard verifiers (e.g. `RustCrypto`
    /// `p256` for [`KeySpec::EcdsaP256`]).
    ///
    /// # Errors
    ///
    /// - [`CloudError::NotFound`] — `key_handle` does not reference a key.
    /// - [`CloudError::Transport`] — HSM/transport failure.
    async fn get_public_key(&self, key_handle: &KeyHandle) -> Result<PublicKey, CloudError>;

    /// Generates a new keypair per `spec` inside the HSM and returns its
    /// handle.
    ///
    /// The private key is created non-exportable; the returned handle is
    /// stable for the lifetime of the key on durable backends.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// HSM signing and HSM cross-region key access: keys are generated inside
    /// the protection boundary, and production backends make them available
    /// in every region the primary can run in (per-region clusters with a
    /// key-replication ceremony — spec §14.2).
    ///
    /// # Errors
    ///
    /// - [`CloudError::Transport`] — HSM/transport failure.
    async fn generate_key(&self, spec: KeySpec) -> Result<KeyHandle, CloudError>;

    /// Reports whether this backend's signing operations are FIPS-validated
    /// (spec §14.4).
    ///
    /// FIPS validation is a property of the deployed HSM, not of this source:
    /// CloudHSM-backed implementations return `true` (validation inherited
    /// from the hardware); `SoftHSM2` and memory backends return `false` and
    /// are dev/test-only. Compliance reports include this value (spec §20.3),
    /// and CI blocks non-FIPS builds from production
    /// (.claude/rules/fips-boundary-preserved).
    ///
    /// Synchronous by design: the posture is a static property of the
    /// backend, required in non-async contexts (report rendering, startup
    /// validation).
    fn is_fips_validated(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::{KeyHandle, PublicKey};

    #[test]
    fn key_handle_round_trips() {
        let handle = KeyHandle::new("checkpoint-signing-v1");
        assert_eq!(handle.as_str(), "checkpoint-signing-v1");
        assert_eq!(handle.to_string(), "checkpoint-signing-v1");
        assert_eq!(handle.into_string(), "checkpoint-signing-v1");
    }

    #[test]
    fn public_key_round_trips_der_bytes() {
        let der = vec![0x30, 0x59, 0x30, 0x13];
        let key = PublicKey::from_spki_der(der.clone());
        assert_eq!(key.spki_der(), der.as_slice());
        assert_eq!(key.into_spki_der(), der);
    }
}
