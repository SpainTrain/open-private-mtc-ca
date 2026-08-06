//! `LocalStack`-backed conformance run of the shared `ObjectStore` suite
//! (ticket aws-backend Testing AC).
//!
//! Requires a running `LocalStack` container --
//! `docker compose -f deploy/local/docker-compose.yml up -d --wait
//! localstack` -- and the `integration` feature:
//! `cargo test -p cloud-aws --features integration`.

mod support;

use cloud_aws::S3ObjectStore;

#[tokio::test]
async fn s3_object_store_passes_shared_conformance_suite() {
    let config = support::provision_test_bucket().await;
    cloud_test_suite::run_object_store_suite(|| async { S3ObjectStore::new(config.clone()) }).await;
}
