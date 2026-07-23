//! Object-safety and `Arc<dyn Trait>` usability assertions (spec §9.4, §19.1).
//!
//! These tests pin the ticket's contract that all four traits are
//! object-safe, `Send + Sync`, and usable as `Arc<dyn Trait>` shared across
//! tokio tasks — the exact shape the `Backend` factory wires at startup.
//! The stub implementations here return canned values only; real semantics
//! belong to the backend crates and the shared conformance suites (§9.7).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use cloud_types::{
    CloudError, Condition, Hsm, Item, Key, KeyHandle, KeySpec, ObjectInfo, ObjectLock,
    ObjectMetadata, ObjectStore, Operation, PublicKey, PutOptions, ReplicatedKv, UpdateExpression,
    Value,
};

/// A single stub type implementing all four traits with canned responses,
/// mirroring how one backend crate provides the whole capability set.
struct StubBackend;

const FIXED_TIME_OFFSET: Duration = Duration::from_secs(1_753_000_000);

fn fixed_time() -> SystemTime {
    // Tests may construct fixed instants; production code would use an
    // injected Clock (spec §22.11).
    SystemTime::UNIX_EPOCH + FIXED_TIME_OFFSET
}

#[async_trait]
impl ObjectStore for StubBackend {
    async fn put(&self, _key: &str, _data: &[u8], _opts: PutOptions) -> Result<(), CloudError> {
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        Err(CloudError::NotFound {
            key: key.to_string(),
        })
    }

    async fn head(&self, _key: &str) -> Result<ObjectMetadata, CloudError> {
        Ok(ObjectMetadata {
            size_bytes: 4,
            last_modified: fixed_time(),
        })
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectInfo>, CloudError> {
        Ok(vec![ObjectInfo {
            key: format!("{prefix}0001"),
            size_bytes: 4,
            last_modified: fixed_time(),
        }])
    }

    async fn delete(&self, _key: &str) -> Result<(), CloudError> {
        Ok(())
    }
}

#[async_trait]
impl ObjectLock for StubBackend {
    async fn put_with_retention(
        &self,
        _key: &str,
        _data: &[u8],
        _retain_until: SystemTime,
    ) -> Result<(), CloudError> {
        Ok(())
    }

    async fn extend_retention(
        &self,
        _key: &str,
        _new_retain_until: SystemTime,
    ) -> Result<(), CloudError> {
        Err(CloudError::RetentionViolation {
            reason: "retention is forward-only".to_string(),
        })
    }

    async fn get_retention(&self, _key: &str) -> Result<SystemTime, CloudError> {
        Ok(fixed_time())
    }
}

#[async_trait]
impl ReplicatedKv for StubBackend {
    async fn get(&self, key: &Key) -> Result<Item, CloudError> {
        Ok(Item {
            key: key.clone(),
            value: Value::U64(7),
        })
    }

    async fn put(
        &self,
        _key: &Key,
        _value: Value,
        conditions: &[Condition],
    ) -> Result<(), CloudError> {
        if conditions.contains(&Condition::NotExists) {
            return Err(CloudError::ConditionFailed {
                reason: "item exists".to_string(),
            });
        }
        Ok(())
    }

    async fn atomic_update(
        &self,
        key: &Key,
        _expr: UpdateExpression,
        _conditions: &[Condition],
    ) -> Result<Item, CloudError> {
        Ok(Item {
            key: key.clone(),
            value: Value::Map(BTreeMap::from([("next_index".to_string(), Value::U64(8))])),
        })
    }

    async fn transact(&self, _ops: Vec<Operation>) -> Result<(), CloudError> {
        Ok(())
    }

    async fn query(&self, prefix: &str) -> Result<Vec<Item>, CloudError> {
        Ok(vec![Item {
            key: Key::new(format!("{prefix}lease")),
            value: Value::Bool(true),
        }])
    }
}

#[async_trait]
impl Hsm for StubBackend {
    async fn sign(&self, _key_handle: &KeyHandle, _data: &[u8]) -> Result<Vec<u8>, CloudError> {
        Ok(vec![0u8; 64])
    }

    async fn get_public_key(&self, key_handle: &KeyHandle) -> Result<PublicKey, CloudError> {
        Err(CloudError::NotFound {
            key: key_handle.as_str().to_string(),
        })
    }

    async fn generate_key(&self, _spec: KeySpec) -> Result<KeyHandle, CloudError> {
        Ok(KeyHandle::new("stub-key-1"))
    }

    fn is_fips_validated(&self) -> bool {
        false
    }
}

fn assert_send_sync<T: Send + Sync + ?Sized>() {}

/// Compile-time proof: each trait is object-safe (`dyn Trait` is a valid
/// type) and its trait objects are `Send + Sync`, so `Arc<dyn Trait>` can be
/// shared across tasks (spec §9.4).
#[test]
fn traits_are_object_safe_and_trait_objects_are_send_sync() {
    assert_send_sync::<dyn ObjectStore>();
    assert_send_sync::<dyn ObjectLock>();
    assert_send_sync::<dyn ReplicatedKv>();
    assert_send_sync::<dyn Hsm>();

    assert_send_sync::<Arc<dyn ObjectStore>>();
    assert_send_sync::<Arc<dyn ObjectLock>>();
    assert_send_sync::<Arc<dyn ReplicatedKv>>();
    assert_send_sync::<Arc<dyn Hsm>>();
}

/// The §9.4 `Backend` shape: one struct holding all four capabilities as
/// `Arc<dyn Trait>` handles, buildable from a single backend type.
struct Backend {
    object_store: Arc<dyn ObjectStore>,
    object_lock: Arc<dyn ObjectLock>,
    replicated_kv: Arc<dyn ReplicatedKv>,
    hsm: Arc<dyn Hsm>,
}

fn stub_backend() -> Backend {
    Backend {
        object_store: Arc::new(StubBackend),
        object_lock: Arc::new(StubBackend),
        replicated_kv: Arc::new(StubBackend),
        hsm: Arc::new(StubBackend),
    }
}

#[tokio::test]
async fn object_store_methods_are_callable_through_arc_dyn() {
    let store: Arc<dyn ObjectStore> = Arc::new(StubBackend);

    store
        .put("entries/0001", b"leaf", PutOptions::if_not_exists())
        .await
        .expect("stub put succeeds");
    let missing = store
        .get("entries/0001")
        .await
        .expect_err("stub get misses");
    assert!(matches!(missing, CloudError::NotFound { .. }));
    let meta = store
        .head("entries/0001")
        .await
        .expect("stub head succeeds");
    assert_eq!(meta.size_bytes, 4);
    let listed = store.list("entries/").await.expect("stub list succeeds");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "entries/0001");
    store
        .delete("entries/0001")
        .await
        .expect("stub delete succeeds");
}

#[tokio::test]
async fn object_lock_methods_are_callable_through_arc_dyn() {
    let lock: Arc<dyn ObjectLock> = Arc::new(StubBackend);
    let retain_until = fixed_time() + Duration::from_secs(3600);

    lock.put_with_retention("checkpoints/0001", b"cp", retain_until)
        .await
        .expect("stub put_with_retention succeeds");
    let violation = lock
        .extend_retention("checkpoints/0001", fixed_time())
        .await
        .expect_err("stub rejects shortening");
    assert!(matches!(violation, CloudError::RetentionViolation { .. }));
    let retained = lock
        .get_retention("checkpoints/0001")
        .await
        .expect("stub get_retention succeeds");
    assert_eq!(retained, fixed_time());
}

#[tokio::test]
async fn replicated_kv_methods_are_callable_through_arc_dyn() {
    let kv: Arc<dyn ReplicatedKv> = Arc::new(StubBackend);
    let key = Key::new("coord/counter");

    let item = kv.get(&key).await.expect("stub get succeeds");
    assert_eq!(item.value, Value::U64(7));

    let lost = kv
        .put(&key, Value::U64(8), &[Condition::NotExists])
        .await
        .expect_err("stub reports CAS loss");
    assert!(lost.is_precondition_failure());

    let updated = kv
        .atomic_update(
            &key,
            UpdateExpression::new().increment("next_index", 1),
            &[Condition::AttributeEquals {
                attribute: "epoch".to_string(),
                expected: Value::U64(3),
            }],
        )
        .await
        .expect("stub atomic_update succeeds");
    assert_eq!(updated.key, key);

    kv.transact(vec![Operation::ConditionCheck {
        key: key.clone(),
        conditions: vec![Condition::Exists],
    }])
    .await
    .expect("stub transact succeeds");

    let items = kv.query("coord/").await.expect("stub query succeeds");
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn hsm_methods_are_callable_through_arc_dyn() {
    let hsm: Arc<dyn Hsm> = Arc::new(StubBackend);

    let handle = hsm
        .generate_key(KeySpec::EcdsaP256)
        .await
        .expect("stub generate_key succeeds");
    let signature = hsm
        .sign(&handle, b"checkpoint bytes")
        .await
        .expect("stub sign succeeds");
    assert_eq!(signature.len(), 64, "P1363 r||s encoding for P-256");
    let unknown = hsm
        .get_public_key(&KeyHandle::new("missing"))
        .await
        .expect_err("stub reports unknown handle");
    assert!(matches!(unknown, CloudError::NotFound { .. }));
    assert!(!hsm.is_fips_validated(), "stub/dev backends are non-FIPS");
}

/// `Arc<dyn Trait>` handles must be shareable across concurrently running
/// tokio tasks — the runtime usage pattern of the CA service (spec §9.4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backend_handles_are_shareable_across_tasks() {
    let backend = Arc::new(stub_backend());

    let mut joins = Vec::new();
    for task in 0..4u32 {
        let backend = Arc::clone(&backend);
        joins.push(tokio::spawn(async move {
            let key = format!("entries/{task:04}");
            backend
                .object_store
                .put(&key, b"leaf", PutOptions::default())
                .await
                .expect("put succeeds");
            backend
                .object_lock
                .get_retention(&key)
                .await
                .expect("get_retention succeeds");
            backend
                .replicated_kv
                .get(&Key::new(key))
                .await
                .expect("kv get succeeds");
            assert!(!backend.hsm.is_fips_validated());
        }));
    }
    for join in joins {
        join.await.expect("task completes");
    }
}
