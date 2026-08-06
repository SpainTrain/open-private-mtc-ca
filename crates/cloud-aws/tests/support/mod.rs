//! Shared `LocalStack` test-bucket bootstrap for `cloud-aws`'s integration
//! tests (`--features integration`, ticket aws-backend Testing AC).
//!
//! [`provision_test_bucket`] creates a *fresh, uniquely-named* bucket per
//! test run -- versioning + Object Lock enabled, but *no* bucket-level
//! default retention rule (unlike `deploy/local`'s `mtc-log-local` bucket,
//! which carries a 1-day Compliance default -- see
//! `deploy/local/localstack/init/ready.d/01-init-mtc.sh`). Two reasons a
//! fresh bucket beats a fixed, long-lived one here:
//!
//! - A default retention rule would attach Object Lock retention to every
//!   plain `ObjectStore::put`, breaking the shared `cloud-test-suite`'s
//!   `test_delete_removes_object` case (an immediate delete after a plain
//!   put must succeed) -- see `crates/cloud-aws/src/lib.rs`'s "Bucket
//!   prerequisites" docs.
//! - The shared suites use fixed key names (e.g. `cts/object-lock/
//!   round-trip`) and some of those keys end up genuinely, permanently
//!   retained by the `ObjectLock` suite's `put_with_retention` cases.
//!   Re-running the suite against the *same* bucket would collide with the
//!   still-locked objects from the previous run. A fresh bucket per run
//!   sidesteps that without touching the shared suite's key names.
//!
//! This support module builds its own raw `aws_sdk_s3::Client` directly
//! (mirroring `crates/dev-replicator/tests/integration.rs`'s `s3_client`
//! helper) rather than going through `cloud_aws::S3Config` -- bucket
//! bootstrapping is test-fixture plumbing, not part of the library's public
//! surface under test.

use std::time::{SystemTime, UNIX_EPOCH};

use aws_sdk_s3::config::{BehaviorVersion, Builder, Credentials, Region};
use aws_sdk_s3::types::{
    BucketVersioningStatus, ObjectLockConfiguration, ObjectLockEnabled, VersioningConfiguration,
};
use aws_sdk_s3::Client;
use cloud_aws::S3Config;

/// `LocalStack` endpoint brought up by `deploy/local/docker-compose.yml`.
pub const ENDPOINT: &str = "http://127.0.0.1:4566";

fn raw_client() -> Client {
    let credentials = Credentials::new("test", "test", None, None, "cloud-aws-test-support");
    let config = Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .endpoint_url(ENDPOINT)
        .credentials_provider(credentials)
        .force_path_style(true)
        .build();
    Client::from_conf(config)
}

/// A collision-resistant-enough bucket name for one test run.
///
/// Ambient wall-clock time used only to make the bucket name unique across
/// repeated runs against a long-lived `LocalStack` container -- not domain
/// time, so the `no-systemtime-now-in-prod` scoped-allow exemption for test
/// code applies (mirrors `crates/dev-replicator/tests/integration.rs`'s
/// `unique` helper).
#[allow(clippy::disallowed_methods)]
fn unique_bucket_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("cloud-aws-test-{nanos}")
}

/// Creates a fresh bucket -- versioning + Object Lock enabled, no default
/// retention rule -- and returns an [`S3Config`] targeting it (see module
/// docs for why fresh-per-run beats a fixed bucket).
///
/// # Panics
///
/// Panics (via `.expect`) if `LocalStack` is unreachable or a setup call
/// fails -- a clear signal that
/// `docker compose -f deploy/local/docker-compose.yml up -d --wait
/// localstack` has not been run.
// This is a non-#[test] helper in an integration-test file, so the
// allow-expect-in-tests clippy.toml exemption does not auto-apply (see
// docs/lint-policy.md deviation 1) -- scoped allow with the same
// justification: test fixture setup, not production code.
#[allow(clippy::expect_used)]
pub async fn provision_test_bucket() -> S3Config {
    let bucket = unique_bucket_name();
    let client = raw_client();
    client
        .create_bucket()
        .bucket(&bucket)
        .object_lock_enabled_for_bucket(true)
        .send()
        .await
        .expect("create cloud-aws test bucket on LocalStack");
    client
        .put_bucket_versioning()
        .bucket(&bucket)
        .versioning_configuration(
            VersioningConfiguration::builder()
                .status(BucketVersioningStatus::Enabled)
                .build(),
        )
        .send()
        .await
        .expect("enable bucket versioning");
    client
        .put_object_lock_configuration()
        .bucket(&bucket)
        .object_lock_configuration(
            // No `.rule(...)` -- deliberately no bucket-level default
            // retention (see module docs).
            ObjectLockConfiguration::builder()
                .object_lock_enabled(ObjectLockEnabled::Enabled)
                .build(),
        )
        .send()
        .await
        .expect("enable Object Lock with no default retention rule");
    S3Config::localstack(bucket, ENDPOINT)
}
