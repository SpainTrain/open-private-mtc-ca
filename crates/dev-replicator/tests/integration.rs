//! Integration tests against two live `LocalStack` instances (ticket
//! dev-crr-replication-sim Testing AC: "two `LocalStack` containers — object
//! written to A appears in B after the lag, not before; pause halts
//! propagation").
//!
//! `#[ignore]`-gated: these need real Docker containers up, which
//! `cargo test` does not provide by default. Run them via:
//!
//! ```console
//! $ tests/e2e/replication-sim-demo.sh
//! ```
//!
//! or manually, after `make replication-sim-up`:
//!
//! ```console
//! $ cargo test -p dev-replicator --test integration -- --ignored --test-threads=1
//! ```
//!
//! Each test uses a unique key/pk prefixed with the test name plus a
//! nanosecond timestamp so repeated runs against the same long-lived
//! containers never collide.
//!
//! Lag timing is driven by an injected [`FakeClock`], not wall-clock sleeps
//! (rule `no-systemtime-now-in-prod`) — these tests are deterministic and
//! fast even though they hit real `LocalStack` over the network for the
//! object/item data path itself.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_s3::primitives::ByteStream;
use clock::FakeClock;
use dev_replicator::ddb::{DdbPoller, TS_ATTR};
use dev_replicator::lag::LagPolicy;
use dev_replicator::link::Link;
use dev_replicator::s3::S3Poller;

const ENDPOINT_A: &str = "http://127.0.0.1:4566";
const ENDPOINT_B: &str = "http://127.0.0.1:4567";
const BUCKET: &str = "mtc-log-local";
const TABLE: &str = "mtc-log-coordination";

fn s3_client(endpoint: &str) -> aws_sdk_s3::Client {
    let credentials =
        aws_sdk_s3::config::Credentials::new("test", "test", None, None, "dev-replicator-test");
    let config = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .region(aws_sdk_s3::config::Region::new("us-east-1"))
        .endpoint_url(endpoint)
        .credentials_provider(credentials)
        .force_path_style(true)
        .build();
    aws_sdk_s3::Client::from_conf(config)
}

fn ddb_client(endpoint: &str) -> aws_sdk_dynamodb::Client {
    let credentials = aws_sdk_dynamodb::config::Credentials::new(
        "test",
        "test",
        None,
        None,
        "dev-replicator-test",
    );
    let config = aws_sdk_dynamodb::config::Builder::new()
        .behavior_version(aws_sdk_dynamodb::config::BehaviorVersion::latest())
        .region(aws_sdk_dynamodb::config::Region::new("us-east-1"))
        .endpoint_url(endpoint)
        .credentials_provider(credentials)
        .build();
    aws_sdk_dynamodb::Client::from_conf(config)
}

/// A unique-enough key for one test run.
///
/// Ambient wall-clock time used only to make test keys collision-resistant
/// across repeated runs against long-lived containers — not domain time, so
/// the `no-systemtime-now-in-prod` scoped-allow exemption for test code
/// applies (docs/lint-policy.md item 2).
#[allow(clippy::disallowed_methods)]
fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("dev-replicator-integration/{prefix}-{nanos}")
}

async fn s3_get_body(client: &aws_sdk_s3::Client, key: &str) -> Option<Vec<u8>> {
    let out = client
        .get_object()
        .bucket(BUCKET)
        .key(key)
        .send()
        .await
        .ok()?;
    let bytes = out.body.collect().await.ok()?.into_bytes();
    Some(bytes.to_vec())
}

async fn ddb_get_value(client: &aws_sdk_dynamodb::Client, pk: &str, sk: &str) -> Option<String> {
    let out = client
        .get_item()
        .table_name(TABLE)
        .key("PK", AttributeValue::S(pk.to_string()))
        .key("SK", AttributeValue::S(sk.to_string()))
        .send()
        .await
        .ok()?;
    let item = out.item?;
    item.get("value")?.as_s().ok().cloned()
}

#[tokio::test]
#[ignore = "requires two LocalStack instances — see tests/e2e/replication-sim-demo.sh"]
async fn s3_object_appears_in_target_only_after_lag_elapses() {
    let source = s3_client(ENDPOINT_A);
    let target = s3_client(ENDPOINT_B);
    let key = unique("s3-lag");

    source
        .put_object()
        .bucket(BUCKET)
        .key(&key)
        .body(ByteStream::from_static(b"hello from region A"))
        .send()
        .await
        .expect("seed source object");

    let clock = Arc::new(FakeClock::new(UNIX_EPOCH));
    let mut poller = S3Poller::new(
        source.clone(),
        target.clone(),
        BUCKET.to_string(),
        clock.clone(),
        LagPolicy::Fixed(Duration::from_secs(5)),
    );

    poller.discover().await.expect("discover source versions");
    let too_early = poller.apply_due().await;
    assert_eq!(
        too_early.applied, 0,
        "must not replicate before lag elapses"
    );
    assert!(
        s3_get_body(&target, &key).await.is_none(),
        "object must be absent from target before the lag elapses"
    );

    clock.advance(Duration::from_secs(5));
    let on_time = poller.apply_due().await;
    assert_eq!(on_time.applied, 1, "must replicate once lag has elapsed");
    assert_eq!(
        s3_get_body(&target, &key).await.as_deref(),
        Some(&b"hello from region A"[..])
    );
}

#[tokio::test]
#[ignore = "requires two LocalStack instances — see tests/e2e/replication-sim-demo.sh"]
async fn s3_pause_halts_propagation_until_resumed() {
    let source = s3_client(ENDPOINT_A);
    let target = s3_client(ENDPOINT_B);
    let key = unique("s3-pause");

    source
        .put_object()
        .bucket(BUCKET)
        .key(&key)
        .body(ByteStream::from_static(b"paused payload"))
        .send()
        .await
        .expect("seed source object");

    let s3 = S3Poller::new(
        source.clone(),
        target.clone(),
        BUCKET.to_string(),
        Arc::new(FakeClock::new(UNIX_EPOCH)),
        LagPolicy::immediate(),
    );
    let (mut link, _status) = Link::new(
        "s3-pause-test".to_string(),
        Duration::from_millis(50),
        LagPolicy::immediate(),
        Some(s3),
        None,
    );
    // Pause before the object was ever discovered — the partition hook.
    link.control_handle().paused.store(true, Ordering::SeqCst);

    link.run_one_cycle().await;
    assert!(
        s3_get_body(&target, &key).await.is_none(),
        "a paused link must not replicate anything"
    );

    link.control_handle().paused.store(false, Ordering::SeqCst);
    link.run_one_cycle().await;
    assert!(
        s3_get_body(&target, &key).await.is_some(),
        "resuming the link replicates the change"
    );
}

#[tokio::test]
#[ignore = "requires two LocalStack instances — see tests/e2e/replication-sim-demo.sh"]
async fn ddb_replicates_updates_and_is_idempotent_on_replay() {
    let source = ddb_client(ENDPOINT_A);
    let target = ddb_client(ENDPOINT_B);
    let pk = unique("ddb-update");
    let sk = "item";

    source
        .put_item()
        .table_name(TABLE)
        .item("PK", AttributeValue::S(pk.clone()))
        .item("SK", AttributeValue::S(sk.to_string()))
        .item("value", AttributeValue::S("v1".to_string()))
        .send()
        .await
        .expect("seed source item");

    let clock = Arc::new(FakeClock::new(UNIX_EPOCH));
    let mut poller = DdbPoller::new(
        source.clone(),
        target.clone(),
        TABLE.to_string(),
        clock.clone(),
        LagPolicy::immediate(),
    );
    poller.discover().await.expect("discover v1");
    let s1 = poller.apply_due().await;
    assert_eq!(s1.applied, 1);
    assert_eq!(ddb_get_value(&target, &pk, sk).await.as_deref(), Some("v1"));

    // Idempotency: re-running discover+apply with no source change queues
    // and applies nothing new.
    clock.advance(Duration::from_secs(1));
    poller.discover().await.expect("re-discover unchanged item");
    let replay = poller.apply_due().await;
    assert_eq!(replay.attempted(), 0, "unchanged content must not re-apply");

    // A genuine update is replicated.
    source
        .put_item()
        .table_name(TABLE)
        .item("PK", AttributeValue::S(pk.clone()))
        .item("SK", AttributeValue::S(sk.to_string()))
        .item("value", AttributeValue::S("v2".to_string()))
        .send()
        .await
        .expect("update source item");
    clock.advance(Duration::from_secs(1));
    poller.discover().await.expect("discover v2");
    let s2 = poller.apply_due().await;
    assert_eq!(s2.applied, 1);
    assert_eq!(ddb_get_value(&target, &pk, sk).await.as_deref(), Some("v2"));
}

#[tokio::test]
#[ignore = "requires two LocalStack instances — see tests/e2e/replication-sim-demo.sh"]
async fn ddb_last_writer_wins_rejects_a_stale_conflicting_write() {
    let source = ddb_client(ENDPOINT_A);
    let target = ddb_client(ENDPOINT_B);
    let pk = unique("ddb-lww");
    let sk = "item";

    // Simulate a *different, faster* link that already replicated a newer
    // write directly onto the target (e.g. the same coordination item,
    // replicated from a peer region that caught up first).
    target
        .put_item()
        .table_name(TABLE)
        .item("PK", AttributeValue::S(pk.clone()))
        .item("SK", AttributeValue::S(sk.to_string()))
        .item("value", AttributeValue::S("from-a-faster-link".to_string()))
        .item(TS_ATTR, AttributeValue::N("99999999999999".to_string()))
        .send()
        .await
        .expect("seed a newer competing write on the target");

    // This link's own source independently has an "older" write for the
    // same key (by replication clock — its apply timestamp will be far
    // earlier than the write already on target).
    source
        .put_item()
        .table_name(TABLE)
        .item("PK", AttributeValue::S(pk.clone()))
        .item("SK", AttributeValue::S(sk.to_string()))
        .item("value", AttributeValue::S("stale-write".to_string()))
        .send()
        .await
        .expect("seed source item");

    let clock = Arc::new(FakeClock::new(UNIX_EPOCH)); // apply ts = 0ms
    let mut poller = DdbPoller::new(
        source.clone(),
        target.clone(),
        TABLE.to_string(),
        clock,
        LagPolicy::immediate(),
    );
    poller.discover().await.expect("discover the stale write");
    let summary = poller.apply_due().await;

    assert_eq!(summary.stale, 1, "the older write must lose the LWW race");
    assert_eq!(summary.applied, 0);
    assert_eq!(
        ddb_get_value(&target, &pk, sk).await.as_deref(),
        Some("from-a-faster-link"),
        "target must retain the newer write, not be clobbered by a stale replay"
    );
}
