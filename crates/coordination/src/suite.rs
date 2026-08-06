//! A reusable, backend-agnostic conformance suite for the lease/epoch protocol
//! (spec §9.7 shared-suite pattern).
//!
//! [`run_lease_suite`] drives one full lifecycle — bootstrap, acquire, renew,
//! blocked takeover (within the safety margin), successful takeover with an
//! epoch bump, and fencing of the demoted primary — against **any**
//! [`ReplicatedKv`] backend. It runs against `cloud-memory` today
//! (`tests/lease_suite.rs`); the same call will run against the cloud-aws
//! `DynamoDB` backend once it lands (bead mtc-lf7), proving both backends uphold
//! one behavioral contract (spec §9.7).
//!
//! ## Why the clock is a concrete [`FakeClock`]
//!
//! Exercising expiry and takeover requires *advancing time deterministically*
//! — a [`FakeClock`] capability that is deliberately **not** on the injected
//! [`Clock`](clock::Clock) trait (production time only moves forward on its
//! own). The suite therefore takes an `Arc<FakeClock>` so it can fast-forward,
//! and injects the very same clock into the coordinators under test as
//! `Arc<dyn Clock>`. This keeps the suite identical across memory and
//! `DynamoDB`: the KV backend never observes time, so only the storage argument
//! changes.

use std::sync::Arc;
use std::time::Duration;

use clock::{Clock, FakeClock};
use cloud_types::ReplicatedKv;
use mtc::{Epoch, LogId};

use crate::errors::LeaseError;
use crate::ids::{EpochExt, HolderId, Region, INITIAL_EPOCH};
use crate::protocol::{Lease, LeaseCoordinator};
use crate::{LEASE_TTL, TAKEOVER_SAFETY_MARGIN};

/// Extracts the [`Lease`] from an operation expected to succeed, panicking with
/// context otherwise (this is a test-harness assertion helper).
fn expect_lease(result: Result<Lease, LeaseError>, context: &str) -> Lease {
    match result {
        Ok(lease) => lease,
        Err(err) => panic!("{context}: expected a lease, got {err:?}"),
    }
}

/// Runs the full lease/epoch conformance scenario against `kv`, driving time
/// via `clock` (spec §8.2/§8.3).
///
/// Two regions share one log's lease: `us-east-1` (holder `inst-a`) acquires
/// and renews; `us-west-2` (holder `inst-b`) is blocked from taking over until
/// the incumbent has been expired past the [`TAKEOVER_SAFETY_MARGIN`], then
/// takes over with the epoch advanced, after which the demoted primary is
/// fenced out of renewing.
///
/// Call it from an async test with a fresh, empty `kv`. Construct `clock` at a
/// whole-second instant (e.g. [`FakeClock::default`]) so the millisecond
/// resolution of `expires_at` does not perturb the ordering assertions.
///
/// # Panics
///
/// Panics on the first deviation from the protocol contract — it is a
/// conformance suite, meant to run under a test harness where a panic is a
/// test failure.
#[allow(clippy::too_many_lines)] // one linear end-to-end story; splitting it would obscure the sequence
pub async fn run_lease_suite(kv: Arc<dyn ReplicatedKv>, clock: Arc<FakeClock>) {
    let Ok(log_id) = LogId::new("lease-suite-log") else {
        panic!("the suite's log id literal is non-empty");
    };
    let dyn_clock: Arc<dyn Clock> = clock.clone();

    let region_a = Region::new("us-east-1");
    let region_b = Region::new("us-west-2");
    let coord_a = LeaseCoordinator::new(
        Arc::clone(&kv),
        Arc::clone(&dyn_clock),
        &log_id,
        HolderId::new("us-east-1/inst-a"),
        region_a.clone(),
    );
    let coord_b = LeaseCoordinator::new(
        Arc::clone(&kv),
        dyn_clock,
        &log_id,
        HolderId::new("us-west-2/inst-b"),
        region_b.clone(),
    );

    // --- 1. Bootstrap: no lease exists yet. ---------------------------------
    assert!(
        matches!(coord_a.read_lease().await, Err(LeaseError::NoLease { .. })),
        "empty backend must report NoLease"
    );
    assert!(
        matches!(coord_b.claim_lease().await, Err(LeaseError::NoLease { .. })),
        "claim with no lease to take over must report NoLease"
    );

    // --- 2. A acquires from the unheld state. -------------------------------
    let t_acquire = clock.now();
    let l1 = expect_lease(coord_a.acquire().await, "A first acquire");
    assert_eq!(
        l1.epoch, INITIAL_EPOCH,
        "first acquire records INITIAL_EPOCH"
    );
    assert_eq!(l1.holder_id, *coord_a.holder_id());
    assert_eq!(l1.region, region_a);
    assert!(l1.expires_at > t_acquire, "expiry is in the future");
    assert!(
        l1.expires_at <= t_acquire + LEASE_TTL,
        "expiry within one TTL"
    );

    // read_lease reflects the acquired lease.
    let read = expect_lease(coord_a.read_lease().await, "read after acquire");
    assert_eq!(read, l1);

    // --- 3. Acquire is insert-only: any existing lease blocks it. -----------
    assert_eq!(
        coord_a.acquire().await,
        Err(LeaseError::LeaseHeld),
        "re-acquiring an existing lease is LeaseHeld"
    );
    assert_eq!(
        coord_b.acquire().await,
        Err(LeaseError::LeaseHeld),
        "B cannot acquire over A's lease"
    );

    // --- 4. A challenger cannot take over a fresh, valid lease. -------------
    assert_eq!(
        coord_b.claim_lease().await,
        Err(LeaseError::LeaseHeld),
        "unexpired lease is not takeover-eligible"
    );

    // --- 5. A renews (epoch unchanged); a stale epoch cannot renew. ---------
    clock.advance(LEASE_TTL / 3); // ~one renewal cadence
    let l2 = expect_lease(coord_a.renew(INITIAL_EPOCH).await, "A renew");
    assert_eq!(l2.epoch, INITIAL_EPOCH, "renew never changes the epoch");
    assert!(l2.expires_at > l1.expires_at, "renew extends expiry");
    let stale = Epoch(INITIAL_EPOCH.0 + 5);
    assert_eq!(
        coord_a.renew(stale).await,
        Err(LeaseError::LostLease),
        "renewing at the wrong epoch is LostLease"
    );

    // --- 6. Expired, but still within the safety margin: takeover blocked. --
    clock.advance(LEASE_TTL + Duration::from_secs(1)); // just past expiry
    assert_eq!(
        coord_b.claim_lease().await,
        Err(LeaseError::LeaseHeld),
        "expired-but-within-safety-margin is not yet takeover-eligible"
    );

    // --- 7. Past the safety margin: B takes over, epoch bumps. --------------
    clock.advance(TAKEOVER_SAFETY_MARGIN);
    let l3 = expect_lease(coord_b.claim_lease().await, "B takeover");
    let Ok(expected_epoch) = INITIAL_EPOCH.checked_next() else {
        panic!("INITIAL_EPOCH advances without overflow");
    };
    assert_eq!(
        l3.epoch, expected_epoch,
        "takeover advances the epoch by one"
    );
    assert!(l3.epoch.0 > l2.epoch.0, "epoch is strictly monotonic");
    assert_eq!(l3.holder_id, *coord_b.holder_id());
    assert_eq!(l3.region, region_b, "takeover records the new region");
    let read_b = expect_lease(coord_b.read_lease().await, "read after takeover");
    assert_eq!(read_b, l3);

    // --- 8. The demoted primary A is fenced. --------------------------------
    assert_eq!(
        coord_a.renew(INITIAL_EPOCH).await,
        Err(LeaseError::LostLease),
        "old primary cannot renew after epoch advance (fencing)"
    );
    assert_eq!(
        coord_a.claim_lease().await,
        Err(LeaseError::LeaseHeld),
        "A cannot immediately reclaim B's fresh lease"
    );

    // --- 9. New primary B renews at the advanced epoch. ---------------------
    clock.advance(LEASE_TTL / 3);
    let l4 = expect_lease(coord_b.renew(l3.epoch).await, "B renew at new epoch");
    assert_eq!(l4.epoch, expected_epoch);
    assert!(l4.expires_at > l3.expires_at, "B's renewal extends expiry");
}
