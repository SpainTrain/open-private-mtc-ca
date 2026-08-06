//! Runs the reusable lease/epoch conformance suite ([`run_lease_suite`])
//! against the `cloud-memory` backend (spec §9.7). The identical call will run
//! against the cloud-aws `DynamoDB` backend once it lands — see
//! `tests/lease_ddb.rs`.

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use clock::FakeClock;
use cloud_memory::MemoryReplicatedKv;
use cloud_types::ReplicatedKv;
use coordination::run_lease_suite;

#[tokio::test]
async fn lease_suite_passes_against_memory_backend() {
    // Whole-second base so millisecond `expires_at` resolution is exact.
    let clock = Arc::new(FakeClock::new(
        UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    ));
    let kv: Arc<dyn ReplicatedKv> = Arc::new(MemoryReplicatedKv::new());
    run_lease_suite(kv, clock).await;
}
