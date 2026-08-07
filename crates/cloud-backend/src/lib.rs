//! The spec §9.4 `Backend` factory: wires the four `cloud-types` capability
//! traits into one struct from runtime configuration (ticket
//! `cloud-backend-factory`).
//!
//! | Type | Role | Spec |
//! |---|---|---|
//! | [`Backend`] | Struct of four `Arc<dyn Trait>` handles | §9.4 |
//! | [`Provider`] | Which concrete backend to wire (`Memory`/`Aws`/`Localstack`) | §9.4 |
//! | [`BackendConfig`] | `serde`-deserializable factory input | §9.4 |
//! | [`build_backend`] | `BackendConfig -> Result<Backend, BackendError>` | §9.4 |
//!
//! # Provider status
//!
//! [`Provider::Memory`] is fully wired against `cloud-memory`: zero external
//! dependencies, no Docker, no `LocalStack` (spec §9.6). [`Provider::Aws`] and
//! [`Provider::Localstack`] parse out of [`BackendConfig`] -- config
//! validation does not depend on which providers happen to be implemented --
//! but [`build_backend`] returns [`BackendError::Unimplemented`] for both
//! until `cloud-backend-factory-aws` wires `cloud-aws` / `cloud-localstack` +
//! `cloud-softhsm` in.
//!
//! # Cargo feature gating (spec §9.3 "wired via Cargo features")
//!
//! The `aws` feature gates this crate's (currently unused) optional
//! `cloud-aws` dependency, so a plain `cargo build -p cloud-backend` never
//! compiles the AWS SDK -- only `cloud-memory` and its zero-external-
//! dependency graph. `cloud-backend-factory-aws` implements
//! [`Provider::Aws`]'s real wiring behind this same feature.
//!
//! # Consumers never name a concrete backend (spec §9.3)
//!
//! Services depend on `cloud-backend` + `cloud-types` only:
//!
//! ```
//! use std::sync::Arc;
//!
//! use cloud_backend::{build_backend, Backend, BackendConfig, Provider};
//!
//! // The §9.4 `CaService::new(Arc<Backend>)` shape: a service holds a
//! // `Backend` and never names `cloud-memory`, `cloud-aws`, or any other
//! // concrete backend crate.
//! struct CaService {
//!     backend: Arc<Backend>,
//! }
//!
//! impl CaService {
//!     fn new(backend: Arc<Backend>) -> Self {
//!         Self { backend }
//!     }
//! }
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! let cfg = BackendConfig {
//!     provider: Provider::Memory,
//! };
//! let backend = Arc::new(
//!     build_backend(cfg)
//!         .await
//!         .expect("the memory provider always builds"),
//! );
//! let service = CaService::new(backend);
//!
//! // The service can now use any of the four capabilities through its
//! // Backend, e.g. writing a log entry:
//! service
//!     .backend
//!     .object_store
//!     .put(
//!         "entries/0001",
//!         b"leaf",
//!         cloud_types::PutOptions::default(),
//!     )
//!     .await
//!     .expect("memory object_store accepts the write");
//! # }
//! ```

#![warn(missing_docs)]

mod config;
mod error;
mod factory;

use std::sync::Arc;

use cloud_types::{Hsm, ObjectLock, ObjectStore, ReplicatedKv};

pub use config::{BackendConfig, ConfigError, Provider};
pub use error::BackendError;
pub use factory::build_backend;

/// The four cloud capabilities the CA service depends on, wired to one
/// concrete backend at startup (spec §9.4).
///
/// Built exclusively by [`build_backend`]; consumers depend on this struct
/// and `cloud-types`, never on a concrete backend crate (spec §9.3).
pub struct Backend {
    /// Durable, immutable object storage (spec §9.1).
    pub object_store: Arc<dyn ObjectStore>,
    /// Storage-layer retention locking (spec §9.1).
    pub object_lock: Arc<dyn ObjectLock>,
    /// Replicated KV with conditional + transactional writes (spec §9.1).
    pub replicated_kv: Arc<dyn ReplicatedKv>,
    /// Hardware-backed signing (spec §9.1).
    pub hsm: Arc<dyn Hsm>,
}
