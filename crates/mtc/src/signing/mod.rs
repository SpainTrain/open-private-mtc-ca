//! Algorithm-agnostic signature-scheme abstraction and its v1 algorithm.
//!
//! This module models the *cosigner signature* concept of the MTC draft
//! (`draft-ietf-plants-merkle-tree-certs-03` §5.4: a cosigner signs an
//! `MTCSubtreeSignatureInput` — a checkpoint signature when the subtree starts
//! at zero) and the architecture spec's key roster (spec §14.1: ECDSA P-256 is
//! the v1 algorithm for the checkpoint, pruning, revocation, and reporting
//! keys; ML-DSA-65 is the v2/post-quantum algorithm). The abstraction is
//! deliberately algorithm-agnostic so ML-DSA slots in behind a feature flag as
//! a second [`SignatureScheme`] implementation (ticket `mtclib-ml-dsa-quxpqc`)
//! without touching any caller.
//!
//! # What the draft does and does not fix
//!
//! The draft (§5.4.2) enumerates its signature algorithms *by name*
//! (ECDSA P-256/SHA-256, ECDSA P-384/SHA-384, Ed25519, ML-DSA-44/65/87) and
//! binds the algorithm to the cosigner's **public key / trust-anchor ID** — it
//! assigns **no numeric wire codepoints** and prescribes **no signature
//! encoding** (`MTCSubtreeSignatureInput` carries no algorithm field; "Log
//! clients ... are assumed to be configured with all parameters necessary to
//! verify that cosigner's signatures, including the signature algorithm and
//! version of the signature format"). Two consequences shape this module:
//!
//! 1. [`SignatureAlgorithm`] is a closed enum matching the draft's named set.
//!    Its numeric [`code`](SignatureAlgorithm::code) values are the **IANA TLS
//!    `SignatureScheme` codepoints** (RFC 8446 §4.2.3 for the classical
//!    schemes; `draft-ietf-tls-mldsa` for the ML-DSA schemes) — the natural,
//!    standards-grounded registry for these algorithms in the TLS/PKI
//!    ecosystem the MTC draft is built on. This is a *local* identifier
//!    registry; the on-wire binding of algorithm to key stays a draft §5.4
//!    concern owned by the trust-anchor-id and checkpoint tickets.
//! 2. The **signature encoding is pinned to the repository-wide contract**: for
//!    ECDSA P-256 a signature is the fixed 64-byte `r || s` IEEE P1363 form,
//!    matching `cloud_types::Hsm::sign`'s documented output ("the fixed-width
//!    64-byte `r || s` encoding (IEEE P1363, PKCS#11 native output) — not
//!    ASN.1 DER"). A software signer here and a future HSM-backed signer must
//!    therefore produce byte-identical signatures, so the same [`verify`] path
//!    checks both.
//!
//! [`verify`]: SignatureScheme::verify
//!
//! # Backing a signer with the HSM later (spec §14, §9.1)
//!
//! Production checkpoint signing runs on `cloud_types::Hsm` (async, key held
//! behind a `KeyHandle`). This module is deliberately **domain-typed only**
//! (spec §22.8: no vendor SDK types cross the boundary): [`Signature`] is the
//! 64-byte P1363 blob an HSM returns, [`VerifyingKey`] wraps the DER
//! `SubjectPublicKeyInfo` an HSM exports (identical representation to
//! `cloud_types::PublicKey`), and [`SignatureScheme::verify`] is a pure
//! function of those bytes. An HSM-backed signer added in the cloud/ca-service
//! layer produces exactly these [`Signature`]/[`VerifyingKey`] values, so the
//! verification callers in this crate (checkpoint, `MTCProof`) never change.
//! The
//! software [`sign`](SignatureScheme::sign) path in this module is the
//! dev/test/library stand-in.
//!
//! # Dispatch (spec §22.7)
//!
//! Signing and verifying a checkpoint happen once per checkpoint, not per byte
//! — this is not a hot path — so [`SignatureScheme`] is object-safe and the
//! registry ([`scheme_for`]) hands out `&'static dyn SignatureScheme`. The
//! per-byte hot paths (tree hashing, serialization) use generics elsewhere.

use core::fmt;

use thiserror::Error;
use zeroize::Zeroizing;

mod ecdsa_p256;

pub use ecdsa_p256::EcdsaP256;

/// A signature algorithm identifier: the closed set the MTC draft defines
/// (`draft-ietf-plants-merkle-tree-certs-03` §5.4.2).
///
/// The draft names these algorithms and binds each to a cosigner's public key;
/// it assigns no numeric codepoints. We adopt the **IANA TLS `SignatureScheme`
/// registry** values as the local numeric identifier ([`code`](Self::code)) —
/// see the module docs. The set is closed and exhaustively matchable (spec
/// §22.3): a future draft algorithm is an intentional breaking change, exactly
/// as `cloud_types::KeySpec` treats the v2 ML-DSA addition.
///
/// Presence in this enum is independent of whether *this build* can sign or
/// verify with the algorithm: [`scheme_for`] returns
/// [`UnsupportedAlgorithm`] for a known algorithm with no compiled
/// implementation (ML-DSA without the `ml-dsa` feature; the P-384/Ed25519
/// schemes, which are outside this CA's v1/v2 scope). That split is what lets
/// a feature-off build *parse* an ML-DSA identifier and answer "unsupported"
/// rather than panicking (ticket `mtclib-ml-dsa-quxpqc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureAlgorithm {
    /// ECDSA on NIST P-256 with SHA-256 (draft §5.4.2; spec §14.1 v1
    /// algorithm). IANA TLS `ecdsa_secp256r1_sha256` = `0x0403`.
    EcdsaP256Sha256,
    /// ECDSA on NIST P-384 with SHA-384 (draft §5.4.2). IANA TLS
    /// `ecdsa_secp384r1_sha384` = `0x0503`. Outside this CA's key roster
    /// (spec §14.1); present so the registry matches the draft.
    EcdsaP384Sha384,
    /// Ed25519 (draft §5.4.2; RFC 8032). IANA TLS `ed25519` = `0x0807`.
    /// Outside this CA's key roster (spec §14.1).
    Ed25519,
    /// ML-DSA-44 (draft §5.4.2; FIPS 204). IANA TLS `mldsa44` = `0x0904`
    /// (`draft-ietf-tls-mldsa`).
    MlDsa44,
    /// ML-DSA-65 (draft §5.4.2; FIPS 204; spec §14.1 v2 algorithm). IANA TLS
    /// `mldsa65` = `0x0905` (`draft-ietf-tls-mldsa`). Implemented behind the
    /// `ml-dsa` feature (ticket `mtclib-ml-dsa-quxpqc`).
    MlDsa65,
    /// ML-DSA-87 (draft §5.4.2; FIPS 204). IANA TLS `mldsa87` = `0x0906`
    /// (`draft-ietf-tls-mldsa`).
    MlDsa87,
}

impl SignatureAlgorithm {
    /// The algorithm's local numeric identifier: its IANA TLS
    /// `SignatureScheme` codepoint (see the type docs).
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::EcdsaP256Sha256 => 0x0403,
            Self::EcdsaP384Sha384 => 0x0503,
            Self::Ed25519 => 0x0807,
            Self::MlDsa44 => 0x0904,
            Self::MlDsa65 => 0x0905,
            Self::MlDsa87 => 0x0906,
        }
    }

    /// Resolves a numeric identifier back to its algorithm.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownAlgorithm`] if `code` is not one of the registry's
    /// codepoints — an untrusted or newer identifier is a typed error, never a
    /// panic (spec §22.6).
    pub const fn from_code(code: u16) -> Result<Self, UnknownAlgorithm> {
        match code {
            0x0403 => Ok(Self::EcdsaP256Sha256),
            0x0503 => Ok(Self::EcdsaP384Sha384),
            0x0807 => Ok(Self::Ed25519),
            0x0904 => Ok(Self::MlDsa44),
            0x0905 => Ok(Self::MlDsa65),
            0x0906 => Ok(Self::MlDsa87),
            other => Err(UnknownAlgorithm { code: other }),
        }
    }

    /// A stable, human-readable name for logs and telemetry.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EcdsaP256Sha256 => "ecdsa_secp256r1_sha256",
            Self::EcdsaP384Sha384 => "ecdsa_secp384r1_sha384",
            Self::Ed25519 => "ed25519",
            Self::MlDsa44 => "mldsa44",
            Self::MlDsa65 => "mldsa65",
            Self::MlDsa87 => "mldsa87",
        }
    }
}

impl fmt::Display for SignatureAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A digital signature, as the scheme-defined opaque byte string.
///
/// The encoding is the scheme's own: for [`SignatureAlgorithm::EcdsaP256Sha256`]
/// it is the fixed 64-byte `r || s` IEEE P1363 form (the repository-wide
/// contract shared with `cloud_types::Hsm::sign`); ML-DSA signatures are
/// larger (ML-DSA-65 is 3309 bytes) and variable, which is why this newtype
/// stores a length-flexible byte buffer rather than a fixed array. The bytes
/// are not interpreted here — [`SignatureScheme::verify`] parses and validates
/// them per algorithm.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signature(Box<[u8]>);

impl Signature {
    /// Wraps raw scheme-defined signature bytes.
    #[must_use]
    pub fn from_bytes(bytes: impl Into<Box<[u8]>>) -> Self {
        Self(bytes.into())
    }

    /// Borrows the raw signature bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The length of the signature in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the signature is empty (a malformed zero-length blob).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A public verification key, as DER-encoded `SubjectPublicKeyInfo`
/// (RFC 5280), tagged with its algorithm.
///
/// The DER `SubjectPublicKeyInfo` representation is exactly what
/// `cloud_types::Hsm::get_public_key` exports and what `cloud_types::PublicKey`
/// wraps, so an HSM-exported key crosses into this abstraction unchanged (spec
/// §22.8, §14). The algorithm tag drives registry dispatch ([`scheme_for`]) and
/// lets [`SignatureScheme::verify`] reject a key handed to the wrong scheme
/// before touching the curve math. The SPKI bytes are validated lazily: a
/// malformed key surfaces as [`VerifyError::MalformedKey`] at verify time, not
/// as a panic.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VerifyingKey {
    algorithm: SignatureAlgorithm,
    spki_der: Box<[u8]>,
}

impl VerifyingKey {
    /// Wraps DER-encoded `SubjectPublicKeyInfo` bytes for `algorithm`.
    ///
    /// This is a raw, unvalidated constructor for keys arriving from a trusted
    /// producer (e.g. `cloud_types::PublicKey` from the HSM). Cryptographic
    /// validity of the encoded point is checked when the key is used in
    /// [`SignatureScheme::verify`]. Scheme constructors such as
    /// [`EcdsaP256::verifying_key_from_sec1`] validate eagerly instead.
    #[must_use]
    pub fn from_spki_der(algorithm: SignatureAlgorithm, spki_der: impl Into<Box<[u8]>>) -> Self {
        Self {
            algorithm,
            spki_der: spki_der.into(),
        }
    }

    /// The algorithm this key is for.
    #[must_use]
    pub const fn algorithm(&self) -> SignatureAlgorithm {
        self.algorithm
    }

    /// Borrows the DER-encoded `SubjectPublicKeyInfo` bytes.
    #[must_use]
    pub fn spki_der(&self) -> &[u8] {
        &self.spki_der
    }
}

/// A private signing key, as scheme-defined secret bytes, zeroized on drop.
///
/// For [`SignatureAlgorithm::EcdsaP256Sha256`] the secret is the 32-byte
/// big-endian private scalar. The bytes are wrapped in [`Zeroizing`] so key
/// material is scrubbed from memory when the value is dropped (crypto hygiene;
/// the concrete key the scheme reconstructs per operation zeroizes likewise).
/// Construct instances through a scheme (e.g.
/// [`EcdsaP256::signing_key_from_bytes`] or
/// [`EcdsaP256::generate_keypair`]), which validate the secret.
#[derive(Clone)]
pub struct SigningKey {
    algorithm: SignatureAlgorithm,
    secret: Zeroizing<Vec<u8>>,
}

impl SigningKey {
    /// The algorithm this key is for.
    #[must_use]
    pub const fn algorithm(&self) -> SignatureAlgorithm {
        self.algorithm
    }

    /// Constructs from already-validated secret bytes (scheme-internal).
    pub(crate) fn from_validated_secret(algorithm: SignatureAlgorithm, secret: Vec<u8>) -> Self {
        Self {
            algorithm,
            secret: Zeroizing::new(secret),
        }
    }

    /// The raw secret bytes (scheme-internal — never leaves the crate).
    pub(crate) fn secret_bytes(&self) -> &[u8] {
        &self.secret
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never render secret key material (crypto hygiene).
        f.debug_struct("SigningKey")
            .field("algorithm", &self.algorithm)
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// An algorithm-agnostic signing and verification scheme.
///
/// Each algorithm (ECDSA P-256 here; ML-DSA-65 behind the `ml-dsa` feature in
/// ticket `mtclib-ml-dsa-quxpqc`) is one implementation. The trait is
/// object-safe so [`scheme_for`] can hand out `&'static dyn SignatureScheme`;
/// signing and verifying a checkpoint are not per-byte hot paths, so dynamic
/// dispatch here is the right side of the spec §22.7 line. All parameters and
/// results are domain types (spec §22.8).
pub trait SignatureScheme {
    /// The algorithm this scheme implements.
    fn algorithm(&self) -> SignatureAlgorithm;

    /// Signs `message` with `key`, producing a scheme-encoded [`Signature`].
    ///
    /// ECDSA P-256 signing is RFC 6979 deterministic (no per-signature
    /// randomness), so the output is reproducible and directly testable
    /// against published vectors.
    ///
    /// # Errors
    ///
    /// Returns [`SignError`] if `key` is for a different algorithm than this
    /// scheme, or if the secret key material is malformed (spec §22.6 — never a
    /// panic).
    fn sign(&self, key: &SigningKey, message: &[u8]) -> Result<Signature, SignError>;

    /// Verifies that `signature` is a valid signature over `message` under
    /// `key`.
    ///
    /// # Errors
    ///
    /// Returns [`VerifyError`] if the key is for a different algorithm, the key
    /// or signature bytes are malformed (including the degenerate ECDSA cases
    /// `r = 0` or `s = 0`), or the signature simply does not verify. Verifying
    /// arbitrary bytes never panics (spec §19.6, §22.6).
    #[must_use = "an unchecked signature verification result is a security bug (spec §22.10)"]
    fn verify(
        &self,
        key: &VerifyingKey,
        message: &[u8],
        signature: &Signature,
    ) -> Result<(), VerifyError>;
}

/// Returns the scheme implementing `algorithm` in this build.
///
/// This is the algorithm registry's dispatch point. It returns a shared,
/// zero-sized `&'static dyn SignatureScheme`.
///
/// # Errors
///
/// Returns [`UnsupportedAlgorithm`] when `algorithm` is a recognized draft
/// algorithm that this build has no implementation for — ML-DSA when the
/// `ml-dsa` feature is off, and the P-384/Ed25519 schemes that are outside this
/// CA's v1/v2 key roster (spec §14.1). Unknown *codepoints* are rejected
/// earlier, by [`SignatureAlgorithm::from_code`].
pub fn scheme_for(
    algorithm: SignatureAlgorithm,
) -> Result<&'static dyn SignatureScheme, UnsupportedAlgorithm> {
    match algorithm {
        SignatureAlgorithm::EcdsaP256Sha256 => Ok(&EcdsaP256),
        other => Err(UnsupportedAlgorithm { algorithm: other }),
    }
}

/// A numeric algorithm identifier did not match any registry codepoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("unknown signature-algorithm codepoint {code:#06x}")]
pub struct UnknownAlgorithm {
    /// The unrecognized codepoint.
    pub code: u16,
}

/// A recognized algorithm has no signing/verification implementation in this
/// build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("signature algorithm {algorithm} is not supported by this build")]
pub struct UnsupportedAlgorithm {
    /// The recognized-but-unimplemented algorithm.
    pub algorithm: SignatureAlgorithm,
}

/// Reasons a key's bytes were rejected as malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum KeyRejected {
    /// The secret or public key was the wrong length for its algorithm.
    #[error("key has wrong length: expected {expected} bytes, got {actual}")]
    WrongLength {
        /// Expected length in bytes.
        expected: usize,
        /// Actual length supplied.
        actual: usize,
    },
    /// The bytes did not decode to a valid key (bad DER, off-curve point,
    /// zero/identity value, or scalar out of range).
    #[error("key bytes are not a valid {algorithm} key")]
    Invalid {
        /// The algorithm the key was being decoded for.
        algorithm: SignatureAlgorithm,
    },
    /// The key belongs to a different algorithm than the scheme.
    #[error("algorithm mismatch: scheme is {expected}, key is {actual}")]
    AlgorithmMismatch {
        /// The scheme's algorithm.
        expected: SignatureAlgorithm,
        /// The key's algorithm.
        actual: SignatureAlgorithm,
    },
}

/// Signing failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SignError {
    /// The signing key was rejected.
    #[error(transparent)]
    Key(#[from] KeyRejected),
}

/// Verification failed.
///
/// Distinguishing malformed inputs from a cryptographically-invalid-but-
/// well-formed signature aids triage without exposing a padding/timing oracle
/// (these are structural, not secret-dependent distinctions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum VerifyError {
    /// The verifying key belongs to a different algorithm than the scheme.
    #[error("algorithm mismatch: scheme is {expected}, key is {actual}")]
    AlgorithmMismatch {
        /// The scheme's algorithm.
        expected: SignatureAlgorithm,
        /// The key's algorithm.
        actual: SignatureAlgorithm,
    },
    /// The public key bytes did not decode to a valid key.
    #[error("malformed verifying key")]
    MalformedKey,
    /// The signature bytes were not a valid encoding for the algorithm —
    /// including the degenerate ECDSA cases `r = 0` or `s = 0`, and any wrong
    /// length.
    #[error("malformed signature")]
    MalformedSignature,
    /// The signature is well-formed but does not verify over this message and
    /// key.
    #[error("signature verification failed")]
    BadSignature,
}

#[cfg(test)]
mod tests {
    use super::{
        scheme_for, SignatureAlgorithm, UnknownAlgorithm, UnsupportedAlgorithm, VerifyingKey,
    };

    #[test]
    fn algorithm_codepoints_round_trip_via_iana_tls_values() {
        // The six draft §5.4.2 algorithms and their IANA TLS SignatureScheme
        // codepoints (RFC 8446 + draft-ietf-tls-mldsa).
        let cases = [
            (SignatureAlgorithm::EcdsaP256Sha256, 0x0403u16),
            (SignatureAlgorithm::EcdsaP384Sha384, 0x0503),
            (SignatureAlgorithm::Ed25519, 0x0807),
            (SignatureAlgorithm::MlDsa44, 0x0904),
            (SignatureAlgorithm::MlDsa65, 0x0905),
            (SignatureAlgorithm::MlDsa87, 0x0906),
        ];
        for (algorithm, code) in cases {
            assert_eq!(algorithm.code(), code, "{algorithm} code");
            assert_eq!(SignatureAlgorithm::from_code(code), Ok(algorithm));
        }
    }

    #[test]
    fn unknown_codepoints_parse_to_a_structured_error() {
        for code in [0x0000u16, 0x0001, 0x0404, 0x0900, 0xFFFF] {
            assert_eq!(
                SignatureAlgorithm::from_code(code),
                Err(UnknownAlgorithm { code }),
            );
        }
    }

    #[test]
    fn algorithm_display_is_the_iana_name() {
        assert_eq!(
            SignatureAlgorithm::EcdsaP256Sha256.to_string(),
            "ecdsa_secp256r1_sha256",
        );
        assert_eq!(SignatureAlgorithm::MlDsa65.to_string(), "mldsa65");
    }

    #[test]
    fn registry_resolves_the_v1_algorithm() {
        let scheme = scheme_for(SignatureAlgorithm::EcdsaP256Sha256)
            .expect("ECDSA P-256 is implemented in the default build");
        assert_eq!(scheme.algorithm(), SignatureAlgorithm::EcdsaP256Sha256);
    }

    #[test]
    fn registry_reports_unimplemented_algorithms_as_unsupported() {
        // ML-DSA (feature off in the default build) and the out-of-scope
        // classical schemes are known but unsupported — never a panic. This is
        // the behaviour the ml-dsa feature-off build relies on.
        for algorithm in [
            SignatureAlgorithm::EcdsaP384Sha384,
            SignatureAlgorithm::Ed25519,
            SignatureAlgorithm::MlDsa44,
            SignatureAlgorithm::MlDsa65,
            SignatureAlgorithm::MlDsa87,
        ] {
            // `.err()` (not `.unwrap_err()`): the Ok type `&dyn SignatureScheme`
            // is not `Debug`.
            assert_eq!(
                scheme_for(algorithm).err(),
                Some(UnsupportedAlgorithm { algorithm }),
            );
        }
    }

    #[test]
    fn verifying_key_keeps_its_algorithm_tag_and_bytes() {
        let vk = VerifyingKey::from_spki_der(SignatureAlgorithm::EcdsaP256Sha256, vec![0x30, 0x59]);
        assert_eq!(vk.algorithm(), SignatureAlgorithm::EcdsaP256Sha256);
        assert_eq!(vk.spki_der(), &[0x30, 0x59]);
    }
}
