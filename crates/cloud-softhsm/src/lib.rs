//! PKCS#11 [`Hsm`](cloud_types::Hsm) backend for `SoftHSM2` — the non-FIPS dev
//! stand-in for `CloudHSM` (spec §9.3, §14, §18.1).
//!
//! [`SoftHsm`] implements the four-method [`Hsm`](cloud_types::Hsm) capability
//! (`generate_key` / `get_public_key` / `sign` / `is_fips_validated`) over the
//! `cryptoki` safe PKCS#11 wrapper, targeting a `SoftHSM2` token provisioned by
//! `scripts/softhsm-init.sh` (or the `deploy/local` docker-compose dev
//! environment). It behaves identically to the in-memory
//! [`cloud_memory::MemoryHsm`] under the shared `cloud-test-suite` Hsm
//! conformance suite, with `expected_fips == false`.
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use cloud_softhsm::{Pkcs11Config, SoftHsm};
//! use cloud_types::{Hsm, KeySpec};
//!
//! # async fn demo() -> Result<(), cloud_types::CloudError> {
//! // Resolve module/token/PIN from the MTC_PKCS11_* environment contract.
//! let hsm: Arc<dyn Hsm> = Arc::new(SoftHsm::from_env()?);
//! let handle = hsm.generate_key(KeySpec::EcdsaP256).await?;
//! let signature = hsm.sign(&handle, b"checkpoint bytes").await?; // 64-byte r‖s
//! assert_eq!(signature.len(), 64);
//! let _spki_der = hsm.get_public_key(&handle).await?;
//! assert!(!hsm.is_fips_validated()); // dev-only, never FIPS (spec §14.4)
//! # Ok(())
//! # }
//! ```
//!
//! # Safety posture
//!
//! This crate contains **no `unsafe` code** and inherits the workspace
//! `unsafe_code = "forbid"` lint: the `unsafe` PKCS#11 FFI lives entirely
//! inside `cryptoki`. It therefore does **not** take the `docs/lint-policy.md`
//! PKCS#11 FFI exception (which is reserved for a hypothetical hand-rolled
//! `pkcs11-sys` crate). See `docs/adr/0006-cloud-softhsm-pkcs-11-hsm-backend-against-softhsm2-via-cryptoki.md`.

#![warn(missing_docs)]

pub mod config;
pub mod hsm;

pub use config::{
    ConfigError, Pkcs11Config, DEFAULT_KEY_LABEL, DEFAULT_MODULE_PATH, DEFAULT_PIN,
    DEFAULT_TOKEN_LABEL, ENV_KEY_LABEL, ENV_MODULE_PATH, ENV_PIN, ENV_TOKEN_LABEL,
};
pub use hsm::SoftHsm;
