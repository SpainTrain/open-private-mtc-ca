//! [`MemoryObjectStore`] — pure in-memory [`ObjectStore`] (spec §9.3, §9.6).
//!
//! Backed by a `Mutex`-guarded `BTreeMap`: no external dependencies, no
//! Docker, no `LocalStack`. The same type also implements
//! [`ObjectLock`](cloud_types::ObjectLock) under the
//! [`MemoryObjectLock`](crate::MemoryObjectLock) alias — see
//! `object_lock.rs` and the crate-level docs for why the two capabilities
//! deliberately share one struct and one map.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use async_trait::async_trait;
use clock::Clock;
use cloud_types::{CloudError, ObjectInfo, ObjectMetadata, ObjectStore, PutMode, PutOptions};

/// One stored object plus its optional retention instant
/// ([`ObjectLock::put_with_retention`](cloud_types::ObjectLock::put_with_retention)).
///
/// Crate-visible (not exported from `lib.rs`) — `object_lock.rs` reads and
/// writes this directly since it operates on the same map. Declared `pub`
/// rather than `pub(crate)`: the containing module is itself private, so the
/// two are equivalent and `pub` is what `clippy::redundant_pub_crate` wants.
#[derive(Debug, Clone)]
pub struct StoredObject {
    pub(crate) data: Vec<u8>,
    pub(crate) last_modified: SystemTime,
    pub(crate) retain_until: Option<SystemTime>,
}

/// Object size as `u64`, saturating rather than panicking on platforms where
/// `usize` could theoretically exceed it (never happens in practice, but
/// `as u64` is a banned lossy cast under `clippy::pedantic`).
pub fn size_bytes(data: &[u8]) -> u64 {
    u64::try_from(data.len()).unwrap_or(u64::MAX)
}

/// Pure in-memory, process-local [`ObjectStore`] — and, via the same type,
/// [`ObjectLock`](cloud_types::ObjectLock) (see the crate-level docs).
///
/// Cheap to [`Clone`]: internal state is `Arc`-shared, so every clone
/// observes the same objects — the same sharing pattern
/// `Arc<dyn ObjectStore>` / `Arc<dyn ObjectLock>` need from the `Backend`
/// factory (spec §9.4), which can wrap two clones of one store.
#[derive(Clone)]
pub struct MemoryObjectStore {
    pub(crate) objects: Arc<Mutex<BTreeMap<String, StoredObject>>>,
    pub(crate) clock: Arc<dyn Clock>,
}

impl MemoryObjectStore {
    /// Creates an empty store that reads retention/timestamp instants from
    /// `clock`.
    ///
    /// Inject [`clock::FakeClock`] in tests (deterministic retention-window
    /// assertions) and `clock::SystemClock` in production wiring — production
    /// code never calls `SystemTime::now()` directly (rule
    /// `no-systemtime-now-in-prod`, spec §22.11).
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            objects: Arc::new(Mutex::new(BTreeMap::new())),
            clock,
        }
    }

    /// Locks the object map, recovering from poisoning rather than
    /// panicking: a prior panicked task must not wedge every subsequent
    /// operation in a pure in-memory test/dev backend (rule
    /// `no-unwrap-in-prod` — this uses `unwrap_or_else`, not `unwrap`, and is
    /// the standard recovery idiom for `std::sync::Mutex`).
    pub(crate) fn lock_objects(&self) -> MutexGuard<'_, BTreeMap<String, StoredObject>> {
        self.objects.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Shared by [`ObjectStore::delete`] and [`ObjectStore::put`] (under
/// [`PutMode::Overwrite`]): both must refuse to touch a retained object
/// (spec §9.5 "cannot delete during retention window even by admins").
pub fn reject_if_retained(
    existing: &StoredObject,
    clock: &dyn Clock,
    key: &str,
) -> Result<(), CloudError> {
    if let Some(retain_until) = existing.retain_until {
        if clock.now() < retain_until {
            return Err(CloudError::RetentionViolation {
                reason: format!("{key} is retained until {retain_until:?}"),
            });
        }
    }
    Ok(())
}

#[async_trait]
impl ObjectStore for MemoryObjectStore {
    async fn put(&self, key: &str, data: &[u8], opts: PutOptions) -> Result<(), CloudError> {
        let mut objects = self.lock_objects();
        match opts.mode {
            PutMode::IfNotExists => {
                if objects.contains_key(key) {
                    return Err(CloudError::AlreadyExists {
                        key: key.to_string(),
                    });
                }
            }
            PutMode::Overwrite => {
                if let Some(existing) = objects.get(key) {
                    reject_if_retained(existing, self.clock.as_ref(), key)?;
                }
            }
        }
        objects.insert(
            key.to_string(),
            StoredObject {
                data: data.to_vec(),
                last_modified: self.clock.now(),
                // ObjectStore::put never sets retention; only
                // ObjectLock::put_with_retention does.
                retain_until: None,
            },
        );
        drop(objects);
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        self.lock_objects()
            .get(key)
            .map(|object| object.data.clone())
            .ok_or_else(|| CloudError::NotFound {
                key: key.to_string(),
            })
    }

    async fn head(&self, key: &str) -> Result<ObjectMetadata, CloudError> {
        self.lock_objects()
            .get(key)
            .map(|object| ObjectMetadata {
                size_bytes: size_bytes(&object.data),
                last_modified: object.last_modified,
            })
            .ok_or_else(|| CloudError::NotFound {
                key: key.to_string(),
            })
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectInfo>, CloudError> {
        Ok(self
            .lock_objects()
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(key, object)| ObjectInfo {
                key: key.clone(),
                size_bytes: size_bytes(&object.data),
                last_modified: object.last_modified,
            })
            .collect())
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        let mut objects = self.lock_objects();
        let existing = objects.get(key).ok_or_else(|| CloudError::NotFound {
            key: key.to_string(),
        })?;
        reject_if_retained(existing, self.clock.as_ref(), key)?;
        objects.remove(key);
        drop(objects);
        Ok(())
    }
}

// Contract-level tests (put/get round-trip, if-not-exists, head, list,
// delete, NotFound semantics) moved to the shared cloud-test-suite
// ObjectStore conformance suite (spec §9.7), wired against this backend in
// `crates/cloud-memory/tests/object_store_suite.rs` -- see
// `docs/journal.md` (cloud-test-suite-object entry).
