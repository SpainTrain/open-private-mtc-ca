//! End-to-end `build_backend` tests (ticket `cloud-backend-factory` Testing
//! AC: "memory build round-trip (put/get through Backend)"; Demo: "cargo
//! test -p cloud-backend green with no Docker").

use std::time::Duration;

use clock::{Clock, SystemClock};
use cloud_backend::{build_backend, Backend, BackendConfig, BackendError, Provider};
use cloud_types::{CloudError, Condition, Key, KeySpec, PutOptions, Value};

// Non-#[test] helper in an integration-test file: the allow-expect-in-tests
// clippy.toml exemption does not auto-apply (docs/lint-policy.md deviation
// 1) -- scoped allow with the same justification: test fixture setup, not
// production code.
#[allow(clippy::expect_used)]
async fn memory_backend() -> Backend {
    build_backend(BackendConfig {
        provider: Provider::Memory,
    })
    .await
    .expect("the memory provider always builds")
}

#[tokio::test]
async fn memory_backend_round_trips_through_all_four_capabilities() {
    let backend = memory_backend().await;

    // ObjectStore
    backend
        .object_store
        .put("entries/0001", b"leaf", PutOptions::default())
        .await
        .expect("put succeeds");
    assert_eq!(
        backend
            .object_store
            .get("entries/0001")
            .await
            .expect("get succeeds"),
        b"leaf"
    );

    // ObjectLock
    let retain_until = SystemClock.now() + Duration::from_hours(1);
    backend
        .object_lock
        .put_with_retention("checkpoints/0001", b"cp", retain_until)
        .await
        .expect("put_with_retention succeeds");
    assert_eq!(
        backend
            .object_lock
            .get_retention("checkpoints/0001")
            .await
            .expect("get_retention succeeds"),
        retain_until
    );

    // ReplicatedKv
    let key = Key::new("coord/lease");
    backend
        .replicated_kv
        .put(&key, Value::U64(1), &[Condition::NotExists])
        .await
        .expect("put succeeds");
    let item = backend.replicated_kv.get(&key).await.expect("get succeeds");
    assert_eq!(item.value, Value::U64(1));

    // Hsm
    let handle = backend
        .hsm
        .generate_key(KeySpec::EcdsaP256)
        .await
        .expect("generate_key succeeds");
    let signature = backend
        .hsm
        .sign(&handle, b"checkpoint bytes")
        .await
        .expect("sign succeeds");
    assert_eq!(signature.len(), 64, "P1363 r||s encoding for P-256");
    backend
        .hsm
        .get_public_key(&handle)
        .await
        .expect("get_public_key succeeds");
    assert!(
        !backend.hsm.is_fips_validated(),
        "the memory backend is dev-only and must report that honestly"
    );
}

#[tokio::test]
async fn object_store_and_object_lock_share_the_same_underlying_objects() {
    // build_backend must wrap two clones of one MemoryObjectStore (see
    // factory.rs's docs), not two independent stores that could disagree
    // about what is retained -- this is what makes true append-only
    // enforcement possible at all.
    let backend = memory_backend().await;
    let retain_until = SystemClock.now() + Duration::from_hours(1);
    backend
        .object_lock
        .put_with_retention("checkpoints/0001", b"cp", retain_until)
        .await
        .expect("put_with_retention succeeds");

    assert_eq!(
        backend
            .object_store
            .get("checkpoints/0001")
            .await
            .expect("object written via object_lock is visible via object_store"),
        b"cp"
    );

    let deleted = backend
        .object_store
        .delete("checkpoints/0001")
        .await
        .expect_err("delete must be blocked while the retention window is active");
    assert!(matches!(deleted, CloudError::RetentionViolation { .. }));
}

#[tokio::test]
async fn aws_and_localstack_providers_are_unimplemented() {
    for provider in [Provider::Aws, Provider::Localstack] {
        // Not expect_err/unwrap_err: those require the Ok type (Backend) to
        // implement Debug, which it deliberately does not -- its fields are
        // Arc<dyn Trait> and the cloud-types traits are not Debug.
        match build_backend(BackendConfig { provider }).await {
            Err(err) => assert_eq!(err, BackendError::Unimplemented { provider }),
            Ok(_) => panic!("provider {provider} must not build yet"),
        }
    }
}
