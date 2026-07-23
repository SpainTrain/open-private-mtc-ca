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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use clock::{Clock, FakeClock};
    use cloud_types::{ObjectStore, PutOptions};
    use pretty_assertions::assert_eq;

    use super::*;

    fn store_with_clock() -> (MemoryObjectStore, Arc<FakeClock>) {
        let clock = Arc::new(FakeClock::default());
        (MemoryObjectStore::new(clock.clone()), clock)
    }

    #[tokio::test]
    async fn put_with_retention_then_get_retention_round_trips() {
        let (store, clock) = store_with_clock();
        let retain_until = clock.now() + Duration::from_hours(1);
        store
            .put_with_retention("checkpoints/0001", b"cp", retain_until)
            .await
            .expect("put_with_retention succeeds");
        assert_eq!(
            store.get_retention("checkpoints/0001").await.expect("get"),
            retain_until
        );
        assert_eq!(store.get("checkpoints/0001").await.expect("get"), b"cp");
    }

    #[tokio::test]
    async fn put_with_retention_is_create_only() {
        let (store, clock) = store_with_clock();
        let retain_until = clock.now() + Duration::from_hours(1);
        store
            .put_with_retention("checkpoints/0001", b"cp", retain_until)
            .await
            .expect("first put succeeds");
        let err = store
            .put_with_retention("checkpoints/0001", b"cp2", retain_until)
            .await
            .expect_err("second put must fail");
        assert!(matches!(err, CloudError::AlreadyExists { .. }));
    }

    #[tokio::test]
    async fn get_retention_missing_object_is_not_found() {
        let (store, _clock) = store_with_clock();
        assert!(matches!(
            store.get_retention("missing").await,
            Err(CloudError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn get_retention_on_unlocked_object_is_not_found() {
        let (store, _clock) = store_with_clock();
        store
            .put("plain/0001", b"x", PutOptions::default())
            .await
            .expect("plain put");
        assert!(matches!(
            store.get_retention("plain/0001").await,
            Err(CloudError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn extend_retention_forward_succeeds() {
        let (store, clock) = store_with_clock();
        let first = clock.now() + Duration::from_hours(1);
        let second = clock.now() + Duration::from_hours(2);
        store
            .put_with_retention("checkpoints/0001", b"cp", first)
            .await
            .expect("put");
        store
            .extend_retention("checkpoints/0001", second)
            .await
            .expect("extend succeeds");
        assert_eq!(
            store.get_retention("checkpoints/0001").await.expect("get"),
            second
        );
    }

    #[tokio::test]
    async fn extend_retention_rejects_shortening() {
        let (store, clock) = store_with_clock();
        let first = clock.now() + Duration::from_hours(2);
        let shorter = clock.now() + Duration::from_hours(1);
        store
            .put_with_retention("checkpoints/0001", b"cp", first)
            .await
            .expect("put");
        let err = store
            .extend_retention("checkpoints/0001", shorter)
            .await
            .expect_err("shortening must fail");
        assert!(matches!(err, CloudError::RetentionViolation { .. }));
        // Unchanged.
        assert_eq!(
            store.get_retention("checkpoints/0001").await.expect("get"),
            first
        );
    }

    #[tokio::test]
    async fn extend_retention_rejects_equal_instant() {
        let (store, clock) = store_with_clock();
        let retain_until = clock.now() + Duration::from_hours(1);
        store
            .put_with_retention("checkpoints/0001", b"cp", retain_until)
            .await
            .expect("put");
        let err = store
            .extend_retention("checkpoints/0001", retain_until)
            .await
            .expect_err("no-op extend must fail");
        assert!(matches!(err, CloudError::RetentionViolation { .. }));
    }

    #[tokio::test]
    async fn extend_retention_missing_object_is_not_found() {
        let (store, clock) = store_with_clock();
        assert!(matches!(
            store
                .extend_retention("missing", clock.now() + Duration::from_hours(1))
                .await,
            Err(CloudError::NotFound { .. })
        ));
    }

    // --- Cross-trait: ObjectStore::delete / put must honor ObjectLock
    // retention because both traits share this store's map (spec §9.5). ---

    #[tokio::test]
    async fn delete_during_retention_window_is_rejected() {
        let (store, clock) = store_with_clock();
        let retain_until = clock.now() + Duration::from_hours(1);
        store
            .put_with_retention("checkpoints/0001", b"cp", retain_until)
            .await
            .expect("put");
        let err = store
            .delete("checkpoints/0001")
            .await
            .expect_err("delete during retention must fail");
        assert!(matches!(err, CloudError::RetentionViolation { .. }));
        // Still there.
        assert_eq!(store.get("checkpoints/0001").await.expect("get"), b"cp");
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

    #[tokio::test]
    async fn overwrite_during_retention_window_is_rejected() {
        let (store, clock) = store_with_clock();
        let retain_until = clock.now() + Duration::from_hours(1);
        store
            .put_with_retention("checkpoints/0001", b"cp", retain_until)
            .await
            .expect("put");
        let err = store
            .put("checkpoints/0001", b"tampered", PutOptions::overwrite())
            .await
            .expect_err("overwrite during retention must fail");
        assert!(matches!(err, CloudError::RetentionViolation { .. }));
        assert_eq!(store.get("checkpoints/0001").await.expect("get"), b"cp");
    }

    #[tokio::test]
    async fn if_not_exists_put_over_retained_key_is_already_exists() {
        // AlreadyExists is the primary contract of IfNotExists — it fires
        // before any retention check, regardless of whether the occupant
        // happens to be retained.
        let (store, clock) = store_with_clock();
        let retain_until = clock.now() + Duration::from_hours(1);
        store
            .put_with_retention("checkpoints/0001", b"cp", retain_until)
            .await
            .expect("put");
        let err = store
            .put("checkpoints/0001", b"tampered", PutOptions::if_not_exists())
            .await
            .expect_err("if-not-exists put over occupied key must fail");
        assert!(matches!(err, CloudError::AlreadyExists { .. }));
    }
}
