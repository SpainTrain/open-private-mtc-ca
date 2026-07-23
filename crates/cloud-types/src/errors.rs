//! Error taxonomy shared by every cloud backend (spec §9.1, §9.3).
//!
//! All four capability traits ([`ObjectStore`](crate::ObjectStore),
//! [`ObjectLock`](crate::ObjectLock), [`ReplicatedKv`](crate::ReplicatedKv),
//! [`Hsm`](crate::Hsm)) return [`CloudError`]. Backends translate their native
//! failures (AWS SDK errors, PKCS#11 `CKR_*` codes, ...) into this taxonomy at
//! the trait boundary and never leak vendor error types outward
//! (.claude/rules/no-sdk-types-in-domain, spec §22.8).
//!
//! The taxonomy deliberately distinguishes the failure classes the CA's
//! protocols depend on:
//!
//! - [`CloudError::NotFound`] — read/miss semantics.
//! - [`CloudError::AlreadyExists`] — if-not-exists puts, the append-only
//!   invariant's storage-level enforcement (spec §8, §9.7
//!   `test_overwrite_fails`).
//! - [`CloudError::ConditionFailed`] — conditional-write (CAS) losses, the
//!   signal the lease/epoch protocol is built on (spec §9.5).
//! - [`CloudError::RetentionViolation`] — retention-lock enforcement
//!   ("cannot delete during retention window even by admins", spec §9.5).
//! - [`CloudError::Transport`] — transport/service-level failures, tagged
//!   retryable or terminal so callers can apply retry policy uniformly.

/// Unified error type for all cloud-capability trait methods.
///
/// This enum is deliberately exhaustive (no `#[non_exhaustive]`): adding a
/// variant is a conscious, breaking change that every backend and caller must
/// handle explicitly (spec §22.3 — exhaustive matching is the language
/// default and we keep it).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloudError {
    /// The requested object, item, or key handle does not exist.
    ///
    /// Returned by reads (`get`/`head`/`get_retention`/KV `get`) for missing
    /// keys, and by HSM operations referencing an unknown
    /// [`KeyHandle`](crate::KeyHandle). Terminal: retrying without a state
    /// change cannot succeed.
    #[error("not found: {key}")]
    NotFound {
        /// The object key, KV key, or HSM key-handle identifier that was not
        /// found.
        key: String,
    },

    /// The target already exists and the operation required that it did not.
    ///
    /// Returned by [`ObjectStore::put`](crate::ObjectStore::put) under
    /// [`PutMode::IfNotExists`](crate::PutMode::IfNotExists) and by
    /// [`ObjectLock::put_with_retention`](crate::ObjectLock::put_with_retention)
    /// when the key is taken. This is the storage-level face of the
    /// append-only invariant (spec §8): committed bytes are never replaced.
    /// Terminal.
    #[error("already exists: {key}")]
    AlreadyExists {
        /// The key that already holds an object or item.
        key: String,
    },

    /// A conditional KV write's condition did not hold.
    ///
    /// Returned by [`ReplicatedKv`](crate::ReplicatedKv) `put` /
    /// `atomic_update` / `transact` when any [`Condition`](crate::Condition)
    /// evaluates false (e.g. DynamoDB `ConditionalCheckFailedException`).
    /// Losing a CAS is a normal protocol outcome — the lease/epoch protocol
    /// treats it as "another writer won" (spec §9.5) — so it is terminal at
    /// the transport level; callers re-read state and decide at the protocol
    /// level whether to retry.
    #[error("condition failed: {reason}")]
    ConditionFailed {
        /// Backend-provided description of which condition failed.
        reason: String,
    },

    /// The operation would violate a retention lock.
    ///
    /// Returned when deleting or overwriting an object during its retention
    /// window, or when attempting to shorten retention via
    /// [`ObjectLock::extend_retention`](crate::ObjectLock::extend_retention)
    /// (retention is forward-only). Enforces the §9.5 bar: "cannot delete
    /// during retention window even by admins". Terminal.
    #[error("retention violation: {reason}")]
    RetentionViolation {
        /// Description of the violated retention constraint.
        reason: String,
    },

    /// A transport- or service-level failure between the CA and the backend.
    ///
    /// Covers timeouts, connection resets, throttling, 5xx responses,
    /// authentication/authorization failures, and backend-internal faults
    /// (e.g. PKCS#11 `CKR_DEVICE_ERROR`). The `retryable` flag is the
    /// backend's classification: `true` for transient faults (timeout,
    /// throttle, 5xx) where retrying with backoff is sound; `false` for
    /// terminal faults (auth failure, malformed request, misconfiguration)
    /// where retrying cannot help.
    #[error("transport error (retryable={retryable}): {reason}")]
    Transport {
        /// Whether retrying the same operation (with backoff) may succeed.
        retryable: bool,
        /// Backend-provided description of the failure.
        reason: String,
    },
}

impl CloudError {
    /// Returns `true` when retrying the failed operation (with backoff) may
    /// succeed without any other state change.
    ///
    /// Only [`CloudError::Transport`] with `retryable == true` qualifies.
    /// Everything else is terminal at the transport level:
    /// [`CloudError::ConditionFailed`] and [`CloudError::AlreadyExists`] are
    /// protocol outcomes (re-read state, then decide), and
    /// [`CloudError::NotFound`] / [`CloudError::RetentionViolation`] cannot be
    /// resolved by repetition.
    ///
    /// ```
    /// use cloud_types::CloudError;
    ///
    /// let throttled = CloudError::Transport {
    ///     retryable: true,
    ///     reason: "throttled".to_string(),
    /// };
    /// assert!(throttled.is_retryable());
    ///
    /// let lost_cas = CloudError::ConditionFailed {
    ///     reason: "epoch mismatch".to_string(),
    /// };
    /// assert!(!lost_cas.is_retryable());
    /// ```
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Transport {
                retryable: true,
                ..
            }
        )
    }

    /// Returns `true` for failures caused by a violated write precondition:
    /// [`CloudError::AlreadyExists`] (if-not-exists put lost) or
    /// [`CloudError::ConditionFailed`] (conditional KV write lost).
    ///
    /// These are the "another writer got there first" outcomes that the
    /// lease/epoch protocol and append-only writes treat as normal protocol
    /// signals rather than faults (spec §9.5).
    #[must_use]
    pub fn is_precondition_failure(&self) -> bool {
        matches!(
            self,
            Self::AlreadyExists { .. } | Self::ConditionFailed { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::CloudError;

    fn all_variants() -> Vec<CloudError> {
        vec![
            CloudError::NotFound {
                key: "tile/0/000".to_string(),
            },
            CloudError::AlreadyExists {
                key: "entries/00000001".to_string(),
            },
            CloudError::ConditionFailed {
                reason: "epoch mismatch".to_string(),
            },
            CloudError::RetentionViolation {
                reason: "retain_until not reached".to_string(),
            },
            CloudError::Transport {
                retryable: true,
                reason: "throttled".to_string(),
            },
            CloudError::Transport {
                retryable: false,
                reason: "access denied".to_string(),
            },
        ]
    }

    #[test]
    fn only_retryable_transport_is_retryable() {
        let retryable: Vec<bool> = all_variants()
            .iter()
            .map(CloudError::is_retryable)
            .collect();
        assert_eq!(retryable, vec![false, false, false, false, true, false]);
    }

    #[test]
    fn precondition_failures_are_exactly_already_exists_and_condition_failed() {
        let precondition: Vec<bool> = all_variants()
            .iter()
            .map(CloudError::is_precondition_failure)
            .collect();
        assert_eq!(precondition, vec![false, true, true, false, false, false]);
    }

    #[test]
    fn display_includes_classification_and_detail() {
        let retryable = CloudError::Transport {
            retryable: true,
            reason: "timeout".to_string(),
        };
        assert_eq!(
            retryable.to_string(),
            "transport error (retryable=true): timeout"
        );

        let terminal = CloudError::Transport {
            retryable: false,
            reason: "access denied".to_string(),
        };
        assert_eq!(
            terminal.to_string(),
            "transport error (retryable=false): access denied"
        );

        let not_found = CloudError::NotFound {
            key: "tile/0/000".to_string(),
        };
        assert_eq!(not_found.to_string(), "not found: tile/0/000");
    }

    #[test]
    fn error_trait_is_implemented() {
        // thiserror derives std::error::Error; keep it that way so callers can
        // box/chain these through generic error-handling layers.
        fn assert_error<E: std::error::Error + Send + Sync + 'static>(_: &E) {}
        for variant in all_variants() {
            assert_error(&variant);
        }
    }
}
