//! Primary-region lease and epoch-fencing protocol (spec §8.2/§8.3, §11).
//!
//! This crate implements the coordination protocol that lets exactly one
//! region act as the log's primary at a time, built entirely on the
//! cloud-agnostic [`ReplicatedKv`](cloud_types::ReplicatedKv) trait — never on
//! a provider SDK (rule no-sdk-types-in-domain, §22.8). It is the critical-path
//! anchor for all multi-region work (§25.5).
//!
//! # The protocol in one paragraph
//!
//! The primary holds a lease item ([`LeaseCoordinator`]) that it
//! [`renew`](LeaseCoordinator::renew)s every [`RENEWAL_INTERVAL`] with a
//! [`LEASE_TTL`] expiry (spec §8.3: 20s / 60s). A standby that observes the
//! lease expired past the [`TAKEOVER_SAFETY_MARGIN`] may
//! [`claim_lease`](LeaseCoordinator::claim_lease), which **atomically advances
//! the fencing [`Epoch`]** in one conditional write. Because every takeover
//! bumps the epoch and every coordination write carries [`epoch_condition`],
//! a primary demoted by a takeover can no longer mutate log state — its writes
//! fail the epoch condition (spec §8.3, §11).
//!
//! # Fencing epoch is shared, not re-invented
//!
//! The [`Epoch`] here is re-exported from the `mtc` core crate: the lease's
//! epoch, the index counter's epoch, and the checkpoint/batch epochs are one
//! and the same write-path fencing token (spec §11), so they are one
//! compile-time type. [`EpochExt`] adds the checked, strictly-monotonic
//! successor operation the protocol needs.
//!
//! # Not in this crate
//!
//! The background renewal task, wiring the epoch condition into the CA write
//! path, and the full Kani invariant proofs are separate beads; this crate
//! ships the protocol primitives, a reusable conformance suite
//! ([`run_lease_suite`]), and a basic no-panic Kani harness.

use std::time::Duration;

mod errors;
mod ids;
mod protocol;
mod suite;

pub use errors::LeaseError;
pub use ids::{EpochExt, EpochOverflow, HolderId, Region, INITIAL_EPOCH};
pub use protocol::{epoch_condition, Lease, LeaseCoordinator};
pub use suite::run_lease_suite;

// The fencing epoch and the log identifier are core domain types owned by the
// `mtc` crate (spec §22.1); re-exported so callers can name them as
// `coordination::Epoch` / `coordination::LogId` without a direct `mtc` import.
pub use mtc::{Epoch, LogId};

/// How often the primary renews its lease (spec §8.3: "Renewed every 20s").
pub const RENEWAL_INTERVAL: Duration = Duration::from_secs(20);

/// Lease time-to-live (spec §8.3: "60s TTL" — one minute).
///
/// A renewal sets `expires_at` to one `LEASE_TTL` ahead of now; three renewal
/// intervals fit within one TTL, tolerating two consecutive missed renewals
/// before expiry.
pub const LEASE_TTL: Duration = Duration::from_mins(1);

/// Extra buffer past `expires_at` a challenger waits before a lease becomes
/// takeover-eligible (spec §8.3: "Expiry beyond 60s safety margin makes lease
/// takeover-eligible").
///
/// A challenger may only [`claim_lease`](LeaseCoordinator::claim_lease) once
/// the clock has passed `expires_at` by at least this margin, so a demoted
/// primary has provably stopped renewing (and, absorbing clock skew up to the
/// margin, provably considers its own lease expired) before anyone takes over.
/// This favors safety over availability, as a CA must (a split-brain corrupts
/// the log irreparably). See the crate README/report for the interpretation of
/// the §8.3 wording adopted here. Equal to [`LEASE_TTL`] (60s) but conceptually
/// distinct: the TTL is how long a lease lives, the margin is the extra buffer
/// a challenger waits after expiry.
pub const TAKEOVER_SAFETY_MARGIN: Duration = Duration::from_mins(1);

// Basic no-panic Kani harness (rule kani-for-critical-paths). Compiled only
// under `cargo kani` (`--cfg kani`), so it never affects normal builds/tests.
// The full lease/epoch invariant proofs are a separate bead (mtc-8l0u).
#[cfg(kani)]
#[path = "../proofs/lease_epoch.rs"]
mod proofs;
