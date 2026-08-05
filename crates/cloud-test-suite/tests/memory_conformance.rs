//! Self-validation (spec §9.7: "the memory backend proves the suite
//! itself"): runs every `run_*_suite` function here against `cloud-memory`,
//! the reference backend. This is the mirror image of
//! `crates/cloud-memory/tests/` -- those files prove `cloud-memory` passes
//! the shared suites; this file proves the suites themselves are sound
//! (compile, run, and pass) before any other backend depends on them.

use std::sync::Arc;

use clock::{Clock, FakeClock};
use cloud_memory::{MemoryHsm, MemoryObjectLock, MemoryObjectStore, MemoryReplicatedKv};

#[tokio::test]
async fn object_store_suite_passes_against_memory() {
    cloud_test_suite::run_object_store_suite(|| async {
        MemoryObjectStore::new(Arc::new(FakeClock::default()))
    })
    .await;
}

#[tokio::test]
async fn object_lock_suite_passes_against_memory() {
    let clock = Arc::new(FakeClock::default());
    let suite_clock: Arc<dyn Clock> = clock.clone();
    cloud_test_suite::run_object_lock_suite(
        || async { MemoryObjectLock::new(clock.clone()) },
        suite_clock,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn replicated_kv_suite_passes_against_memory() {
    cloud_test_suite::run_replicated_kv_suite(|| async { MemoryReplicatedKv::new() }).await;
}

#[tokio::test]
async fn hsm_suite_passes_against_memory() {
    cloud_test_suite::run_hsm_suite(|| async { MemoryHsm::new() }, false).await;
}
