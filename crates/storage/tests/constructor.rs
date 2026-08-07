//! `S3DdbStorage::new` constructor tests (ticket `mtc-f35` AC: "Constructor
//! ... tests run over the pure-memory backend with zero external deps",
//! §9.6).

use std::sync::Arc;

use cloud_backend::{build_backend, BackendConfig, Provider};
use storage::{S3DdbStorage, StorageConfig};

fn sample_config() -> StorageConfig {
    StorageConfig {
        bucket: "mtc-log-1".to_string(),
        table: "mtc-coordination".to_string(),
        log_id: "log-1".to_string(),
        retention_days: 2555,
    }
}

// Non-#[test] helper in an integration-test file: the allow-expect-in-tests
// clippy.toml exemption does not auto-apply (docs/lint-policy.md deviation
// 1) -- scoped allow with the same justification: test fixture setup, not
// production code.
#[allow(clippy::expect_used)]
async fn memory_backend() -> cloud_backend::Backend {
    build_backend(BackendConfig {
        provider: Provider::Memory,
    })
    .await
    .expect("the memory provider always builds")
}

#[tokio::test]
async fn new_wires_the_backend_and_config_over_the_memory_provider() {
    let backend = Arc::new(memory_backend().await);
    let config = sample_config();
    let storage = S3DdbStorage::new(backend, config.clone());

    assert_eq!(storage.config(), &config);

    // The backend is reachable and its four capabilities are live through
    // the facade, proving `new` actually wired the Arc<Backend> through
    // rather than dropping it.
    storage
        .backend()
        .object_store
        .put("entries/0001", b"leaf", cloud_types::PutOptions::default())
        .await
        .expect("memory object_store accepts the write");
}

#[tokio::test]
async fn config_accessor_returns_every_field() {
    let backend = Arc::new(memory_backend().await);
    let config = sample_config();
    let storage = S3DdbStorage::new(backend, config);

    assert_eq!(storage.config().bucket, "mtc-log-1");
    assert_eq!(storage.config().table, "mtc-coordination");
    assert_eq!(storage.config().log_id, "log-1");
    assert_eq!(storage.config().retention_days, 2555);
    assert_eq!(storage.config().coordination_prefix(), "log#log-1");
}
