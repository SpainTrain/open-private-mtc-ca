//! Runs the shared [`Hsm`](cloud_types::Hsm) conformance suite (spec §9.7)
//! against [`MemoryHsm`].

use cloud_memory::MemoryHsm;

#[tokio::test]
async fn memory_hsm_passes_the_shared_suite() {
    // MemoryHsm::is_fips_validated() is always false (spec §14.4) -- dev-only
    // by definition.
    cloud_test_suite::run_hsm_suite(|| async { MemoryHsm::new() }, false).await;
}
