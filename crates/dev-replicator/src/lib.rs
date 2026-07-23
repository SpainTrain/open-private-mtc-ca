//! `dev-replicator`: local S3 CRR + `DynamoDB` Global Tables replication
//! simulator with configurable, runtime-adjustable per-link lag (ticket
//! dev-crr-replication-sim, spec §18.3) — the substrate
//! `dev-multiregion-harness` builds the three-region topology on.
//!
//! One process instance is one directed **link**: it replicates an S3
//! bucket, a `DynamoDB` table, or both, from a source `LocalStack` endpoint to a
//! target `LocalStack` endpoint. Arbitrary directed-link topologies (the
//! ticket's AC) are built by running one instance per edge of the desired
//! topology graph — see [`config::LinkConfig`] for the env-var contract and
//! `deploy/local/docker-compose.replication-sim.yml` for a two-region
//! example.
//!
//! # Module map
//!
//! - [`lag`] — pure, infra-free lag scheduling and idempotent dedup. Both
//!   pollers below are thin IO adapters around one of these; the ordering,
//!   dedup, and lag-timing logic is written and tested exactly once here.
//! - [`s3`] — the S3 CRR simulator (`ListObjectVersions` polling).
//! - [`ddb`] — the `DynamoDB` Global Tables simulator (scan-diff + LWW).
//! - [`control`] — the local HTTP control endpoint: runtime lag changes
//!   (including stall/infinite lag), pause/resume (the partition hook), and
//!   `/status` for observability.
//! - [`link`] — orchestrates one link: owns the pollers, the shared
//!   lag/pause controls, and the poll loop.
//! - [`config`] — environment-variable configuration for one link.
//!
//! # Why this crate talks to the AWS SDK directly
//!
//! Rule `no-sdk-types-in-domain` (spec §22.8) forbids vendor SDK types in
//! *domain* trait signatures — the four cloud abstraction traits
//! (`cloud-types`) that the CA service is built against. This crate does not
//! implement, and is not consumed through, any of those traits: it is a
//! standalone dev-environment tool that simulates what CRR/Global Tables do
//! *between two `LocalStack` containers*, entirely outside the CA service's
//! own storage path. Using `aws-sdk-s3`/`aws-sdk-dynamodb` directly here
//! (the pattern every backend implementation crate will eventually use
//! *inside* its own translation layer) is the correct-scoped choice, not a
//! rule violation — see the crate's `Cargo.toml` header.
//!
//! # Runtime control (mr-replication-sim AC)
//!
//! Per-link lag is adjustable at runtime via the control endpoint, including
//! **stall** (infinite lag, for `chaos-crr-stall`): see [`lag::LagPolicy`]
//! and [`control`]. Pause/resume (a full discovery+apply halt — the
//! partition-simulation hook `dev-partition-failover-scenarios` will use) is
//! separate from lag and documented in [`link`].
//!
//! # Conflict semantics (mr-replication-sim AC)
//!
//! `DynamoDB` replication resolves conflicting writes with **last-writer-wins**
//! — see the [`ddb`] module docs for the exact mechanism (a hidden
//! timestamp attribute plus a conditional write).

pub mod config;
pub mod control;
pub mod ddb;
mod error;
pub mod lag;
pub mod link;
pub mod s3;

pub use error::ReplicatorError;

/// Outcome of one `apply_due` pass, shared by the S3 and `DynamoDB` pollers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ApplySummary {
    /// Changes successfully written to the target.
    pub applied: usize,
    /// Changes rejected by `DynamoDB`'s LWW condition (stale, not an error —
    /// always `0` for the S3 poller, which has no conflict concept).
    pub stale: usize,
    /// Changes that failed for a real reason (logged via `tracing::error!`
    /// at the call site).
    pub failed: usize,
}

impl ApplySummary {
    /// Total items this pass attempted to apply.
    #[must_use]
    pub const fn attempted(&self) -> usize {
        self.applied + self.stale + self.failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempted_sums_all_three_outcomes() {
        let s = ApplySummary {
            applied: 2,
            stale: 1,
            failed: 3,
        };
        assert_eq!(s.attempted(), 6);
    }
}
