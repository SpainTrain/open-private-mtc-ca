//! The spec §11.4/§13.3 `Storage` facade: the CA service's single storage
//! seam, sitting above the four `cloud-types` capabilities via a
//! `cloud-backend::Backend` (ticket `mtc-f35`).
//!
//! | Type | Role | Spec |
//! |---|---|---|
//! | [`Storage`] | The facade trait: every storage operation the write path (§11.4) and promotion procedure (§13.3) call | §11.4, §13.3 |
//! | [`StorageError`] | Unified error taxonomy every `Storage` method returns | ticket AC |
//! | [`StorageConfig`] | `serde`-deserializable `S3DdbStorage` configuration | ticket AC |
//! | [`S3DdbStorage`] | The (eventual) S3 + `ReplicatedKv`-table implementation, wired from a `Backend` | §9.4 |
//! | [`Lease`], [`BatchState`], [`BatchStatus`] | Supporting domain types the trait's methods take or return | §8.2, §11.4, §13.3 |
//!
//! # Scope: trait surface only, every body `Unimplemented` (ticket `mtc-f35`)
//!
//! This ticket is the seam the per-pattern sub-tickets fill in: it declares
//! the full `Storage` method surface and the `StorageError` taxonomy, and
//! wires `S3DdbStorage`'s constructor from a `Backend`, but every method
//! currently returns [`StorageError::Unimplemented`] -- a typed error, never
//! a panic (rule `no-unwrap-in-prod`). Real bodies, the lease/epoch protocol,
//! and the cloud trait definitions themselves are out of scope here; see each
//! method's rustdoc for the spec step it will eventually implement.
//!
//! # Consumers never name a concrete backend (spec §9.3)
//!
//! Like `cloud-backend::Backend`, the CA service depends on `storage` +
//! `cloud-backend` + `mtc` and holds a `Storage` trait object -- never naming
//! `S3DdbStorage` directly outside the composition root that constructs it:
//!
//! ```
//! use std::sync::Arc;
//!
//! use cloud_backend::{build_backend, BackendConfig, Provider};
//! use mtc::{BatchId, Epoch};
//! use storage::{S3DdbStorage, Storage, StorageConfig, StorageError};
//!
//! // The §11.4 `CaService { storage: Arc<dyn Storage>, ... }` shape: a
//! // service holds a `Storage` trait object and never names `S3DdbStorage`.
//! struct CaService {
//!     storage: Arc<dyn Storage>,
//! }
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! let backend = Arc::new(
//!     build_backend(BackendConfig {
//!         provider: Provider::Memory,
//!     })
//!     .await
//!     .expect("the memory provider always builds"),
//! );
//! let config = StorageConfig {
//!     bucket: "mtc-log-1".to_string(),
//!     table: "mtc-coordination".to_string(),
//!     log_id: "log-1".to_string(),
//!     retention_days: 2555,
//! };
//! let service = CaService {
//!     storage: Arc::new(S3DdbStorage::new(backend, config)),
//! };
//!
//! // Every method is wired but, this ticket, returns the typed
//! // Unimplemented error rather than panicking -- sub-tickets fill each in.
//! let err = service
//!     .storage
//!     .allocate_indices(4, Epoch(0))
//!     .await
//!     .expect_err("allocate_indices has no body yet");
//! assert_eq!(
//!     err,
//!     StorageError::Unimplemented {
//!         method: "allocate_indices"
//!     },
//! );
//! # }
//! ```

#![warn(missing_docs)]

mod config;
mod error;
mod types;

use std::sync::Arc;

use async_trait::async_trait;
use cloud_backend::Backend;
use mtc::{BatchId, Checkpoint, Epoch, Index, LogEntry, Signed, Tile};

pub use config::StorageConfig;
pub use error::StorageError;
pub use types::{BatchState, BatchStatus, Lease};

/// The CA service's storage facade: every operation the write path (spec
/// §11.4) and the promotion procedure (spec §13.3) perform against durable
/// state.
///
/// Implementations sit above the four `cloud-types` capabilities (see
/// [`S3DdbStorage`]) so the CA service itself never calls `ObjectStore` /
/// `ObjectLock` / `ReplicatedKv` / `Hsm` directly -- it depends on `Arc<dyn
/// Storage>` the same way `cloud-backend::Backend`'s fields are `Arc<dyn
/// Trait>` (spec §9.4, §22.7: the sanctioned dynamic-dispatch seam for a
/// runtime-swappable backend).
#[async_trait]
pub trait Storage: Send + Sync {
    /// Reads the current primary-region lease (spec §8.2; write path step 1,
    /// §11.4: verify we hold it, capture its epoch).
    async fn read_lease(&self) -> Result<Lease, StorageError>;

    /// Atomically allocates `count` consecutive indices under `epoch`,
    /// returning the half-open range `[start, end)` (write path step 3,
    /// §11.4: counter `UpdateItem` with `ConditionExpression epoch = :epoch`).
    async fn allocate_indices(
        &self,
        count: usize,
        epoch: Epoch,
    ) -> Result<(Index, Index), StorageError>;

    /// Persists a batch's coordination-state record as `status` (write path
    /// step 4, §11.4: written as "pending" when the batch is first
    /// assembled).
    async fn persist_batch_state(
        &self,
        batch_id: &BatchId,
        start: Index,
        end: Index,
        status: BatchStatus,
        epoch: Epoch,
    ) -> Result<(), StorageError>;

    /// Writes `entries` at the consecutive indices starting at `start` (write
    /// path step 5, §11.4: N parallel `PutObject`s to
    /// `entries/.../NNNNNN.entry`).
    async fn write_entries(&self, start: Index, entries: &[LogEntry]) -> Result<(), StorageError>;

    /// Writes newly computed or updated tiles (write path step 6, §11.4: the
    /// tree update's affected interior nodes).
    async fn write_tiles(&self, tiles: &[Tile]) -> Result<(), StorageError>;

    /// Commits a signed checkpoint as the write path's linearization point
    /// (write path step 8, §11.4, §11.2): writes the checkpoint object and
    /// atomically updates the latest-checkpoint pointer and the batch's
    /// status to committed.
    async fn commit_checkpoint(
        &self,
        checkpoint: &Checkpoint<Signed>,
        batch_id: &BatchId,
        epoch: Epoch,
    ) -> Result<(), StorageError>;

    /// Reads the latest committed checkpoint (promotion step 1, §13.3:
    /// verifying the local view of log state before claiming the lease).
    async fn read_latest_checkpoint(&self) -> Result<Checkpoint<Signed>, StorageError>;

    /// Lists batches that are allocated but not yet committed or abandoned
    /// (promotion step 2, §13.3: identifying in-flight batches to abandon
    /// before claiming the lease).
    async fn query_pending_batches(&self) -> Result<Vec<BatchState>, StorageError>;

    /// Marks a pending batch abandoned; its indices become permanent
    /// `null_entry` gaps rather than being reused (promotion step 2, §13.3;
    /// write path invariant, §11.2).
    async fn mark_batch_abandoned(&self, batch: &BatchState) -> Result<(), StorageError>;
}

/// The (eventual) S3 + `ReplicatedKv`-table [`Storage`] implementation, wired
/// from a [`Backend`] (spec §9.4).
///
/// Despite the name, this struct never names a concrete provider -- it holds
/// the cloud-agnostic `Backend`'s trait objects, exactly like
/// `cloud_backend::build_backend`'s own callers (rule
/// `no-sdk-types-in-domain`). "`S3Ddb`" names the *shape* of storage this facade
/// targets (object storage + a replicated KV table, spec §9.1), not a
/// dependency on either SDK.
///
/// Every [`Storage`] method on this type currently returns
/// [`StorageError::Unimplemented`]; per-pattern sub-tickets fill each in
/// against `self.backend`'s four capabilities and `self.config`.
pub struct S3DdbStorage {
    backend: Arc<Backend>,
    config: StorageConfig,
}

impl S3DdbStorage {
    /// Creates a storage facade wired to `backend`'s four cloud capabilities,
    /// configured by `config` (spec §9.4).
    #[must_use]
    pub const fn new(backend: Arc<Backend>, config: StorageConfig) -> Self {
        Self { backend, config }
    }

    /// Borrows the wired [`Backend`].
    #[must_use]
    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// Borrows this facade's configuration.
    #[must_use]
    pub const fn config(&self) -> &StorageConfig {
        &self.config
    }
}

#[async_trait]
impl Storage for S3DdbStorage {
    async fn read_lease(&self) -> Result<Lease, StorageError> {
        Err(StorageError::Unimplemented {
            method: "read_lease",
        })
    }

    async fn allocate_indices(
        &self,
        _count: usize,
        _epoch: Epoch,
    ) -> Result<(Index, Index), StorageError> {
        Err(StorageError::Unimplemented {
            method: "allocate_indices",
        })
    }

    async fn persist_batch_state(
        &self,
        _batch_id: &BatchId,
        _start: Index,
        _end: Index,
        _status: BatchStatus,
        _epoch: Epoch,
    ) -> Result<(), StorageError> {
        Err(StorageError::Unimplemented {
            method: "persist_batch_state",
        })
    }

    async fn write_entries(
        &self,
        _start: Index,
        _entries: &[LogEntry],
    ) -> Result<(), StorageError> {
        Err(StorageError::Unimplemented {
            method: "write_entries",
        })
    }

    async fn write_tiles(&self, _tiles: &[Tile]) -> Result<(), StorageError> {
        Err(StorageError::Unimplemented {
            method: "write_tiles",
        })
    }

    async fn commit_checkpoint(
        &self,
        _checkpoint: &Checkpoint<Signed>,
        _batch_id: &BatchId,
        _epoch: Epoch,
    ) -> Result<(), StorageError> {
        Err(StorageError::Unimplemented {
            method: "commit_checkpoint",
        })
    }

    async fn read_latest_checkpoint(&self) -> Result<Checkpoint<Signed>, StorageError> {
        Err(StorageError::Unimplemented {
            method: "read_latest_checkpoint",
        })
    }

    async fn query_pending_batches(&self) -> Result<Vec<BatchState>, StorageError> {
        Err(StorageError::Unimplemented {
            method: "query_pending_batches",
        })
    }

    async fn mark_batch_abandoned(&self, _batch: &BatchState) -> Result<(), StorageError> {
        Err(StorageError::Unimplemented {
            method: "mark_batch_abandoned",
        })
    }
}
