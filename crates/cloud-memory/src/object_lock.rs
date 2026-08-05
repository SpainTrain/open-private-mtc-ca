//! [`ObjectLock`] impl for [`MemoryObjectStore`] (spec §9.3, §9.5).
//!
//! Retention windows are enforced against the injected `Arc<dyn Clock>`
//! (spec §22.11, §18.4 time-travel pattern): a retention instant is "active"
//! exactly when `clock.now() < retain_until`. This impl lives on
//! [`MemoryObjectStore`] itself rather than a separate struct — see the
//! crate-level docs and `object_store.rs` for why the two capabilities share
//! one map: [`ObjectStore::delete`](cloud_types::ObjectStore::delete) has to
//! see the same retention state this trait writes.

use async_trait::async_trait;
use cloud_types::{CloudError, ObjectLock};
use std::time::SystemTime;

use crate::object_store::{MemoryObjectStore, StoredObject};

/// [`MemoryObjectStore`] also implements [`ObjectLock`].
///
/// This alias names it by the capability it provides at construction sites
/// (e.g. wiring a `Backend`'s `object_lock: Arc<dyn ObjectLock>` field —
/// spec §9.4).
pub type MemoryObjectLock = MemoryObjectStore;

#[async_trait]
impl ObjectLock for MemoryObjectStore {
    async fn put_with_retention(
        &self,
        key: &str,
        data: &[u8],
        retain_until: SystemTime,
    ) -> Result<(), CloudError> {
        let mut objects = self.lock_objects();
        // Create-only: retained objects are immutable (cloud-types
        // rustdoc — replacing an existing object through this path is never
        // valid).
        if objects.contains_key(key) {
            return Err(CloudError::AlreadyExists {
                key: key.to_string(),
            });
        }
        objects.insert(
            key.to_string(),
            StoredObject {
                data: data.to_vec(),
                last_modified: self.clock.now(),
                retain_until: Some(retain_until),
            },
        );
        drop(objects);
        Ok(())
    }

    async fn extend_retention(
        &self,
        key: &str,
        new_retain_until: SystemTime,
    ) -> Result<(), CloudError> {
        let mut objects = self.lock_objects();
        let existing = objects.get_mut(key).ok_or_else(|| CloudError::NotFound {
            key: key.to_string(),
        })?;
        // An object with no retention lock at all is "not found" from
        // ObjectLock's point of view, matching get_retention below — there is
        // no window to extend.
        let current = existing.retain_until.ok_or_else(|| CloudError::NotFound {
            key: key.to_string(),
        })?;
        if new_retain_until <= current {
            return Err(CloudError::RetentionViolation {
                reason: format!(
                    "{key}: retention is forward-only (current {current:?}, requested {new_retain_until:?})"
                ),
            });
        }
        existing.retain_until = Some(new_retain_until);
        drop(objects);
        Ok(())
    }

    async fn get_retention(&self, key: &str) -> Result<SystemTime, CloudError> {
        let objects = self.lock_objects();
        let existing = objects.get(key).ok_or_else(|| CloudError::NotFound {
            key: key.to_string(),
        })?;
        let retain_until = existing.retain_until;
        drop(objects);
        retain_until.ok_or_else(|| CloudError::NotFound {
            key: key.to_string(),
        })
    }
}

// Contract-level tests (round-trip, create-only, forward-only extend,
// retention-vs-delete/overwrite/put interaction, NotFound semantics) moved to
// the shared cloud-test-suite ObjectLock conformance suite (spec §9.7),
// wired against this backend in
// `crates/cloud-memory/tests/object_lock_suite.rs` -- see `docs/journal.md`
// (cloud-test-suite-object entry). One case stays here: proving retention
// *expiry* actually unblocks delete requires fast-forwarding wall-clock
// time, a capability only a fake/injectable-clock backend has, so it is not
// part of the cross-backend contract the shared suite enforces.
#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use clock::{Clock, FakeClock};
    use cloud_types::ObjectStore;

    use super::*;

    fn store_with_clock() -> (MemoryObjectStore, Arc<FakeClock>) {
        let clock = Arc::new(FakeClock::default());
        (MemoryObjectStore::new(clock.clone()), clock)
    }

    #[tokio::test]
    async fn delete_after_retention_expires_succeeds() {
        let (store, clock) = store_with_clock();
        let retain_until = clock.now() + Duration::from_hours(1);
        store
            .put_with_retention("checkpoints/0001", b"cp", retain_until)
            .await
            .expect("put");
        clock.advance(Duration::from_hours(2));
        store
            .delete("checkpoints/0001")
            .await
            .expect("delete after expiry succeeds");
    }
}
