//! S3 CRR simulation.
//!
//! Polls a source bucket's `ListObjectVersions` and replicates newly
//! discovered versions (and delete markers) to a target bucket of the same
//! name, preserving Object Lock retention/legal-hold metadata (ticket
//! dev-crr-replication-sim AC).
//!
//! # Known limitations (documented, not bugs)
//!
//! - **Lag is measured from the poller's own discovery time**, not from the
//!   object's `LastModified` timestamp. Real CRR lag is "time since write";
//!   this simulator's lag is "time since *this process* noticed the write" —
//!   within one poll interval of the same thing, and it decouples the
//!   simulator from clock skew between the replicator process and the two
//!   `LocalStack` containers. See the crate-level docs.
//! - **Version IDs are not preserved** — `LocalStack` assigns its own version
//!   ID to each replicated `PutObject` on the target. What *is* preserved is
//!   the relative order versions appear in (ticket AC "preserving ... version
//!   order"): events apply strictly in discovery order.
//! - **Delete markers are replicated as deletes**, creating a fresh delete
//!   marker on the target (matching versioned-bucket semantics) rather than
//!   reproducing the source's exact delete-marker version ID.

use std::sync::Arc;

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ObjectLockMode;
use aws_sdk_s3::Client;
use clock::Clock;

use crate::error::ReplicatorError;
use crate::lag::{LagPolicy, LagScheduler};
use crate::ApplySummary;

/// Dedup/ordering key for one S3 change.
///
/// An S3 object version is immutable and globally unique per
/// `(key, version_id)`, so this key alone is a correct idempotency guard
/// (unlike `DynamoDB`'s mutable-item keys — see `ddb.rs`).
pub type S3Key = (String, String);

/// One discovered S3 change: a new object version, or a new delete marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3VersionEvent {
    /// The object key.
    pub object_key: String,
    /// The specific version ID this event refers to.
    pub version_id: String,
    /// Whether this version is a delete marker rather than object content.
    pub is_delete_marker: bool,
}

/// Polls one S3 bucket on the source endpoint and replicates it to the same
/// bucket name on the target endpoint.
pub struct S3Poller {
    source: Client,
    target: Client,
    bucket: String,
    clock: Arc<dyn Clock>,
    scheduler: LagScheduler<S3Key, S3VersionEvent>,
}

impl S3Poller {
    /// Creates a poller for `bucket`, replicating `source` → `target`.
    #[must_use]
    pub fn new(
        source: Client,
        target: Client,
        bucket: String,
        clock: Arc<dyn Clock>,
        initial_lag: LagPolicy,
    ) -> Self {
        Self {
            source,
            target,
            bucket,
            clock,
            scheduler: LagScheduler::new(initial_lag),
        }
    }

    /// The link's current lag policy.
    #[must_use]
    pub const fn lag_policy(&self) -> LagPolicy {
        self.scheduler.policy()
    }

    /// Updates the lag policy (control endpoint / mr-replication-sim
    /// runtime-adjustable lag requirement).
    pub const fn set_lag_policy(&mut self, policy: LagPolicy) {
        self.scheduler.set_policy(policy);
    }

    /// Items discovered but not yet replicated.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.scheduler.pending_len()
    }

    /// Age of the oldest undelivered discovery, if any.
    #[must_use]
    pub fn oldest_pending_age(&self) -> Option<std::time::Duration> {
        self.scheduler.oldest_pending_age(self.clock.now())
    }

    /// Total versions replicated so far.
    #[must_use]
    pub fn applied_len(&self) -> usize {
        self.scheduler.applied_len()
    }

    /// Lists all object versions and delete markers on the source, queuing
    /// anything not already pending or applied. Paginates via
    /// `key_marker`/`version_id_marker` until `IsTruncated` is false.
    ///
    /// # Errors
    ///
    /// Returns [`ReplicatorError::S3`] if `ListObjectVersions` fails.
    pub async fn discover(&mut self) -> Result<usize, ReplicatorError> {
        let now = self.clock.now();
        let mut newly_queued = 0usize;
        let mut key_marker: Option<String> = None;
        let mut version_id_marker: Option<String> = None;

        loop {
            let mut req = self.source.list_object_versions().bucket(&self.bucket);
            if let Some(k) = &key_marker {
                req = req.key_marker(k);
            }
            if let Some(v) = &version_id_marker {
                req = req.version_id_marker(v);
            }
            let output = req
                .send()
                .await
                .map_err(|e| ReplicatorError::s3("list_object_versions", &e))?;

            for v in output.versions() {
                if let (Some(key), Some(version_id)) = (v.key(), v.version_id()) {
                    let dedup_key = (key.to_string(), version_id.to_string());
                    let event = S3VersionEvent {
                        object_key: key.to_string(),
                        version_id: version_id.to_string(),
                        is_delete_marker: false,
                    };
                    if self.scheduler.discover(dedup_key, event, now) {
                        newly_queued += 1;
                    }
                }
            }
            for dm in output.delete_markers() {
                if let (Some(key), Some(version_id)) = (dm.key(), dm.version_id()) {
                    let dedup_key = (key.to_string(), version_id.to_string());
                    let event = S3VersionEvent {
                        object_key: key.to_string(),
                        version_id: version_id.to_string(),
                        is_delete_marker: true,
                    };
                    if self.scheduler.discover(dedup_key, event, now) {
                        newly_queued += 1;
                    }
                }
            }

            if output.is_truncated() == Some(true) {
                key_marker = output.next_key_marker().map(String::from);
                version_id_marker = output.next_version_id_marker().map(String::from);
            } else {
                break;
            }
        }
        Ok(newly_queued)
    }

    /// Replicates every version whose lag has elapsed. Per-item failures are
    /// logged (`tracing::error!`) and counted in the returned
    /// [`ApplySummary`] rather than aborting the whole batch — one bad
    /// object should not stall the rest of the link.
    pub async fn apply_due(&mut self) -> ApplySummary {
        let now = self.clock.now();
        let due = self.scheduler.drain_due(now);
        let mut summary = ApplySummary::default();
        for item in due {
            match self.apply_one(&item.event).await {
                Ok(()) => summary.applied += 1,
                Err(err) => {
                    summary.failed += 1;
                    tracing::error!(
                        bucket = %self.bucket,
                        key = %item.event.object_key,
                        version_id = %item.event.version_id,
                        error = %err,
                        "s3 replication failed for this version"
                    );
                }
            }
        }
        summary
    }

    async fn apply_one(&self, event: &S3VersionEvent) -> Result<(), ReplicatorError> {
        if event.is_delete_marker {
            self.target
                .delete_object()
                .bucket(&self.bucket)
                .key(&event.object_key)
                .send()
                .await
                .map_err(|e| ReplicatorError::s3("delete_object", &e))?;
            return Ok(());
        }

        let get = self
            .source
            .get_object()
            .bucket(&self.bucket)
            .key(&event.object_key)
            .version_id(&event.version_id)
            .send()
            .await
            .map_err(|e| ReplicatorError::s3("get_object", &e))?;
        let content_type = get.content_type().map(String::from);
        let body = get
            .body
            .collect()
            .await
            .map_err(|e| ReplicatorError::Body(format!("{e:?}")))?
            .into_bytes();

        // Object Lock metadata is best-effort: LocalStack (and real S3)
        // return an error when the object has no retention / no legal hold
        // configured, which we treat as "nothing to carry over", not a
        // failure of the replication itself.
        let retention = self
            .source
            .get_object_retention()
            .bucket(&self.bucket)
            .key(&event.object_key)
            .version_id(&event.version_id)
            .send()
            .await
            .ok();
        let legal_hold = self
            .source
            .get_object_legal_hold()
            .bucket(&self.bucket)
            .key(&event.object_key)
            .version_id(&event.version_id)
            .send()
            .await
            .ok();

        let mut put = self
            .target
            .put_object()
            .bucket(&self.bucket)
            .key(&event.object_key)
            .body(ByteStream::from(body.to_vec()));
        if let Some(ct) = content_type {
            put = put.content_type(ct);
        }
        if let Some(retention) = retention.as_ref().and_then(|r| r.retention()) {
            if let Some(mode) = retention.mode() {
                // ObjectLockRetentionMode (GetObjectRetention) and
                // ObjectLockMode (PutObject) are distinct SDK enums for the
                // same wire values — convert via their shared string repr.
                put = put.object_lock_mode(ObjectLockMode::from(mode.as_str()));
            }
            if let Some(until) = retention.retain_until_date() {
                put = put.object_lock_retain_until_date(*until);
            }
        }
        if let Some(status) = legal_hold
            .as_ref()
            .and_then(|l| l.legal_hold())
            .and_then(|l| l.status())
        {
            put = put.object_lock_legal_hold_status(status.clone());
        }
        put.send()
            .await
            .map_err(|e| ReplicatorError::s3("put_object", &e))?;
        Ok(())
    }
}

// `discover`/`apply_one` need a live LocalStack pair to exercise end-to-end
// (see `tests/integration.rs`, `#[ignore]`-gated). The dedup/ordering/lag
// logic they build on is exhaustively unit- and property-tested infra-free
// in `lag.rs`.
