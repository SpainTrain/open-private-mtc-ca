//! Shared `LocalStack` test-bucket/-table bootstrap for `cloud-aws`'s
//! integration tests (`--features integration`, ticket aws-backend Testing
//! AC; ticket mtc-lf7 adds the `DynamoDB` half).
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
//! [`provision_test_table`] does the `DynamoDB` equivalent: a fresh table with
//! the same `(PK: String HASH, SK: String RANGE)` key schema as
//! `01-init-mtc.sh`'s `mtc-log-coordination` table (spec §8.2) -- fresh per
//! run for the identical "shared suite uses fixed keys" reason above,
//! rather than reusing the `deploy/local` coordination table.
//!
//! This support module builds its own raw `aws_sdk_s3::Client`/
//! `aws_sdk_dynamodb::Client` directly (mirroring
//! `crates/dev-replicator/tests/integration.rs`'s `s3_client` helper)
//! rather than going through `cloud_aws::S3Config`/`DynamoDbConfig` --
//! bucket/table bootstrapping is test-fixture plumbing, not part of the
//! library's public surface under test.
//!
//! `mod support;` is included whole by three independent `[[test]]` binaries
//! (`object_store_suite`, `object_lock_suite`, `replicated_kv_suite`), each
//! of which calls only the S3 *or* the `DynamoDB` half -- so from any one
//! binary's own dead-code analysis, the other half's helpers are genuinely
//! unreachable, even though every function here is used by at least one
//! sibling binary. `docs/lint-policy.md`'s "prefer fixing the code" does not
//! have a code-shaped fix for this: splitting the file per binary would only
//! duplicate the S3/DynamoDB bootstrap logic, and every `[[test]]` target
//! compiles this module independently regardless of file layout.

#![allow(dead_code)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_sdk_dynamodb::client::Waiters;
use aws_sdk_dynamodb::config::{
    BehaviorVersion as DdbBehaviorVersion, Builder as DdbBuilder, Credentials as DdbCredentials,
    Region as DdbRegion,
};
use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType,
};
use aws_sdk_dynamodb::Client as DdbClient;
use aws_sdk_s3::config::{BehaviorVersion, Builder, Credentials, Region};
use aws_sdk_s3::types::{
    BucketVersioningStatus, ObjectLockConfiguration, ObjectLockEnabled, VersioningConfiguration,
};
use aws_sdk_s3::Client;
use cloud_aws::{DynamoDbConfig, S3Config};

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

fn raw_ddb_client() -> DdbClient {
    let credentials = DdbCredentials::new("test", "test", None, None, "cloud-aws-ddb-test-support");
    let config = DdbBuilder::new()
        .behavior_version(DdbBehaviorVersion::latest())
        .region(DdbRegion::new("us-east-1"))
        .endpoint_url(ENDPOINT)
        .credentials_provider(credentials)
        .build();
    DdbClient::from_conf(config)
}

/// A collision-resistant-enough table name for one test run -- see
/// [`unique_bucket_name`]'s identical rationale.
#[allow(clippy::disallowed_methods)]
fn unique_table_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("cloud-aws-ddb-test-{nanos}")
}

/// Creates a fresh table -- `(PK: String HASH, SK: String RANGE)`, the same
/// key schema `01-init-mtc.sh` gives `mtc-log-coordination` (spec §8.2) --
/// and returns a [`DynamoDbConfig`] targeting it. Fresh and uniquely-named
/// per run for the same reason [`provision_test_bucket`] uses a fresh
/// bucket: the shared suite's fixed key names would otherwise collide with
/// state left over from a previous run against a long-lived table.
///
/// # Panics
///
/// Panics (via `.expect`) if `LocalStack` is unreachable, a setup call
/// fails, or the table does not reach `ACTIVE` within 30s -- see
/// [`provision_test_bucket`]'s identical rationale.
#[allow(clippy::expect_used)]
pub async fn provision_test_table() -> DynamoDbConfig {
    let table = unique_table_name();
    let client = raw_ddb_client();
    client
        .create_table()
        .table_name(&table)
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("PK")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("valid PK attribute definition"),
        )
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("SK")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("valid SK attribute definition"),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("PK")
                .key_type(KeyType::Hash)
                .build()
                .expect("valid PK key schema element"),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("SK")
                .key_type(KeyType::Range)
                .build()
                .expect("valid SK key schema element"),
        )
        .billing_mode(BillingMode::PayPerRequest)
        .send()
        .await
        .expect("create cloud-aws DynamoDB test table on LocalStack");
    client
        .wait_until_table_exists()
        .table_name(&table)
        .wait(Duration::from_secs(30))
        .await
        .expect("DynamoDB test table becomes ACTIVE");
    DynamoDbConfig::localstack(table, ENDPOINT)
}
