//! Runs the shared [`ObjectStore`](cloud_types::ObjectStore) conformance
//! suite (spec §9.7) against [`MemoryObjectStore`].

use std::sync::Arc;

use clock::FakeClock;
use cloud_memory::MemoryObjectStore;

#[tokio::test]
async fn memory_object_store_passes_the_shared_suite() {
    cloud_test_suite::run_object_store_suite(|| async {
        MemoryObjectStore::new(Arc::new(FakeClock::default()))
    })
    .await;
}
