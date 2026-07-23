//! ACME server core (RFC 8555): directory, anti-replay nonces, accounts,
//! and ES256 JWS verification.
//!
//! This crate is the HTTP face of the **native ACME adapter** — the first
//! `EntryIntake` adapter of the issuance pipeline (spec §10; write path
//! §11). Scope is deliberately the RFC 8555 foundation only:
//!
//! - `GET /acme/directory` (§7.1.1) — lists implemented resources
//! - `HEAD`/`GET /acme/new-nonce` (§7.2) — anti-replay nonces, single-use,
//!   fresh `Replay-Nonce` on every response
//! - `POST /acme/new-account` (§7.3) — ES256 JWS-authenticated, accounts
//!   keyed by RFC 7638 JWK thumbprint
//!
//! Orders/authorizations/challenges (`ca-acme-orders`), finalize/issuance
//! plus `EntryIntake` wiring (`ca-acme-issuance`), and external account
//! binding (admin-surface epic) are separate tickets.
//!
//! Failures surface as ACME problem documents
//! (`application/problem+json`, §6.7): `badNonce`,
//! `badSignatureAlgorithm`, `malformed`, `unauthorized`,
//! `accountDoesNotExist`, `badPublicKey`.
//!
//! Time is injected via the local [`clock::Clock`] seam (to be replaced by
//! the shared `crates/clock` once ticket `ca-clock` lands). JWS parsing is
//! fuzzed (`fuzz/`, spec §19.3) and property-tested to never panic on
//! arbitrary bytes.

pub mod account;
pub mod client;
pub mod clock;
pub mod error;
pub mod jws;
pub mod nonce;
pub mod problem;
pub mod routes;

pub use account::{Account, AccountId, AccountStatus, AccountStore, NewAccountRequest};
pub use clock::{Clock, ManualClock, MonotonicClock, MonotonicMillis};
pub use error::{AcmeError, ES256};
pub use jws::{AccountKeySource, Jwk, JwkThumbprint, Jws, ProtectedHeader};
pub use nonce::{Nonce, NonceStore, DEFAULT_NONCE_TTL_MILLIS};
pub use problem::{ProblemDocument, ProblemType};
pub use routes::{
    router, AcmeState, BaseUrl, ACCOUNT_PATH_PREFIX, DIRECTORY_PATH, NEW_ACCOUNT_PATH,
    NEW_NONCE_PATH,
};
