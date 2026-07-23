//! ECDSA P-256 with SHA-256 — the v1 signature algorithm (spec §14.1, §24
//! Phase 1), implemented over `RustCrypto`'s `p256` (spec §28: reimplementing
//! curve primitives is out of the question; the `RustCrypto` crate is the
//! sanctioned dependency).
//!
//! # Contracts this implementation pins
//!
//! - **Signature encoding**: the fixed 64-byte `r || s` IEEE P1363 form, byte
//!   for byte the encoding `cloud_types::Hsm::sign` documents. Not ASN.1 DER.
//! - **Deterministic signing**: RFC 6979 (SHA-256), so signing takes no
//!   randomness and is reproducible — the property the known-answer tests
//!   below exploit against the RFC 6979 Appendix A.2.5 vectors.
//! - **Verification accepts high-`s`**: standard ECDSA verification does not
//!   reject signatures with `s > n/2`. PKCS#11 / HSM backends emit raw
//!   (non-normalized) `r || s`, so rejecting high-`s` would reject valid
//!   HSM-produced signatures. Verification *does* reject the degenerate
//!   `r = 0` / `s = 0` encodings (spec §19.6 crypto invariants).
//! - **Public keys** are DER `SubjectPublicKeyInfo`, matching
//!   `cloud_types::PublicKey`.

use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{
    Signature as P256Signature, SigningKey as P256SigningKey, VerifyingKey as P256VerifyingKey,
};
use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
use rand_core::OsRng;

use super::{
    KeyRejected, SignError, Signature, SignatureAlgorithm, SignatureScheme, SigningKey,
    VerifyError, VerifyingKey,
};

/// The algorithm this scheme implements.
const ALGORITHM: SignatureAlgorithm = SignatureAlgorithm::EcdsaP256Sha256;

/// The size in bytes of a P-256 private scalar and of each `r`/`s` half.
const SCALAR_LEN: usize = 32;

/// The size in bytes of a P-256 `r || s` IEEE P1363 signature.
const P1363_LEN: usize = 2 * SCALAR_LEN;

/// ECDSA P-256 / SHA-256, the v1 signature scheme (spec §14.1).
///
/// A zero-sized handle to the algorithm; obtain the shared instance from
/// [`scheme_for`](super::scheme_for), or use the inherent constructors
/// ([`signing_key_from_bytes`](Self::signing_key_from_bytes),
/// [`verifying_key_from_sec1`](Self::verifying_key_from_sec1),
/// [`generate_keypair`](Self::generate_keypair)) directly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EcdsaP256;

// Associated functions (no `&self`): `EcdsaP256` is a zero-sized strategy
// handle, so key construction is a type-level concern — no instance needed.
impl EcdsaP256 {
    /// Loads a signing key from a 32-byte big-endian private scalar.
    ///
    /// # Errors
    ///
    /// Returns [`KeyRejected::WrongLength`] if `bytes` is not 32 bytes, or
    /// [`KeyRejected::Invalid`] if the scalar is zero or not in `[1, n)` —
    /// degenerate private keys are refused, never accepted or panicked on
    /// (spec §19.6).
    pub fn signing_key_from_bytes(bytes: &[u8]) -> Result<SigningKey, KeyRejected> {
        if bytes.len() != SCALAR_LEN {
            return Err(KeyRejected::WrongLength {
                expected: SCALAR_LEN,
                actual: bytes.len(),
            });
        }
        // RustCrypto rejects a zero or out-of-range scalar here.
        let key = P256SigningKey::from_slice(bytes).map_err(|_| KeyRejected::Invalid {
            algorithm: ALGORITHM,
        })?;
        Ok(SigningKey::from_validated_secret(
            ALGORITHM,
            key.to_bytes().to_vec(),
        ))
    }

    /// Loads a verifying key from a SEC1-encoded point (compressed or
    /// uncompressed), re-encoding it as the canonical DER
    /// `SubjectPublicKeyInfo`.
    ///
    /// # Errors
    ///
    /// Returns [`KeyRejected::Invalid`] if the bytes are not a valid P-256
    /// point (off-curve, the identity, or malformed).
    pub fn verifying_key_from_sec1(sec1: &[u8]) -> Result<VerifyingKey, KeyRejected> {
        let key = P256VerifyingKey::from_sec1_bytes(sec1).map_err(|_| KeyRejected::Invalid {
            algorithm: ALGORITHM,
        })?;
        Ok(Self::to_domain_verifying_key(&key))
    }

    /// Generates a fresh keypair from the operating-system CSPRNG.
    ///
    /// Used for dev/test and the demo; production keys live in the HSM (spec
    /// §14). Key generation is the one operation here that consumes randomness
    /// — signing is deterministic (RFC 6979).
    #[must_use]
    pub fn generate_keypair() -> (SigningKey, VerifyingKey) {
        let signing = P256SigningKey::random(&mut OsRng);
        let verifying = Self::to_domain_verifying_key(signing.verifying_key());
        let signing_key = SigningKey::from_validated_secret(ALGORITHM, signing.to_bytes().to_vec());
        (signing_key, verifying)
    }

    /// Encodes a `RustCrypto` verifying key as a domain [`VerifyingKey`]
    /// carrying its DER `SubjectPublicKeyInfo`.
    fn to_domain_verifying_key(key: &P256VerifyingKey) -> VerifyingKey {
        // `EncodePublicKey` for a P-256 key is infallible in practice (fixed
        // structure), but we route any failure through the raw constructor
        // rather than unwrap, keeping this panic-free (spec §22.6).
        key.to_public_key_der().map_or_else(
            |_| VerifyingKey::from_spki_der(ALGORITHM, Vec::new()),
            |der| VerifyingKey::from_spki_der(ALGORITHM, der.as_bytes().to_vec()),
        )
    }

    /// Reconstructs the `RustCrypto` verifying key from a domain key's SPKI
    /// DER.
    fn parse_verifying_key(key: &VerifyingKey) -> Result<P256VerifyingKey, VerifyError> {
        if key.algorithm() != ALGORITHM {
            return Err(VerifyError::AlgorithmMismatch {
                expected: ALGORITHM,
                actual: key.algorithm(),
            });
        }
        P256VerifyingKey::from_public_key_der(key.spki_der()).map_err(|_| VerifyError::MalformedKey)
    }
}

impl SignatureScheme for EcdsaP256 {
    fn algorithm(&self) -> SignatureAlgorithm {
        ALGORITHM
    }

    fn sign(&self, key: &SigningKey, message: &[u8]) -> Result<Signature, SignError> {
        if key.algorithm() != ALGORITHM {
            return Err(SignError::Key(KeyRejected::AlgorithmMismatch {
                expected: ALGORITHM,
                actual: key.algorithm(),
            }));
        }
        let signing = P256SigningKey::from_slice(key.secret_bytes()).map_err(|_| {
            SignError::Key(KeyRejected::Invalid {
                algorithm: ALGORITHM,
            })
        })?;
        // RFC 6979 deterministic signing over SHA-256(message).
        let signature: P256Signature = signing.try_sign(message).map_err(|_| {
            SignError::Key(KeyRejected::Invalid {
                algorithm: ALGORITHM,
            })
        })?;
        // `to_bytes()` is the 64-byte P1363 `r || s` form (the HSM contract).
        Ok(Signature::from_bytes(signature.to_bytes().to_vec()))
    }

    fn verify(
        &self,
        key: &VerifyingKey,
        message: &[u8],
        signature: &Signature,
    ) -> Result<(), VerifyError> {
        let verifying = Self::parse_verifying_key(key)?;
        // `from_slice` enforces the 64-byte length and rejects the degenerate
        // `r = 0` / `s = 0` encodings (spec §19.6). High-`s` is accepted, per
        // the HSM-compatibility note in the module docs.
        if signature.len() != P1363_LEN {
            return Err(VerifyError::MalformedSignature);
        }
        let parsed = P256Signature::from_slice(signature.as_bytes())
            .map_err(|_| VerifyError::MalformedSignature)?;
        verifying
            .verify(message, &parsed)
            .map_err(|_| VerifyError::BadSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::{EcdsaP256, ALGORITHM, P1363_LEN, SCALAR_LEN};
    use crate::signing::{
        KeyRejected, SignError, Signature, SignatureAlgorithm, SignatureScheme, VerifyError,
        VerifyingKey,
    };

    /// Decodes an uppercase/lowercase hex string into bytes (test-only).
    fn hexdec(s: &str) -> Vec<u8> {
        assert!(s.len().is_multiple_of(2), "odd-length hex");
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
            .collect()
    }

    // RFC 6979 Appendix A.2.5 — ECDSA, 256 bits (NIST P-256), private key and
    // its two SHA-256 message vectors. These are published standards text
    // (clean-room; spec §6), and exercise RFC 6979 deterministic signing plus
    // our P1363 `r || s` encoding end to end.
    const RFC6979_PRIV: &str = "C9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721";
    // Uncompressed SEC1 point 0x04 || Ux || Uy.
    const RFC6979_PUB_SEC1: &str = concat!(
        "04",
        "60FED4BA255A9D31C961EB74C6356D68C049B8923B61FA6CE669622E60F29FB6",
        "7903FE1008B8BC99A41AE9E95628BC64F2F1B20C2D7E9F5177A3C294D4462299",
    );
    // message = "sample": note s > n/2 (high-s) — a good normalization probe.
    const SAMPLE_R: &str = "EFD48B2AACB6A8FD1140DD9CD45E81D69D2C877B56AAF991C34D0EA84EAF3716";
    const SAMPLE_S: &str = "F7CB1C942D657C41D436C7A1B6E29F65F3E900DBB9AFF4064DC4AB2F843ACDA8";
    // message = "test": s < n/2 (low-s).
    const TEST_R: &str = "F1ABB023518351CD71D881567B1EA663ED3EFCF6C5132B354F28D3B0B7D38367";
    const TEST_S: &str = "019F4113742A2B14BD25926B49C649155F267E60D3814B4C0CC84250E46F0083";

    #[test]
    fn rfc6979_known_answer_vectors() {
        let scheme = EcdsaP256;
        let signing = EcdsaP256::signing_key_from_bytes(&hexdec(RFC6979_PRIV))
            .expect("RFC 6979 private key is valid");
        let verifying = EcdsaP256::verifying_key_from_sec1(&hexdec(RFC6979_PUB_SEC1))
            .expect("RFC 6979 public point is valid");

        for (message, r, s) in [
            (b"sample".as_slice(), SAMPLE_R, SAMPLE_S),
            (b"test".as_slice(), TEST_R, TEST_S),
        ] {
            let mut expected = hexdec(r);
            expected.extend_from_slice(&hexdec(s));
            assert_eq!(expected.len(), P1363_LEN);

            let signature = scheme.sign(&signing, message).expect("signing succeeds");
            assert_eq!(
                signature.as_bytes(),
                expected.as_slice(),
                "deterministic P1363 r||s must match RFC 6979 for {:?}",
                core::str::from_utf8(message).unwrap(),
            );

            // And the produced signature verifies.
            scheme
                .verify(&verifying, message, &signature)
                .expect("KAT signature verifies");
        }
    }

    #[test]
    fn sign_verify_round_trip_with_fresh_key() {
        let scheme = EcdsaP256;
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let message = b"checkpoint: tree_size=1000 root=deadbeef";
        let signature = scheme.sign(&signing, message).expect("sign");
        scheme
            .verify(&verifying, message, &signature)
            .expect("verify");
    }

    #[test]
    fn verify_rejects_tampered_message() {
        let scheme = EcdsaP256;
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let signature = scheme.sign(&signing, b"original message").expect("sign");
        assert_eq!(
            scheme.verify(&verifying, b"tampered message", &signature),
            Err(VerifyError::BadSignature),
        );
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        let scheme = EcdsaP256;
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let message = b"a message to sign";
        let good = scheme.sign(&signing, message).expect("sign");
        let mut bytes = good.as_bytes().to_vec();
        bytes[0] ^= 0x01; // flip a bit in r
        let tampered = Signature::from_bytes(bytes);
        // Either the mangled scalars fail to verify, or (rarely) hit a
        // malformed encoding; both are rejections, never an accept/panic.
        assert!(matches!(
            scheme.verify(&verifying, message, &tampered),
            Err(VerifyError::BadSignature | VerifyError::MalformedSignature),
        ));
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let scheme = EcdsaP256;
        let (signing, _verifying) = EcdsaP256::generate_keypair();
        let (_other_signing, other_verifying) = EcdsaP256::generate_keypair();
        let message = b"signed under the first key";
        let signature = scheme.sign(&signing, message).expect("sign");
        assert_eq!(
            scheme.verify(&other_verifying, message, &signature),
            Err(VerifyError::BadSignature),
        );
    }

    #[test]
    fn verify_rejects_degenerate_and_malformed_signatures() {
        let scheme = EcdsaP256;
        let (_signing, verifying) = EcdsaP256::generate_keypair();
        let message = b"message";

        // r = 0, s = 0.
        let all_zero = Signature::from_bytes(vec![0u8; P1363_LEN]);
        assert_eq!(
            scheme.verify(&verifying, message, &all_zero),
            Err(VerifyError::MalformedSignature),
        );

        // r = 0, s = 1 (valid s bytes, zero r).
        let mut r_zero = vec![0u8; P1363_LEN];
        r_zero[P1363_LEN - 1] = 1;
        assert_eq!(
            scheme.verify(&verifying, message, &Signature::from_bytes(r_zero)),
            Err(VerifyError::MalformedSignature),
        );

        // r = 1, s = 0.
        let mut s_zero = vec![0u8; P1363_LEN];
        s_zero[SCALAR_LEN - 1] = 1;
        assert_eq!(
            scheme.verify(&verifying, message, &Signature::from_bytes(s_zero)),
            Err(VerifyError::MalformedSignature),
        );

        // Wrong length (63 and 65 bytes) and empty.
        for bad_len in [0usize, P1363_LEN - 1, P1363_LEN + 1] {
            assert_eq!(
                scheme.verify(
                    &verifying,
                    message,
                    &Signature::from_bytes(vec![7u8; bad_len])
                ),
                Err(VerifyError::MalformedSignature),
            );
        }
    }

    #[test]
    fn verify_rejects_malformed_spki_key() {
        // A junk SPKI DER blob decodes to no key at all: `verify` reports
        // `MalformedKey` before any curve math, never a panic (crypto-review
        // finding 3). Regression guard against a future `p256`/`spki` upgrade
        // changing the decode-failure path.
        let scheme = EcdsaP256;
        let junk = VerifyingKey::from_spki_der(ALGORITHM, vec![0xde, 0xad, 0xbe, 0xef, 0x00]);
        let signature = Signature::from_bytes(vec![1u8; P1363_LEN]);
        assert_eq!(
            scheme.verify(&junk, b"message", &signature),
            Err(VerifyError::MalformedKey),
        );
    }

    #[test]
    fn verify_rejects_out_of_range_scalars() {
        // r = n or s = n: the group order is not a valid scalar (valid range is
        // [1, n)). The `ecdsa` crate rejects it at signature decode; pin that as
        // `MalformedSignature` so a future upgrade cannot silently start
        // reducing an out-of-range value mod n into an accepted form
        // (crypto-review finding 5).
        let scheme = EcdsaP256;
        let (_signing, verifying) = EcdsaP256::generate_keypair();
        let n = hexdec("FFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551");
        let mut one = vec![0u8; SCALAR_LEN];
        one[SCALAR_LEN - 1] = 1;

        // r = n, s = 1.
        let mut r_is_n = n.clone();
        r_is_n.extend_from_slice(&one);
        assert_eq!(
            scheme.verify(&verifying, b"m", &Signature::from_bytes(r_is_n)),
            Err(VerifyError::MalformedSignature),
        );

        // r = 1, s = n.
        let mut s_is_n = one.clone();
        s_is_n.extend_from_slice(&n);
        assert_eq!(
            scheme.verify(&verifying, b"m", &Signature::from_bytes(s_is_n)),
            Err(VerifyError::MalformedSignature),
        );
    }

    #[test]
    fn signing_key_from_bytes_rejects_degenerate_scalars() {
        // `.unwrap_err()` throughout: `SigningKey` deliberately does not
        // implement `PartialEq` (secret-key equality is a side-channel
        // footgun), so we assert on the error rather than on the `Result`.
        // Zero scalar.
        assert_eq!(
            EcdsaP256::signing_key_from_bytes(&[0u8; SCALAR_LEN]).unwrap_err(),
            KeyRejected::Invalid {
                algorithm: ALGORITHM
            },
        );
        // Scalar == n (the group order) is out of range and rejected.
        let order_n = hexdec("FFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551");
        assert_eq!(
            EcdsaP256::signing_key_from_bytes(&order_n).unwrap_err(),
            KeyRejected::Invalid {
                algorithm: ALGORITHM
            },
        );
        // Wrong lengths.
        for bad_len in [0usize, SCALAR_LEN - 1, SCALAR_LEN + 1] {
            assert_eq!(
                EcdsaP256::signing_key_from_bytes(&vec![1u8; bad_len]).unwrap_err(),
                KeyRejected::WrongLength {
                    expected: SCALAR_LEN,
                    actual: bad_len,
                },
            );
        }
    }

    #[test]
    fn verifying_key_from_sec1_rejects_bad_points() {
        // Not a point at all.
        assert_eq!(
            EcdsaP256::verifying_key_from_sec1(&[0u8; 65]),
            Err(KeyRejected::Invalid {
                algorithm: ALGORITHM
            }),
        );
        // Empty.
        assert_eq!(
            EcdsaP256::verifying_key_from_sec1(&[]),
            Err(KeyRejected::Invalid {
                algorithm: ALGORITHM
            }),
        );
    }

    #[test]
    fn algorithm_mismatch_is_rejected_on_both_paths() {
        let scheme = EcdsaP256;
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let message = b"m";
        let signature = scheme.sign(&signing, message).expect("sign");

        // A verifying key mis-tagged as ML-DSA is rejected before curve math.
        let mistagged =
            VerifyingKey::from_spki_der(SignatureAlgorithm::MlDsa65, verifying.spki_der().to_vec());
        assert_eq!(
            scheme.verify(&mistagged, message, &signature),
            Err(VerifyError::AlgorithmMismatch {
                expected: ALGORITHM,
                actual: SignatureAlgorithm::MlDsa65,
            }),
        );

        // A signing key mis-tagged as ML-DSA is rejected on the sign path.
        let mistagged_signing = crate::signing::SigningKey::from_validated_secret(
            SignatureAlgorithm::MlDsa65,
            vec![1u8; SCALAR_LEN],
        );
        assert_eq!(
            scheme.sign(&mistagged_signing, message),
            Err(SignError::Key(KeyRejected::AlgorithmMismatch {
                expected: ALGORITHM,
                actual: SignatureAlgorithm::MlDsa65,
            })),
        );
    }

    #[test]
    fn hsm_exported_spki_key_verifies_unchanged() {
        // Simulate the HSM path: a public key arrives as SPKI DER (exactly the
        // `cloud_types::PublicKey` representation) and must verify a signature
        // over our domain `Signature` type without any caller change.
        let scheme = EcdsaP256;
        let (signing, verifying) = EcdsaP256::generate_keypair();
        let message = b"checkpoint bytes";
        let signature = scheme.sign(&signing, message).expect("sign");

        let hsm_style_key = VerifyingKey::from_spki_der(ALGORITHM, verifying.spki_der().to_vec());
        scheme
            .verify(&hsm_style_key, message, &signature)
            .expect("HSM-style SPKI key verifies");
    }
}
