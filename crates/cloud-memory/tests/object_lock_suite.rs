//! Runs the shared [`ObjectLock`](cloud_types::ObjectLock) conformance suite
//! (spec §9.7) against [`MemoryObjectLock`].

use std::sync::Arc;

use clock::{Clock, FakeClock};
use cloud_memory::MemoryObjectLock;

#[tokio::test]
async fn memory_object_lock_passes_the_shared_suite() {
    let clock = Arc::new(FakeClock::default());
    let suite_clock: Arc<dyn Clock> = clock.clone();
    cloud_test_suite::run_object_lock_suite(
        || async { MemoryObjectLock::new(clock.clone()) },
        suite_clock,
    )
    .await;
}
