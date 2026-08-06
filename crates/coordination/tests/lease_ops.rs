//! Per-operation unit tests for the lease/epoch protocol against the
//! `cloud-memory` backend (spec §8.2/§8.3). Each test isolates one operation
//! and one outcome (success or a specific typed error); the end-to-end
//! scenario lives in `tests/lease_suite.rs` and the concurrent-claim property
//! in `tests/claim_property.rs`.

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use clock::{Clock, FakeClock};
use cloud_memory::MemoryReplicatedKv;
use cloud_types::ReplicatedKv;
use coordination::{
    Epoch, HolderId, LeaseCoordinator, LeaseError, LogId, Region, INITIAL_EPOCH, LEASE_TTL,
    TAKEOVER_SAFETY_MARGIN,
};

/// A whole-second base so the millisecond resolution of `expires_at` is exact.
fn base_clock() -> Arc<FakeClock> {
    Arc::new(FakeClock::new(
        UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    ))
}

fn fresh_kv() -> Arc<dyn ReplicatedKv> {
    Arc::new(MemoryReplicatedKv::new())
}

// Non-`#[test]` helper in an integration test needs a scoped unwrap allow
// (docs/lint-policy.md): the log-id literal is statically non-empty.
#[allow(clippy::unwrap_used)]
fn coordinator(
    kv: &Arc<dyn ReplicatedKv>,
    clock: &Arc<FakeClock>,
    holder: &str,
    region: &str,
) -> LeaseCoordinator {
    let log_id = LogId::new("ops-log").unwrap();
    let dyn_clock: Arc<dyn Clock> = clock.clone();
    LeaseCoordinator::new(
        Arc::clone(kv),
        dyn_clock,
        &log_id,
        HolderId::new(holder),
        Region::new(region),
    )
}

#[tokio::test]
async fn acquire_creates_lease_at_initial_epoch() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let a = coordinator(&kv, &clock, "inst-a", "us-east-1");

    let lease = a.acquire().await.expect("acquire from empty succeeds");
    assert_eq!(lease.epoch, INITIAL_EPOCH);
    assert_eq!(lease.holder_id, HolderId::new("inst-a"));
    assert_eq!(lease.region, Region::new("us-east-1"));
    assert_eq!(lease.expires_at, clock.now() + LEASE_TTL);
}

#[tokio::test]
async fn acquire_over_existing_lease_is_lease_held() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let a = coordinator(&kv, &clock, "inst-a", "us-east-1");
    let b = coordinator(&kv, &clock, "inst-b", "us-west-2");

    a.acquire().await.expect("A acquires");
    assert_eq!(a.acquire().await, Err(LeaseError::LeaseHeld));
    assert_eq!(b.acquire().await, Err(LeaseError::LeaseHeld));
}

#[tokio::test]
async fn read_lease_on_empty_is_no_lease() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let a = coordinator(&kv, &clock, "inst-a", "us-east-1");
    assert!(matches!(
        a.read_lease().await,
        Err(LeaseError::NoLease { .. })
    ));
}

#[tokio::test]
async fn renew_extends_expiry_and_keeps_epoch() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let a = coordinator(&kv, &clock, "inst-a", "us-east-1");

    let first = a.acquire().await.expect("acquire");
    clock.advance(Duration::from_secs(20));
    let renewed = a.renew(INITIAL_EPOCH).await.expect("renew succeeds");
    assert_eq!(renewed.epoch, INITIAL_EPOCH, "renew never bumps the epoch");
    assert!(renewed.expires_at > first.expires_at, "expiry extended");
    assert_eq!(renewed.expires_at, clock.now() + LEASE_TTL);
}

#[tokio::test]
async fn renew_with_wrong_epoch_is_lost_lease() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let a = coordinator(&kv, &clock, "inst-a", "us-east-1");
    a.acquire().await.expect("acquire");
    assert_eq!(
        a.renew(Epoch(INITIAL_EPOCH.0 + 1)).await,
        Err(LeaseError::LostLease)
    );
}

#[tokio::test]
async fn renew_by_wrong_holder_is_lost_lease() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let a = coordinator(&kv, &clock, "inst-a", "us-east-1");
    let b = coordinator(&kv, &clock, "inst-b", "us-west-2");
    a.acquire().await.expect("A acquires");
    // B knows the current epoch but is not the holder.
    assert_eq!(b.renew(INITIAL_EPOCH).await, Err(LeaseError::LostLease));
}

#[tokio::test]
async fn renew_on_empty_is_no_lease() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let a = coordinator(&kv, &clock, "inst-a", "us-east-1");
    assert!(matches!(
        a.renew(INITIAL_EPOCH).await,
        Err(LeaseError::NoLease { .. })
    ));
}

#[tokio::test]
async fn claim_on_empty_is_no_lease() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let b = coordinator(&kv, &clock, "inst-b", "us-west-2");
    assert!(matches!(
        b.claim_lease().await,
        Err(LeaseError::NoLease { .. })
    ));
}

#[tokio::test]
async fn claim_of_unexpired_lease_is_lease_held() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let a = coordinator(&kv, &clock, "inst-a", "us-east-1");
    let b = coordinator(&kv, &clock, "inst-b", "us-west-2");
    a.acquire().await.expect("A acquires");
    assert_eq!(b.claim_lease().await, Err(LeaseError::LeaseHeld));
}

#[tokio::test]
async fn claim_when_expired_but_within_safety_margin_is_lease_held() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let a = coordinator(&kv, &clock, "inst-a", "us-east-1");
    let b = coordinator(&kv, &clock, "inst-b", "us-west-2");
    a.acquire().await.expect("A acquires");
    // Past the TTL (lease expired) but not yet past the safety margin.
    clock.advance(LEASE_TTL + Duration::from_secs(1));
    assert_eq!(b.claim_lease().await, Err(LeaseError::LeaseHeld));
}

#[tokio::test]
async fn claim_past_safety_margin_takes_over_and_bumps_epoch() {
    let (kv, clock) = (fresh_kv(), base_clock());
    let a = coordinator(&kv, &clock, "inst-a", "us-east-1");
    let b = coordinator(&kv, &clock, "inst-b", "us-west-2");
    let acquired = a.acquire().await.expect("A acquires");

    clock.advance(LEASE_TTL + TAKEOVER_SAFETY_MARGIN + Duration::from_secs(1));
    let taken = b.claim_lease().await.expect("B takes over");

    assert_eq!(
        taken.epoch,
        Epoch(INITIAL_EPOCH.0 + 1),
        "epoch advanced by one"
    );
    assert!(taken.epoch.0 > acquired.epoch.0, "epoch strictly monotonic");
    assert_eq!(taken.holder_id, HolderId::new("inst-b"));
    assert_eq!(taken.region, Region::new("us-west-2"));
    assert_eq!(taken.expires_at, clock.now() + LEASE_TTL);

    // The old holder is now fenced: its epoch-1 renewal fails.
    assert_eq!(a.renew(INITIAL_EPOCH).await, Err(LeaseError::LostLease));
}
