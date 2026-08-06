//! `DynamoDB` / `LocalStack` integration test for the lease/epoch protocol
//! (spec §9.7): the *identical* [`run_lease_suite`] the memory backend runs,
//! driven against a real `DynamoDB` key-value backend.
//!
//! Behind the `integration` feature so `cargo test -p coordination` needs no
//! Docker, and `#[ignore]`d because the cloud-aws `DynamoDB` backend does not
//! yet exist (bead mtc-lf7). This file is the finished shape of that test; only
//! [`ddb_backend`] changes when the backend lands.

#![cfg(feature = "integration")]

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use clock::FakeClock;
use cloud_types::ReplicatedKv;
use coordination::run_lease_suite;

/// Builds the KV backend under test.
///
/// TODO(mtc-lf7): replace the in-memory placeholder with the cloud-aws
/// `DynamoDB` backend pointed at `LocalStack` — roughly:
///
/// ```ignore
/// let ddb = cloud_aws::DynamoDbReplicatedKv::connect(localstack_endpoint, table).await;
/// Arc::new(ddb) as Arc<dyn ReplicatedKv>
/// ```
///
/// The suite call below is already final: it is backend-agnostic, so swapping
/// this constructor is the only change needed to make it a real DDB test.
// `async` to match the future async DDB constructor; placeholder to keep the
// file compiling under `--all-features` before mtc-lf7 lands.
#[allow(clippy::unused_async)]
async fn ddb_backend() -> Arc<dyn ReplicatedKv> {
    Arc::new(cloud_memory::MemoryReplicatedKv::new())
}

#[tokio::test]
#[ignore = "pending mtc-lf7: cloud-aws DynamoDB ReplicatedKv against LocalStack; swap ddb_backend() and remove ignore"]
async fn lease_suite_passes_against_dynamodb() {
    let clock = Arc::new(FakeClock::new(
        UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    ));
    let kv = ddb_backend().await;
    run_lease_suite(kv, clock).await;
}
