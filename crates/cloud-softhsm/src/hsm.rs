//! [`SoftHsm`] — a PKCS#11 [`Hsm`] backed by `SoftHSM2` (spec §9.3, §14, §18.1).
//!
//! **Dev-only by definition.** `SoftHSM2` is a software token that emulates a
//! PKCS#11 HSM; it is explicitly *not* FIPS-validated, so
//! [`Hsm::is_fips_validated`] always returns `false` (spec §14.4,
//! `.claude/rules/fips-boundary-preserved`). It is the local stand-in for
//! `CloudHSM`: the same PKCS#11 code path serves the future `CloudHSM` backend,
//! which inherits FIPS validation from the hardware (spec §14.2).
//!
//! # Key material never crosses the boundary
//!
//! Private keys are generated inside the token as non-extractable
//! (`CKA_SENSITIVE = true`, `CKA_EXTRACTABLE = false`) objects. This crate only
//! ever exports public keys and signatures — no `C_WrapKey`, no private-key
//! attribute reads — upholding the fips-boundary rule that no private key
//! material leaves the HSM.
//!
//! # Signature encoding (P1363, 64-byte r‖s)
//!
//! Signing uses `CKM_ECDSA` over an in-Rust SHA-256 digest of the message. The
//! PKCS#11 `CKM_ECDSA` mechanism emits the raw fixed-width `r || s`
//! (IEEE P1363) encoding — 32-byte big-endian `r` followed by 32-byte
//! big-endian `s`, 64 bytes total for P-256 — which is exactly the encoding the
//! [`Hsm::sign`] contract mandates and the shared conformance suite verifies
//! with `RustCrypto` `p256` against the exported SPKI public key (see
//! `docs/adr/0005-softhsm-pkcs11-hsm-backend.md` and
//! `docs/adr/0003` for the repository-wide P1363 / high-`s` decision).
//!
//! # Concurrency
//!
//! A PKCS#11 [`Session`] is not `Sync` and must not be shared across threads
//! (PKCS#11 §5.6.7). Each operation therefore opens its own short-lived session
//! on the (thread-safe, `CKF_OS_LOCKING_OK`) library context and runs on a
//! blocking thread via [`tokio::task::spawn_blocking`], so many tasks sign in
//! parallel without sharing a session and without blocking the async runtime.

use async_trait::async_trait;
use cloud_types::{CloudError, Hsm, KeyHandle, KeySpec, PublicKey};
use cryptoki::context::{CInitializeArgs, Pkcs11};
use cryptoki::error::{Error as Pkcs11Error, RvError};
use cryptoki::mechanism::Mechanism;
use cryptoki::object::{Attribute, AttributeType, ObjectClass, ObjectHandle};
use cryptoki::session::{Session, UserType};
use cryptoki::slot::Slot;
use cryptoki::types::AuthPin;
use p256::pkcs8::EncodePublicKey;
use sha2::{Digest, Sha256};

use crate::config::Pkcs11Config;

/// DER encoding of the `prime256v1` (NIST P-256 / `secp256r1`) named-curve OID
/// `1.2.840.10045.3.1.7`, used as `CKA_EC_PARAMS` when generating a key
/// (spec §14.1 v1 algorithm).
const P256_EC_PARAMS_DER: [u8; 10] = [0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];

/// Number of random bytes used for a generated key's `CKA_ID` and label suffix.
const KEY_ID_LEN: u32 = 16;

/// Expected byte length of a raw P-256 ECDSA signature (`r || s`, IEEE P1363).
const P256_SIGNATURE_LEN: usize = 64;

/// A PKCS#11 [`Hsm`] implementation talking to a `SoftHSM2` token (spec §14).
///
/// Construct one with [`SoftHsm::connect`] (or [`SoftHsm::from_env`]); it is
/// then shared as `Arc<dyn Hsm>` from the `Backend` factory (spec §9.4) and
/// signs concurrently from many tasks.
#[derive(Debug)]
pub struct SoftHsm {
    // Cloneable, `Send + Sync` library context (holds an `Arc` internally);
    // cloned into each blocking task so per-operation sessions share one
    // initialized module.
    pkcs11: Pkcs11,
    slot: Slot,
    config: Pkcs11Config,
}

impl SoftHsm {
    /// Loads the PKCS#11 module, initializes it, and locates the configured
    /// token, validating the PIN by opening and logging in to a throwaway
    /// session so misconfiguration fails fast at construction.
    ///
    /// # Errors
    ///
    /// - [`CloudError::NotFound`] if no token carries the configured label.
    /// - [`CloudError::Transport`] for module-load, initialization, or login
    ///   failures (`retryable` set per the underlying PKCS#11 return code).
    pub fn connect(config: Pkcs11Config) -> Result<Self, CloudError> {
        let pkcs11 = Pkcs11::new(config.module_path()).map_err(|error| CloudError::Transport {
            retryable: false,
            reason: format!("loading PKCS#11 module {}: {error}", config.module_path()),
        })?;

        // Tolerate a library that another context in this process already
        // initialized (PKCS#11 `C_Initialize` is process-global per module).
        match pkcs11.initialize(CInitializeArgs::OsThreads) {
            Ok(())
            | Err(
                Pkcs11Error::AlreadyInitialized
                | Pkcs11Error::Pkcs11(RvError::CryptokiAlreadyInitialized, _),
            ) => {}
            Err(error) => return Err(map_pkcs11("initializing PKCS#11 library", &error)),
        }

        let slot = find_slot(&pkcs11, config.token_label())?;

        // Eager validation: open + login + close, so a wrong PIN or absent
        // token surfaces here rather than on the first sign.
        let session = open_session(&pkcs11, slot, config.pin())?;
        drop(session);

        Ok(Self {
            pkcs11,
            slot,
            config,
        })
    }

    /// Convenience constructor resolving [`Pkcs11Config::from_env`] then
    /// [`connect`](Self::connect).
    ///
    /// # Errors
    ///
    /// [`CloudError::Transport`] (non-retryable) if the environment config is
    /// invalid, plus every error [`connect`](Self::connect) can return.
    pub fn from_env() -> Result<Self, CloudError> {
        let config = Pkcs11Config::from_env().map_err(|error| CloudError::Transport {
            retryable: false,
            reason: format!("resolving PKCS#11 config from environment: {error}"),
        })?;
        Self::connect(config)
    }
}

#[async_trait]
impl Hsm for SoftHsm {
    async fn sign(&self, key_handle: &KeyHandle, data: &[u8]) -> Result<Vec<u8>, CloudError> {
        let pkcs11 = self.pkcs11.clone();
        let slot = self.slot;
        let pin = self.config.pin().to_string();
        let label = key_handle.as_str().to_string();
        let data = data.to_vec();

        let handle = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, CloudError> {
            let session = open_session(&pkcs11, slot, &pin)?;
            let private_key = find_key(&session, &label, ObjectClass::PRIVATE_KEY)?
                .ok_or_else(|| CloudError::NotFound { key: label.clone() })?;

            // CKM_ECDSA signs a pre-computed digest; the trait contract is that
            // the backend digests with SHA-256 for KeySpec::EcdsaP256.
            let digest = Sha256::digest(&data);
            let signature = session
                .sign(&Mechanism::Ecdsa, private_key, digest.as_slice())
                .map_err(|error| map_pkcs11("CKM_ECDSA sign", &error))?;

            // CKM_ECDSA returns fixed-width r||s (IEEE P1363); 64 bytes for
            // P-256. Anything else means a non-P-256 key or a broken token.
            if signature.len() != P256_SIGNATURE_LEN {
                return Err(CloudError::Transport {
                    retryable: false,
                    reason: format!(
                        "expected {P256_SIGNATURE_LEN}-byte P1363 signature, got {}",
                        signature.len()
                    ),
                });
            }
            Ok(signature)
        });

        handle.await.map_err(|error| CloudError::Transport {
            retryable: false,
            reason: format!("signing task failed to join: {error}"),
        })?
    }

    async fn get_public_key(&self, key_handle: &KeyHandle) -> Result<PublicKey, CloudError> {
        let pkcs11 = self.pkcs11.clone();
        let slot = self.slot;
        let pin = self.config.pin().to_string();
        let label = key_handle.as_str().to_string();

        let handle = tokio::task::spawn_blocking(move || -> Result<PublicKey, CloudError> {
            let session = open_session(&pkcs11, slot, &pin)?;
            let public_key = find_key(&session, &label, ObjectClass::PUBLIC_KEY)?
                .ok_or_else(|| CloudError::NotFound { key: label.clone() })?;

            let attributes = session
                .get_attributes(public_key, &[AttributeType::EcPoint])
                .map_err(|error| map_pkcs11("reading CKA_EC_POINT", &error))?;
            let ec_point = attributes
                .into_iter()
                .find_map(|attribute| match attribute {
                    Attribute::EcPoint(bytes) => Some(bytes),
                    _ => None,
                })
                .ok_or_else(|| CloudError::Transport {
                    retryable: false,
                    reason: format!("public key {label} exposes no CKA_EC_POINT"),
                })?;

            spki_der_from_ec_point(&ec_point)
        });

        handle.await.map_err(|error| CloudError::Transport {
            retryable: false,
            reason: format!("get-public-key task failed to join: {error}"),
        })?
    }

    async fn generate_key(&self, spec: KeySpec) -> Result<KeyHandle, CloudError> {
        // Exhaustive on purpose: a new KeySpec variant (ML-DSA in v2) is a
        // deliberate breaking change every backend must then handle (spec §22.3).
        match spec {
            KeySpec::EcdsaP256 => {}
        }

        let pkcs11 = self.pkcs11.clone();
        let slot = self.slot;
        let pin = self.config.pin().to_string();

        let handle = tokio::task::spawn_blocking(move || -> Result<KeyHandle, CloudError> {
            let session = open_session(&pkcs11, slot, &pin)?;

            // Draw a unique key id from the token's RNG; label is its hex.
            let id = session
                .generate_random_vec(KEY_ID_LEN)
                .map_err(|error| map_pkcs11("generating key id", &error))?;
            let label = format!("mtc-key-{}", to_hex(&id));

            let public_template = vec![
                Attribute::Token(true),
                Attribute::Private(false),
                Attribute::Verify(true),
                Attribute::EcParams(P256_EC_PARAMS_DER.to_vec()),
                Attribute::Label(label.clone().into_bytes()),
                Attribute::Id(id.clone()),
            ];
            let private_template = vec![
                Attribute::Token(true),
                Attribute::Private(true),
                // Non-exportable: the private scalar never leaves the token
                // (fips-boundary-preserved).
                Attribute::Sensitive(true),
                Attribute::Extractable(false),
                Attribute::Sign(true),
                Attribute::Label(label.clone().into_bytes()),
                Attribute::Id(id),
            ];

            session
                .generate_key_pair(
                    &Mechanism::EccKeyPairGen,
                    &public_template,
                    &private_template,
                )
                .map_err(|error| map_pkcs11("CKM_EC_KEY_PAIR_GEN", &error))?;

            Ok(KeyHandle::new(label))
        });

        handle.await.map_err(|error| CloudError::Transport {
            retryable: false,
            reason: format!("generate-key task failed to join: {error}"),
        })?
    }

    fn is_fips_validated(&self) -> bool {
        // `SoftHSM2` is a software token — never FIPS-validated (spec §14.4).
        false
    }
}

/// Locates the slot whose token carries `token_label`.
///
/// The PKCS#11 label field is a fixed-width, space-padded array, so trailing
/// whitespace is ignored on both sides of the comparison.
fn find_slot(pkcs11: &Pkcs11, token_label: &str) -> Result<Slot, CloudError> {
    let slots = pkcs11
        .get_slots_with_token()
        .map_err(|error| map_pkcs11("listing token slots", &error))?;
    for slot in slots {
        let info = pkcs11
            .get_token_info(slot)
            .map_err(|error| map_pkcs11("reading token info", &error))?;
        if info.label().trim_end() == token_label.trim_end() {
            return Ok(slot);
        }
    }
    Err(CloudError::NotFound {
        key: format!("token '{token_label}'"),
    })
}

/// Opens an RW session on `slot` and logs in as the normal user.
///
/// Login state is shared across an application's sessions, so a concurrent
/// login that finds the user already logged in is treated as success.
fn open_session(pkcs11: &Pkcs11, slot: Slot, pin: &str) -> Result<Session, CloudError> {
    let session = pkcs11
        .open_rw_session(slot)
        .map_err(|error| map_pkcs11("opening PKCS#11 session", &error))?;
    let auth_pin = AuthPin::new(pin.to_string());
    match session.login(UserType::User, Some(&auth_pin)) {
        Ok(()) | Err(Pkcs11Error::Pkcs11(RvError::UserAlreadyLoggedIn, _)) => Ok(session),
        Err(error) => Err(map_pkcs11("logging in to token", &error)),
    }
}

/// Finds the single token object with `label` of the given `class`, returning
/// `None` when no such object exists (the caller maps that to
/// [`CloudError::NotFound`]).
fn find_key(
    session: &Session,
    label: &str,
    class: ObjectClass,
) -> Result<Option<ObjectHandle>, CloudError> {
    let matches = session
        .find_objects(&[
            Attribute::Class(class),
            Attribute::Label(label.as_bytes().to_vec()),
        ])
        .map_err(|error| map_pkcs11("finding key object", &error))?;
    Ok(matches.into_iter().next())
}

/// Builds SPKI DER (`SubjectPublicKeyInfo`) from a `CKA_EC_POINT` attribute
/// value by decoding the SEC1 point and re-encoding via `RustCrypto` `p256`,
/// guaranteeing the export round-trips with the same verifier the conformance
/// suite uses.
fn spki_der_from_ec_point(ec_point: &[u8]) -> Result<PublicKey, CloudError> {
    let point = sec1_point(ec_point)?;
    let verifying_key =
        p256::PublicKey::from_sec1_bytes(point).map_err(|error| CloudError::Transport {
            retryable: false,
            reason: format!("CKA_EC_POINT is not a valid P-256 point: {error}"),
        })?;
    let der = verifying_key
        .to_public_key_der()
        .map_err(|error| CloudError::Transport {
            retryable: false,
            reason: format!("encoding SPKI DER: {error}"),
        })?;
    Ok(PublicKey::from_spki_der(der.as_bytes().to_vec()))
}

/// Extracts the ANSI X9.62 uncompressed point from a `CKA_EC_POINT` value.
///
/// PKCS#11 defines `CKA_EC_POINT` as the DER encoding of the point, i.e. an
/// `OCTET STRING` wrapping the SEC1 bytes (`SoftHSM2`'s form). Some tokens omit
/// the wrapper and return the raw point. For P-256 the two are unambiguous by
/// length: a raw uncompressed point is 65 bytes (`0x04 ‖ X ‖ Y`), and the
/// wrapped form is 67 bytes (`0x04 0x41` ‖ point).
fn sec1_point(ec_point: &[u8]) -> Result<&[u8], CloudError> {
    match ec_point {
        [0x04, rest @ ..] if rest.len() == 64 => Ok(ec_point),
        [0x04, 0x41, inner @ ..] if inner.len() == 65 && inner[0] == 0x04 => Ok(inner),
        other => Err(CloudError::Transport {
            retryable: false,
            reason: format!("unexpected CKA_EC_POINT encoding: {} bytes", other.len()),
        }),
    }
}

/// Lowercase hex of `bytes` (no dependency on the `hex` crate).
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, byte| {
            // Writing a byte to a String is infallible.
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

/// Classifies a PKCS#11 return code as a transient (retryable) fault.
const fn rverror_is_retryable(rv: RvError) -> bool {
    matches!(
        rv,
        RvError::DeviceError
            | RvError::DeviceMemory
            | RvError::DeviceRemoved
            | RvError::FunctionFailed
            | RvError::GeneralError
            | RvError::HostMemory
    )
}

/// Maps a `cryptoki` error onto the [`CloudError`] taxonomy, tagging transient
/// device faults retryable and everything else terminal (spec §9.3, rule
/// `no-sdk-types-in-domain` — the `CKR_*` code never leaks outward).
fn map_pkcs11(operation: &str, error: &Pkcs11Error) -> CloudError {
    let retryable = matches!(error, Pkcs11Error::Pkcs11(rv, _) if rverror_is_retryable(*rv));
    CloudError::Transport {
        retryable,
        reason: format!("{operation}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use cryptoki::context::Function;
    use p256::pkcs8::DecodePublicKey as _;
    use pretty_assertions::assert_eq;

    use super::*;

    // NIST P-256 generator point G (uncompressed SEC1: 0x04 ‖ Gx ‖ Gy), a
    // known on-curve point used to exercise the CKA_EC_POINT decoders.
    const P256_GENERATOR_SEC1: [u8; 65] = [
        0x04, 0x6B, 0x17, 0xD1, 0xF2, 0xE1, 0x2C, 0x42, 0x47, 0xF8, 0xBC, 0xE6, 0xE5, 0x63, 0xA4,
        0x40, 0xF2, 0x77, 0x03, 0x7D, 0x81, 0x2D, 0xEB, 0x33, 0xA0, 0xF4, 0xA1, 0x39, 0x45, 0xD8,
        0x98, 0xC2, 0x96, 0x4F, 0xE3, 0x42, 0xE2, 0xFE, 0x1A, 0x7F, 0x9B, 0x8E, 0xE7, 0xEB, 0x4A,
        0x7C, 0x0F, 0x9E, 0x16, 0x2B, 0xCE, 0x33, 0x57, 0x6B, 0x31, 0x5E, 0xCE, 0xCB, 0xB6, 0x40,
        0x68, 0x37, 0xBF, 0x51, 0xF5,
    ];

    #[test]
    fn sec1_point_accepts_raw_uncompressed_point() {
        let decoded = sec1_point(&P256_GENERATOR_SEC1).unwrap();
        assert_eq!(decoded, &P256_GENERATOR_SEC1);
    }

    #[test]
    fn sec1_point_unwraps_der_octet_string() {
        // `SoftHSM2` form: OCTET STRING (0x04 len=0x41) wrapping the 65-byte point.
        let mut wrapped = vec![0x04, 0x41];
        wrapped.extend_from_slice(&P256_GENERATOR_SEC1);
        let decoded = sec1_point(&wrapped).unwrap();
        assert_eq!(decoded, &P256_GENERATOR_SEC1);
    }

    #[test]
    fn sec1_point_wrapped_form_round_trips_to_valid_spki() {
        // The full get_public_key decode path against the wrapped generator.
        let mut wrapped = vec![0x04, 0x41];
        wrapped.extend_from_slice(&P256_GENERATOR_SEC1);
        let public_key = spki_der_from_ec_point(&wrapped).unwrap();
        // Parses back as a standard SPKI public key (`RustCrypto` verifier input).
        p256::PublicKey::from_public_key_der(public_key.spki_der())
            .expect("exported SPKI DER must round-trip");
    }

    #[test]
    fn sec1_point_rejects_wrong_length() {
        let err = sec1_point(&[0x04, 0x00, 0x01]).unwrap_err();
        assert!(matches!(
            err,
            CloudError::Transport {
                retryable: false,
                ..
            }
        ));
    }

    #[test]
    fn to_hex_is_lowercase_fixed_width() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
        assert_eq!(to_hex(&[]), "");
    }

    #[test]
    fn device_faults_are_retryable_other_faults_are_not() {
        assert!(rverror_is_retryable(RvError::DeviceError));
        assert!(rverror_is_retryable(RvError::FunctionFailed));
        assert!(!rverror_is_retryable(RvError::PinIncorrect));
        assert!(!rverror_is_retryable(RvError::KeyHandleInvalid));
    }

    #[test]
    fn map_pkcs11_classifies_retryability_and_hides_the_ckr_code_in_transport() {
        let transient = map_pkcs11(
            "sign",
            &Pkcs11Error::Pkcs11(RvError::DeviceError, Function::Sign),
        );
        assert!(matches!(
            transient,
            CloudError::Transport {
                retryable: true,
                ..
            }
        ));

        let terminal = map_pkcs11(
            "login",
            &Pkcs11Error::Pkcs11(RvError::PinIncorrect, Function::Login),
        );
        assert!(matches!(
            terminal,
            CloudError::Transport {
                retryable: false,
                ..
            }
        ));
    }
}
