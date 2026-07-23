//! [`ObjectStore`] — durable, immutable object storage (spec §9.1).
//!
//! Backends: S3 / GCS / Azure Blob / MinIO / on-prem / pure memory.
//!
//! The store holds the certificate transparency log's append-only artifacts
//! (entries, tiles, checkpoints — spec §8). Two §9.5 capabilities anchor every
//! method contract here:
//!
//! - **Object durability** — eleven 9s practical durability (log integrity).
//! - **Object immutability** — bytes never change after write (append-only
//!   invariant).

use std::time::SystemTime;

use async_trait::async_trait;

use crate::errors::CloudError;

/// Overwrite policy for [`ObjectStore::put`].
///
/// The default is [`PutMode::IfNotExists`]: the log is append-only (spec §8),
/// so no-overwrite is the safe default and unconditional overwrite is the
/// explicit opt-in for the few non-log writes that need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PutMode {
    /// Fail with [`CloudError::AlreadyExists`] when the key already holds an
    /// object. Upholds the append-only invariant (spec §8, §9.7
    /// `test_overwrite_fails`); maps to `If-None-Match: *` on S3.
    #[default]
    IfNotExists,
    /// Unconditionally replace any existing object at the key.
    Overwrite,
}

/// Options for [`ObjectStore::put`].
///
/// ```
/// use cloud_types::{PutMode, PutOptions};
///
/// // The default upholds the append-only invariant.
/// assert_eq!(PutOptions::default().mode, PutMode::IfNotExists);
/// assert_eq!(PutOptions::if_not_exists(), PutOptions::default());
/// assert_eq!(PutOptions::overwrite().mode, PutMode::Overwrite);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PutOptions {
    /// Overwrite policy applied by the backend.
    pub mode: PutMode,
}

impl PutOptions {
    /// Options requiring the key to be vacant ([`PutMode::IfNotExists`]).
    #[must_use]
    pub const fn if_not_exists() -> Self {
        Self {
            mode: PutMode::IfNotExists,
        }
    }

    /// Options permitting unconditional replacement ([`PutMode::Overwrite`]).
    #[must_use]
    pub const fn overwrite() -> Self {
        Self {
            mode: PutMode::Overwrite,
        }
    }
}

/// Metadata for a single stored object, returned by [`ObjectStore::head`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectMetadata {
    /// Object size in bytes.
    pub size_bytes: u64,
    /// When the object was written (backend-reported).
    pub last_modified: SystemTime,
}

/// A listing entry returned by [`ObjectStore::list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    /// Full object key.
    pub key: String,
    /// Object size in bytes.
    pub size_bytes: u64,
    /// When the object was written (backend-reported).
    pub last_modified: SystemTime,
}

/// Durable, immutable object storage.
///
/// Object-safe and shared as `Arc<dyn ObjectStore>` — one backend is selected
/// at startup by the `Backend` factory (spec §9.4) and used concurrently from
/// many tasks; the `Send + Sync` supertraits make that sharing sound.
///
/// Keys are opaque UTF-8 paths with `/`-separated segments; prefix semantics
/// in [`ObjectStore::list`] are plain string-prefix matching. Vendor SDK types
/// never appear in these signatures (spec §22.8).
#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Writes `data` at `key`, subject to the overwrite policy in `opts`.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// Object durability (eleven 9s practical — log integrity) and object
    /// immutability (bytes never change after write — append-only invariant).
    /// Under [`PutMode::IfNotExists`] the backend must reject the write
    /// atomically when the key is occupied; best-effort read-then-write
    /// emulation does not meet the bar.
    ///
    /// # Errors
    ///
    /// - [`CloudError::AlreadyExists`] — key occupied and `opts.mode` is
    ///   [`PutMode::IfNotExists`].
    /// - [`CloudError::RetentionViolation`] — overwrite attempted on an object
    ///   under an active retention lock.
    /// - [`CloudError::Transport`] — transport/service failure (see
    ///   `retryable`).
    async fn put(&self, key: &str, data: &[u8], opts: PutOptions) -> Result<(), CloudError>;

    /// Reads the full contents of the object at `key`.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// Object durability: reads observe exactly the immutable bytes that were
    /// written — a successful `get` after a successful `put` returns identical
    /// content, in-region. (Cross-region replicas may lag within the bounded
    /// replication window.)
    ///
    /// # Errors
    ///
    /// - [`CloudError::NotFound`] — no object at `key`.
    /// - [`CloudError::Transport`] — transport/service failure.
    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError>;

    /// Fetches metadata for the object at `key` without reading its bytes.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// Object durability: metadata reflects the immutable stored object;
    /// `size_bytes` matches the length `get` would return.
    ///
    /// # Errors
    ///
    /// - [`CloudError::NotFound`] — no object at `key`.
    /// - [`CloudError::Transport`] — transport/service failure.
    async fn head(&self, key: &str) -> Result<ObjectMetadata, CloudError>;

    /// Lists every object whose key starts with `prefix`.
    ///
    /// Returns entries sorted by key, ascending. Backends with paginated APIs
    /// must drain pagination internally; callers see one complete listing.
    /// An empty result is `Ok(vec![])`, not an error.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// Object durability: a listing must include every durably committed
    /// object under the prefix in this region.
    ///
    /// # Errors
    ///
    /// - [`CloudError::Transport`] — transport/service failure.
    async fn list(&self, prefix: &str) -> Result<Vec<ObjectInfo>, CloudError>;

    /// Deletes the object at `key`.
    ///
    /// Deletion is for lifecycle pruning only (spec §15) — never a mutation
    /// path for log content. Pruning of retained objects goes through
    /// [`ObjectLock`](crate::ObjectLock) retention expiry first.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// Object retention lock: deletion of an object inside its retention
    /// window must fail — "cannot delete during retention window even by
    /// admins".
    ///
    /// # Errors
    ///
    /// - [`CloudError::NotFound`] — no object at `key`.
    /// - [`CloudError::RetentionViolation`] — object is inside an active
    ///   retention window.
    /// - [`CloudError::Transport`] — transport/service failure.
    async fn delete(&self, key: &str) -> Result<(), CloudError>;
}
