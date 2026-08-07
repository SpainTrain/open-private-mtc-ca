//! [`StorageError`]: the [`Storage`](crate::Storage) trait's unified error
//! taxonomy (ticket `mtc-f35` AC; rule `thiserror-for-libs-eyre-for-bins`).

use cloud_types::CloudError;

/// Failure returned by any [`Storage`](crate::Storage) method.
///
/// Deliberately exhaustive (no `#[non_exhaustive]`), mirroring
/// [`CloudError`]'s rationale (spec §22.3: exhaustive matching is the
/// language default): adding a variant is a conscious, breaking change every
/// caller must handle explicitly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    /// The write path no longer holds the primary-region lease (spec §11.3
    /// step 3 failure mode: "Lost lease mid-batch" -- stand down, batch
    /// abandoned).
    #[error("lost the primary-region lease")]
    LostLease,

    /// The target already exists and the operation required that it did not
    /// (the storage-level face of the append-only invariant, spec §8, §11.2).
    #[error("{resource} already exists")]
    AlreadyExists {
        /// Description of the resource that already existed (e.g. an entry
        /// key, a checkpoint, a batch id).
        resource: String,
    },

    /// The requested resource does not exist.
    #[error("{resource} not found")]
    NotFound {
        /// Description of the resource that was not found.
        resource: String,
    },

    /// The operation is not valid for the resource's current state (e.g.
    /// abandoning a batch that is not pending).
    #[error("invalid state: {reason}")]
    InvalidState {
        /// Description of the invalid transition or precondition.
        reason: String,
    },

    /// This [`Storage`](crate::Storage) method has no implementation yet.
    ///
    /// Every method returns this variant until its per-pattern sub-ticket
    /// lands (ticket `mtc-f35` scope: trait surface + error taxonomy +
    /// constructor only, never a panic in place of a real body -- rule
    /// `no-unwrap-in-prod`).
    #[error("{method} is not implemented yet")]
    Unimplemented {
        /// The `Storage` trait method name that was called.
        method: &'static str,
    },

    /// A cloud-capability call ([`ObjectStore`](cloud_types::ObjectStore),
    /// [`ObjectLock`](cloud_types::ObjectLock),
    /// [`ReplicatedKv`](cloud_types::ReplicatedKv),
    /// [`Hsm`](cloud_types::Hsm)) failed.
    #[error("backend error: {0}")]
    Backend(#[from] CloudError),
}

#[cfg(test)]
mod tests {
    use super::StorageError;
    use cloud_types::CloudError;

    #[test]
    fn cloud_error_converts_into_the_backend_variant() {
        let cloud_err = CloudError::ConditionFailed {
            reason: "epoch mismatch".to_string(),
        };
        let storage_err: StorageError = cloud_err.clone().into();
        assert_eq!(storage_err, StorageError::Backend(cloud_err));
    }

    #[test]
    fn question_mark_propagates_a_cloud_error_as_backend() {
        // The `#[from]` conversion is what lets a future implementation
        // write `self.backend.object_store.get(key).await?` directly inside
        // a `Result<_, StorageError>`-returning method.
        fn returns_cloud_error() -> Result<(), CloudError> {
            Err(CloudError::NotFound {
                key: "entries/0001".to_string(),
            })
        }
        fn propagates() -> Result<(), StorageError> {
            returns_cloud_error()?;
            Ok(())
        }
        assert_eq!(
            propagates(),
            Err(StorageError::Backend(CloudError::NotFound {
                key: "entries/0001".to_string(),
            })),
        );
    }

    #[test]
    fn error_trait_is_implemented() {
        // thiserror derives std::error::Error; keep it that way so callers
        // can box/chain these through generic error-handling layers.
        fn assert_error<E: std::error::Error + Send + Sync + 'static>(_: &E) {}
        assert_error(&StorageError::LostLease);
    }

    #[test]
    fn display_messages_are_actionable() {
        assert_eq!(
            StorageError::Unimplemented {
                method: "read_lease"
            }
            .to_string(),
            "read_lease is not implemented yet",
        );
        assert_eq!(
            StorageError::NotFound {
                resource: "checkpoint".to_string()
            }
            .to_string(),
            "checkpoint not found",
        );
        assert_eq!(
            StorageError::AlreadyExists {
                resource: "batch-1".to_string()
            }
            .to_string(),
            "batch-1 already exists",
        );
        assert_eq!(
            StorageError::InvalidState {
                reason: "batch is not pending".to_string()
            }
            .to_string(),
            "invalid state: batch is not pending",
        );
        assert_eq!(
            StorageError::LostLease.to_string(),
            "lost the primary-region lease",
        );
    }
}
