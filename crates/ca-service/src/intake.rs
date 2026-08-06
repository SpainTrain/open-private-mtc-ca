//! The [`EntryIntake`] trait: the one door every adapter knocks on (spec
//! §10.2-§10.3).

use async_trait::async_trait;
use mtc::Index;
use thiserror::Error;

use crate::entry::LogEntry;

/// The source-agnostic entry-intake seam (spec §10.2, §25.9).
///
/// This is the boundary between Stage 1 (source-specific intake — adapters:
/// native ACME today, AWS Private CA / Keyfactor / Cloudflare PCA / etc.
/// later) and Stage 2 (the common log-the-entry pipeline: batch builder, tree
/// updater, HSM signer, commit — spec §10 diagram). Everything on the Stage-1
/// side of the boundary — including the native ACME endpoint — is *just an
/// adapter*: none of them get a special integrated path (spec §10.4).
///
/// Implementations of this trait own the intake queue (spec §8.4: "in-memory,
/// drained by batch builder") and the write-path lease/epoch check (spec
/// §8.3, §11.1 step 1). Both are out of scope for this crate — this trait
/// exists so adapters, and the queue that eventually implements it, can be
/// built against a stable contract independently (ticket mtc-kjl; the queue
/// itself is ticket mtc-2kx).
///
/// # Adapter responsibilities (spec §10.3)
///
/// Every adapter — including the native ACME endpoint — is responsible for:
///
/// 1. Subscribing/polling/listening to its source CA's issuance events.
/// 2. Validating the certificate is one we should log (a policy decision left
///    entirely to the adapter; this trait does not adjudicate it).
/// 3. Constructing a spec-compliant `TbsCertificateLogEntry` from the source
///    cert and placing its serialized bytes in [`LogEntry::tbs_cert`].
/// 4. Submitting via [`submit_entry`](EntryIntake::submit_entry).
/// 5. Handling source-specific authentication, retry, and idempotency (a
///    lost/retried call to `submit_entry` is the adapter's problem to make
///    idempotent, e.g. via `LogEntry::source_id` as a dedupe key upstream of
///    this trait — this trait itself does not deduplicate).
///
/// Adapters can run as Lambda functions (event-driven sources) or Fargate
/// tasks (long-poll or WebSocket sources); either way they authenticate to
/// the CA Service via short-lived credentials issued by the platform's IAM
/// (spec §10.3). None of that shows up in this trait's signature — it is
/// transport- and deployment-agnostic by design.
///
/// # Object safety
///
/// `#[async_trait]` (not native async-fn-in-trait) so this trait is
/// dyn-compatible: implementations are shared as `Arc<dyn EntryIntake>` across
/// every concurrently-running adapter task, the same architectural-seam
/// pattern as the four cloud-types capability traits (rule
/// `prefer-generics-on-hot-paths`).
#[async_trait]
pub trait EntryIntake: Send + Sync {
    /// Submits one entry, returning the [`Index`] assigned to it in the
    /// issuance log.
    ///
    /// The entry is taken by value: once submitted, the caller has no further
    /// use for it and the implementation (the intake queue) is free to buffer
    /// it without cloning.
    ///
    /// # Errors
    ///
    /// - [`IntakeError::QueueFull`] — the intake queue (spec §8.4) is at
    ///   capacity; the caller should back off and retry.
    /// - [`IntakeError::NotPrimary`] — this region does not hold the current
    ///   write-path lease (spec §8.3, §11: writes execute only on the primary
    ///   region); the caller should redirect to the primary.
    /// - [`IntakeError::Shutdown`] — the intake queue is no longer accepting
    ///   entries (service shutting down or already shut down).
    async fn submit_entry(&self, entry: LogEntry) -> Result<Index, IntakeError>;
}

/// Failure modes of [`EntryIntake::submit_entry`].
///
/// Deliberately exhaustive (no `#[non_exhaustive]`, spec §22.3): a new failure
/// mode is a conscious addition every caller must handle, not a silent
/// widening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IntakeError {
    /// The intake queue (spec §8.4) is at capacity and cannot buffer another
    /// entry until the batch builder drains it.
    #[error("entry intake queue is full")]
    QueueFull,
    /// This region does not hold the current write-path lease (spec §8.3):
    /// writes are only valid on the primary region (spec §11). Standby
    /// regions reject write requests with HTTP 503 + redirect at the HTTP
    /// layer above this trait.
    #[error("this region is not the write-path primary")]
    NotPrimary,
    /// The intake queue is shutting down (or has already shut down) and is no
    /// longer accepting entries.
    #[error("entry intake queue is shutting down")]
    Shutdown,
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use mtc::Index;

    use super::{EntryIntake, IntakeError};
    use crate::entry::{LogEntry, SourceId, SourceType};

    fn sample_entry() -> LogEntry {
        LogEntry::new(
            vec![0xAB, 0xCD],
            SourceType::NativeAcme,
            SourceId::from("order-1"),
            UNIX_EPOCH,
        )
    }

    /// A minimal test double: always succeeds, handing back a fixed index.
    struct AlwaysAdmits {
        next_index: u64,
    }

    #[async_trait::async_trait]
    impl EntryIntake for AlwaysAdmits {
        async fn submit_entry(&self, _entry: LogEntry) -> Result<Index, IntakeError> {
            Ok(Index(self.next_index))
        }
    }

    /// A minimal test double: always fails with a configured error.
    struct AlwaysRejects {
        error: IntakeError,
    }

    #[async_trait::async_trait]
    impl EntryIntake for AlwaysRejects {
        async fn submit_entry(&self, _entry: LogEntry) -> Result<Index, IntakeError> {
            Err(self.error)
        }
    }

    #[tokio::test]
    async fn mock_impl_returns_assigned_index() {
        let intake = AlwaysAdmits { next_index: 7 };
        let assigned = intake
            .submit_entry(sample_entry())
            .await
            .expect("mock always admits");
        assert_eq!(assigned, Index(7));
    }

    #[tokio::test]
    async fn every_intake_error_variant_is_exercised() {
        for error in [
            IntakeError::QueueFull,
            IntakeError::NotPrimary,
            IntakeError::Shutdown,
        ] {
            let intake = AlwaysRejects { error };
            let observed = intake
                .submit_entry(sample_entry())
                .await
                .expect_err("mock always rejects");
            assert_eq!(observed, error);
        }
    }

    #[test]
    fn intake_error_messages_are_distinct_and_stable() {
        assert_eq!(
            IntakeError::QueueFull.to_string(),
            "entry intake queue is full"
        );
        assert_eq!(
            IntakeError::NotPrimary.to_string(),
            "this region is not the write-path primary"
        );
        assert_eq!(
            IntakeError::Shutdown.to_string(),
            "entry intake queue is shutting down"
        );
    }
}
