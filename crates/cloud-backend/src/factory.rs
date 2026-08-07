//! [`build_backend`]: the spec §9.4 factory function.

use std::sync::Arc;

use clock::SystemClock;
use cloud_memory::{MemoryHsm, MemoryObjectStore, MemoryReplicatedKv};

use crate::config::{BackendConfig, Provider};
use crate::error::BackendError;
use crate::Backend;

/// Builds a [`Backend`] per `cfg.provider` (spec §9.4).
///
/// # Errors
///
/// [`BackendError::Unimplemented`] for [`Provider::Aws`] /
/// [`Provider::Localstack`] until `cloud-backend-factory-aws` lands.
pub async fn build_backend(cfg: BackendConfig) -> Result<Backend, BackendError> {
    match cfg.provider {
        Provider::Memory => Ok(build_memory_backend()),
        Provider::Aws | Provider::Localstack => Err(BackendError::Unimplemented {
            provider: cfg.provider,
        }),
    }
}

/// The all-memory [`Backend`] (spec §9.6: zero external dependencies).
///
/// `object_store` and `object_lock` wrap two clones of one
/// [`MemoryObjectStore`] rather than two independent stores:
/// `cloud-memory`'s crate docs call this out explicitly as the pattern the
/// `Backend` factory needs, since [`ObjectStore::delete`](cloud_types::ObjectStore::delete)
/// and [`ObjectLock::put_with_retention`](cloud_types::ObjectLock::put_with_retention)
/// have to agree on what is currently retained. Production backends get this
/// coupling for free from one S3 bucket with Object Lock enabled; the memory
/// backend constructs it explicitly here.
fn build_memory_backend() -> Backend {
    let store = MemoryObjectStore::new(Arc::new(SystemClock));
    Backend {
        object_store: Arc::new(store.clone()),
        object_lock: Arc::new(store),
        replicated_kv: Arc::new(MemoryReplicatedKv::new()),
        hsm: Arc::new(MemoryHsm::new()),
    }
}
