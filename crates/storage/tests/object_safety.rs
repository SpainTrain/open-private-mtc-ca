//! Object-safety and `Arc<dyn Storage>` usability assertions (spec §9.4,
//! §11.4: `CaService { storage: Arc<dyn Storage>, ... }`), mirroring
//! `cloud-types/tests/object_safety.rs`'s coverage of the four cloud-types
//! traits.

use std::sync::Arc;

use cloud_backend::{build_backend, BackendConfig, Provider};
use mtc::Epoch;
use storage::{S3DdbStorage, Storage, StorageConfig, StorageError};

const fn assert_send_sync<T: Send + Sync + ?Sized>() {}

/// Compile-time proof: `Storage` is object-safe (`dyn Storage` is a valid
/// type) and its trait object is `Send + Sync`, so `Arc<dyn Storage>` can be
/// shared across tasks (spec §9.4, mirrored for this facade).
#[test]
fn storage_is_object_safe_and_trait_objects_are_send_sync() {
    assert_send_sync::<dyn Storage>();
    assert_send_sync::<Arc<dyn Storage>>();
}

// Non-#[test] helper in an integration-test file: the allow-expect-in-tests
// clippy.toml exemption does not auto-apply (docs/lint-policy.md deviation
// 1) -- scoped allow with the same justification: test fixture setup, not
// production code.
#[allow(clippy::expect_used)]
async fn storage_trait_object() -> Arc<dyn Storage> {
    let backend = build_backend(BackendConfig {
        provider: Provider::Memory,
    })
    .await
    .expect("the memory provider always builds");
    let config = StorageConfig {
        bucket: "mtc-log-1".to_string(),
        table: "mtc-coordination".to_string(),
        log_id: "log-1".to_string(),
        retention_days: 2555,
    };
    Arc::new(S3DdbStorage::new(Arc::new(backend), config))
}

#[tokio::test]
async fn methods_are_callable_through_arc_dyn_storage() {
    let storage: Arc<dyn Storage> = storage_trait_object().await;
    assert_eq!(
        storage.read_lease().await,
        Err(StorageError::Unimplemented {
            method: "read_lease"
        }),
    );
}

/// `Arc<dyn Storage>` must be shareable across concurrently running tokio
/// tasks -- the runtime usage pattern `CaService` needs it for (spec §9.4,
/// §11.4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_handle_is_shareable_across_tasks() {
    let storage = storage_trait_object().await;

    let mut joins = Vec::new();
    for task in 0..4u64 {
        let storage = Arc::clone(&storage);
        joins.push(tokio::spawn(async move {
            let err = storage
                .allocate_indices(1, Epoch(task))
                .await
                .expect_err("allocate_indices has no body yet");
            assert_eq!(
                err,
                StorageError::Unimplemented {
                    method: "allocate_indices"
                },
            );
        }));
    }
    for join in joins {
        join.await.expect("task completes");
    }
}
