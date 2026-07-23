//! [`ObjectLock`] — storage-layer retention locking (spec §9.1).
//!
//! Backends: S3 Object Lock (Compliance mode) / GCS Object Retention / Azure
//! Immutable Storage / in-memory emulation.
//!
//! Retention locking is what turns "append-only by convention" into
//! "append-only at the storage layer" (spec §8, §15.3): while an object's
//! retention window is active, nothing — including administrators and
//! compromised CA credentials — can delete or replace it. The §9.5 capability
//! this trait models is **object retention lock**: "cannot delete during
//! retention window even by admins".

use std::time::SystemTime;

use async_trait::async_trait;

use crate::errors::CloudError;

/// Storage-layer retention locking for append-only enforcement.
///
/// Object-safe and shared as `Arc<dyn ObjectLock>` from the `Backend` factory
/// (spec §9.4); `Send + Sync` supertraits allow concurrent use across tasks.
///
/// Keys share the namespace and conventions of
/// [`ObjectStore`](crate::ObjectStore): opaque UTF-8 paths, no vendor SDK
/// types in any signature (spec §22.8). Retention instants are wall-clock
/// [`SystemTime`] values supplied by the caller (production code obtains time
/// via its injected `Clock`, never `SystemTime::now()` — spec §22.11).
#[async_trait]
pub trait ObjectLock: Send + Sync {
    /// Writes `data` at `key` with a retention lock active until
    /// `retain_until`.
    ///
    /// The write is create-only: retained objects are immutable, so replacing
    /// an existing object through this path is never valid. Compliance-mode
    /// semantics apply — once stored, the object cannot be deleted or
    /// overwritten by any principal until `retain_until` has passed.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// Object retention lock: cannot delete during the retention window even
    /// by admins (true append-only at the storage layer). Also inherits the
    /// object durability and immutability bars of
    /// [`ObjectStore::put`](crate::ObjectStore::put). Backends whose retention
    /// is advisory (removable by an administrator) do not meet the bar.
    ///
    /// # Errors
    ///
    /// - [`CloudError::AlreadyExists`] — an object already exists at `key`.
    /// - [`CloudError::Transport`] — transport/service failure (see
    ///   `retryable`).
    async fn put_with_retention(
        &self,
        key: &str,
        data: &[u8],
        retain_until: SystemTime,
    ) -> Result<(), CloudError>;

    /// Extends the retention window of the object at `key` to
    /// `new_retain_until`.
    ///
    /// Retention is forward-only: `new_retain_until` must be later than the
    /// currently stored retention instant. Shortening or clearing retention is
    /// a [`CloudError::RetentionViolation`] — a lock that can be relaxed is
    /// not a lock (spec §9.5).
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// Object retention lock: the window may only ever grow; no principal can
    /// weaken an existing lock.
    ///
    /// # Errors
    ///
    /// - [`CloudError::NotFound`] — no object at `key`.
    /// - [`CloudError::RetentionViolation`] — `new_retain_until` does not
    ///   extend the existing window.
    /// - [`CloudError::Transport`] — transport/service failure.
    async fn extend_retention(
        &self,
        key: &str,
        new_retain_until: SystemTime,
    ) -> Result<(), CloudError>;

    /// Returns the instant until which the object at `key` is retained.
    ///
    /// Used by pruning (spec §15) to confirm retention has expired before
    /// lifecycle deletion, and by compliance reporting.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// Object retention lock: the reported instant is the authoritative
    /// storage-layer lock expiry, not an application-level annotation.
    ///
    /// # Errors
    ///
    /// - [`CloudError::NotFound`] — no object at `key` (or the object carries
    ///   no retention lock).
    /// - [`CloudError::Transport`] — transport/service failure.
    async fn get_retention(&self, key: &str) -> Result<SystemTime, CloudError>;
}
