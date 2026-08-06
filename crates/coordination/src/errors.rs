//! The lease/epoch protocol's typed error taxonomy (spec §22.6).
//!
//! Conditional-write failures from the KV backend
//! ([`CloudError::ConditionFailed`]) are the protocol's normal "another writer
//! won" signal (spec §9.5); each operation maps that single backend outcome to
//! the *semantically distinct* failure its own state transition implies, so
//! callers match on protocol meaning, not on a generic CAS miss:
//!
//! | Operation | `ConditionFailed` means | Variant |
//! |---|---|---|
//! | [`acquire`](crate::LeaseCoordinator::acquire) | a lease already exists | [`LeaseError::LeaseHeld`] |
//! | [`renew`](crate::LeaseCoordinator::renew) | holder/epoch no longer ours | [`LeaseError::LostLease`] |
//! | [`claim_lease`](crate::LeaseCoordinator::claim_lease) | someone else took over first | [`LeaseError::EpochAdvanced`] |

use cloud_types::CloudError;

use crate::ids::EpochOverflow;

/// Everything a lease/epoch operation can fail with (rule
/// thiserror-for-libs-eyre-for-bins).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeaseError {
    /// The lease is validly held by a primary and is not takeover-eligible.
    ///
    /// Returned by [`acquire`](crate::LeaseCoordinator::acquire) when a lease
    /// item already exists, and by
    /// [`claim_lease`](crate::LeaseCoordinator::claim_lease) when the current
    /// lease has not expired past the safety margin (spec §8.3).
    #[error("lease is held and not takeover-eligible")]
    LeaseHeld,

    /// A renewal or fenced write failed because this holder no longer owns the
    /// current-epoch lease — another region took over (spec §8.3).
    ///
    /// The demoted primary must stop acting as primary immediately: its epoch
    /// is stale and every fenced write it attempts will now fail.
    #[error("lease lost: holder id or epoch no longer matches the current lease")]
    LostLease,

    /// A takeover lost the atomic epoch CAS: the epoch advanced between reading
    /// the lease and writing the claim, so a concurrent challenger won the
    /// race (spec §8.3 "every takeover atomically increments epoch").
    #[error("epoch advanced: another region claimed the lease concurrently")]
    EpochAdvanced,

    /// No lease item exists at the coordination key.
    ///
    /// [`read_lease`](crate::LeaseCoordinator::read_lease) returns this at
    /// bootstrap (before any [`acquire`](crate::LeaseCoordinator::acquire));
    /// [`renew`](crate::LeaseCoordinator::renew) /
    /// [`claim_lease`](crate::LeaseCoordinator::claim_lease) return it when the
    /// item is absent (there is nothing to renew or take over — acquire first).
    #[error("no lease exists at key {key}")]
    NoLease {
        /// The rendered coordination key that held no lease item.
        key: String,
    },

    /// The stored lease item was not a well-formed lease (missing attribute,
    /// wrong value type, or an `expires_at` outside the representable range).
    ///
    /// A data-integrity fault, not a normal protocol outcome: the coordination
    /// item was written by something that does not speak this schema (spec
    /// §8.2), so the protocol refuses to guess rather than act on a corrupt
    /// lease.
    #[error("malformed lease item: {reason}")]
    MalformedLease {
        /// Human-readable description of how the item violated the §8.2 schema.
        reason: String,
    },

    /// The fencing epoch could not be advanced because it already holds
    /// `u64::MAX` (see [`EpochOverflow`]).
    #[error(transparent)]
    EpochOverflow(#[from] EpochOverflow),

    /// A transport- or service-level backend failure that is not a protocol
    /// outcome (timeout, throttle, 5xx, auth): surfaced verbatim so callers can
    /// apply [`CloudError::is_retryable`] retry policy.
    #[error("backend error: {0}")]
    Backend(CloudError),
}
