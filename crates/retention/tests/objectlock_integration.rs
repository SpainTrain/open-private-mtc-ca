//! Integration test (ticket `prune-retention-policy` AC: "Helper computing
//! `retain_until`... used by `ObjectLock` `put_with_retention` call sites,
//! §9.1") proving `RetentionPolicy::retain_until`'s output type is exactly
//! what a real `ObjectLock::put_with_retention` call site needs — not just
//! documentation, an executable check against the in-memory backend
//! (`cloud-memory`, spec §9.3/§9.6).

use std::sync::Arc;
use std::time::Duration;

use clock::{Clock, FakeClock};
use cloud_memory::MemoryObjectStore;
use cloud_types::{CloudError, ObjectLock, ObjectStore};
use retention::{ObjectClass, RetentionPolicyConfig};

#[tokio::test]
async fn entry_retain_until_feeds_put_with_retention_and_blocks_early_delete() {
    let clock = Arc::new(FakeClock::default());
    let store = MemoryObjectStore::new(clock.clone());
    // Dev-mode override so the test doesn't need to advance the clock by
    // years (spec §18.4: pruning demoable on a laptop with the fake clock).
    let policy = RetentionPolicyConfig {
        retention_days: 2555,
        dev_override_minutes: Some(5),
    }
    .build()
    .expect("valid dev config");

    let write_time = clock.now();
    let retain_until = policy
        .retain_until(ObjectClass::Entry, write_time)
        .expect("no overflow");

    store
        .put_with_retention("entries/000/000/000000.entry", b"leaf", retain_until)
        .await
        .expect("put_with_retention accepts retain_until as-is");

    assert_eq!(
        store
            .get_retention("entries/000/000/000000.entry")
            .await
            .expect("retention was set"),
        retain_until
    );

    // Still inside the 5-minute dev retention window: delete is refused.
    clock.advance(Duration::from_mins(1));
    let err = store
        .delete("entries/000/000/000000.entry")
        .await
        .expect_err("delete during retention must fail");
    assert!(matches!(err, CloudError::RetentionViolation { .. }));

    // Past the window: delete succeeds.
    clock.advance(Duration::from_mins(5));
    store
        .delete("entries/000/000/000000.entry")
        .await
        .expect("delete after retention expiry succeeds");
}

#[tokio::test]
async fn pruning_checkpoint_retain_until_survives_far_clock_advancement() {
    let clock = Arc::new(FakeClock::default());
    let store = MemoryObjectStore::new(clock.clone());
    // Even a dev override with minute-scale entry/tile retention must not
    // affect pruning checkpoints (spec §15.3: retained indefinitely).
    let policy = RetentionPolicyConfig {
        retention_days: 2555,
        dev_override_minutes: Some(1),
    }
    .build()
    .expect("valid dev config");

    let write_time = clock.now();
    let retain_until = policy
        .retain_until(ObjectClass::PruningCheckpoint, write_time)
        .expect("indefinite retention never overflows");
    assert_eq!(retain_until, retention::indefinite_retention());

    store
        .put_with_retention("checkpoints/0000000000000256.signed", b"cp", retain_until)
        .await
        .expect("put_with_retention accepts the indefinite sentinel");

    // Advance by 100 years — far past any dev/entry/tile retention window —
    // the pruning checkpoint must still be locked.
    clock.advance(Duration::from_hours(100 * 365 * 24));
    let err = store
        .delete("checkpoints/0000000000000256.signed")
        .await
        .expect_err("pruning checkpoints are never deletable via this path");
    assert!(matches!(err, CloudError::RetentionViolation { .. }));
}
