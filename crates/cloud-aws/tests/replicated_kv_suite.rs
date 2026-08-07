//! `LocalStack`-backed conformance run of the shared `ReplicatedKv` suite
//! (ticket mtc-lf7), including the concurrent-CAS property cases (spec
//! §19.2) -- hence the multi-thread runtime flavor, mirroring
//! `crates/cloud-memory/tests/replicated_kv_suite.rs`'s identical choice.
//!
//! Requires a running `LocalStack` container --
//! `docker compose -f deploy/local/docker-compose.yml up -d --wait
//! localstack` -- and the `integration` feature:
//! `cargo test -p cloud-aws --features integration`.

mod support;

use cloud_aws::DynamoDbReplicatedKv;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn ddb_replicated_kv_passes_shared_conformance_suite() {
    let config = support::provision_test_table().await;
    cloud_test_suite::run_replicated_kv_suite(|| async {
        DynamoDbReplicatedKv::new(config.clone())
    })
    .await;
}
