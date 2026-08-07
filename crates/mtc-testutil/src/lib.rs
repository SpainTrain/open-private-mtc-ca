//! Shared test-support toolkit for the MTC-CA workspace (Testing
//! Infrastructure epic, ticket `mtc-1cw`; spec §19.2, §19.3 Layer 1).
//!
//! This crate is a **dev-dependency toolkit**: component epics' test suites
//! depend on it instead of re-inventing seeded randomness, fixture
//! construction, or proptest strategies. It depends on [`mtc`] read-only —
//! nothing here adds behavior to the domain types themselves. In particular,
//! per-type `Arbitrary` impls are intended to live *with* each spec type
//! inside `crates/mtc` behind a `testing` feature flag (a separate ticket
//! owns that); this crate provides the strategy-generation logic those future
//! impls will wrap, plus the documented convention — see the [`strategy`]
//! module docs.
//!
//! # Modules
//!
//! - [`rng`] — [`rng::seeded_rng`]: deterministic pseudo-randomness for
//!   building one concrete fixture value outside a `proptest!` body (the same
//!   seed always produces the same sequence).
//! - [`mod@env`] — [`env::temp_dir`] and [`env::EnvVarGuard`]: filesystem and
//!   process-environment helpers for tests that need either, cleaned up
//!   automatically on drop.
//! - [`strategy`] — shared `proptest` `Strategy` helpers (`arb_hash_output`,
//!   `arb_log_id`, `arb_claim`, `arb_certificate_entry`, ...), the
//!   `Arbitrary` convention, the `PROPTEST_CASES` extended-iteration
//!   mechanism (spec §19.3 Layer 1), and the `proptest-regressions/`
//!   checked-in-fixture convention.
//! - [`fixtures`] — value builders for checkpoints, log entries, and
//!   tree-leaf sequences, driven by [`rng::seeded_rng`] rather than
//!   `proptest`'s `Strategy` machinery (for tests, benchmarks, and demos that
//!   want one concrete value rather than a generator).
//!
//! # Lint posture (spec §22.12)
//!
//! Same baseline as every workspace crate: `unsafe_code` forbidden,
//! `missing_docs` denied, `clippy::unwrap_used` / `clippy::expect_used`
//! denied outside `#[cfg(test)]` (rule `no-unwrap-in-prod`). This crate's own
//! source is ordinary library code — not itself gated by `#[cfg(test)]`, even
//! though every consumer reaches it from a `[dev-dependencies]` edge — so its
//! fallible builders propagate errors rather than unwrap; see
//! [`fixtures::FixtureError`].

#![deny(missing_docs)]
#![warn(clippy::pedantic, clippy::nursery, clippy::cargo)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
// `rand` (pinned for `rng::seeded_rng`/`StdRng`) and `proptest`'s own internal
// RNG chain resolve to different `rand`/`rand_core`/`getrandom` majors, and
// the trybuild-style dev-tooling graph adds a duplicate `syn` — version
// duplication here is the integrator's concern, not this library's API
// (mirrors `mtc`'s identical allow for its `trybuild` dev-dependency tree).
#![allow(clippy::multiple_crate_versions)]

pub mod env;
pub mod fixtures;
pub mod rng;
pub mod strategy;

pub use fixtures::FixtureError;
