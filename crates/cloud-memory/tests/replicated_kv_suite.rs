//! Runs the shared [`ReplicatedKv`](cloud_types::ReplicatedKv) conformance
//! suite (spec §9.7) against [`MemoryReplicatedKv`] -- including the
//! concurrent-CAS property cases (spec §19.2), which is why this test uses
//! the multi-thread runtime flavor (mirrors the original
//! `replicated_kv_concurrency.rs` this file replaces).

use cloud_memory::MemoryReplicatedKv;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn memory_replicated_kv_passes_the_shared_suite() {
    cloud_test_suite::run_replicated_kv_suite(|| async { MemoryReplicatedKv::new() }).await;
}
