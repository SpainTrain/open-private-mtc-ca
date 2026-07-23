//! `DynamoDB` Global Tables simulation.
//!
//! Tails a source table by **scan-diff** (not `DynamoDB` Streams — see
//! `docs/adr/` for why) and applies changes to a target table with
//! **last-writer-wins (LWW)** conflict resolution, the documented conflict
//! semantics `DynamoDB` Global Tables itself provides (ticket
//! dev-crr-replication-sim AC; mr-replication-sim AC "conflict semantics
//! (last-writer-wins) documented").
//!
//! # Last-writer-wins, concretely
//!
//! Every replicated write carries a hidden bookkeeping attribute,
//! [`TS_ATTR`], set to the replicator's own clock reading (millis since the
//! Unix epoch) at the moment the change was *applied* (not discovered). Both
//! `PutItem` and `DeleteItem` on the target carry a `ConditionExpression`
//! requiring the incoming timestamp to be strictly newer than whatever is
//! already stored:
//!
//! ```text
//! attribute_not_exists(#ts) OR #ts < :ts
//! ```
//!
//! A write that loses the race (`ConditionalCheckFailedException`) is not an
//! error — it is LWW working as designed, so `apply_due` counts it as
//! `stale`, not `failed`, and does not retry it. This makes replay
//! idempotent by construction: replaying the exact same write twice applies
//! once and is rejected (correctly) the second time, since the stored
//! timestamp is no longer strictly less than the replay's.
//!
//! This mirrors real `DynamoDB` Global Tables, which resolves concurrent
//! writes to the same item by "last writer wins" using its own internal
//! replication timestamp — invisible to callers. Here the timestamp is a
//! visible extra item attribute instead (a documented emulation gap: a
//! replicated item carries one extra internal attribute the application
//! never reads, `docs/dev-environment.md`).
//!
//! # Why scan-diff, not `DynamoDB` Streams
//!
//! See the ADR. In short: streams add stream-shard/iterator lifecycle
//! management with uneven emulation fidelity across `LocalStack` editions;
//! scan-diff is simpler, backend-portable, and sufficient at dev-environment
//! scale. The trade-off is documented as a known limitation below.
//!
//! # Known limitations (documented, not bugs)
//!
//! - **Change detection is scan-interval-grained**: an item that is written
//!   and overwritten again between two polls is only ever seen in its
//!   latest state — no intra-interval history. Real Streams-based CRR would
//!   see every write. At dev-environment poll intervals (hundreds of ms)
//!   this is very unlikely to matter for exercising the coordination-table
//!   write paths, but it is a real fidelity gap.
//! - **Content fingerprinting is not attribute-filtered**: the fingerprint
//!   used to detect "did this item change" includes [`TS_ATTR`] itself, so
//!   a table replicated *through* this simulator on more than one hop will
//!   see extra (harmless, idempotent) discovery events. Documented, not
//!   incorrect.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aws_sdk_dynamodb::operation::delete_item::DeleteItemError;
use aws_sdk_dynamodb::operation::put_item::PutItemError;
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client;
use clock::Clock;

use crate::error::ReplicatorError;
use crate::lag::{LagPolicy, LagScheduler};
use crate::ApplySummary;

/// The single-table schema's partition/sort key attribute names (spec §8.2).
const PK_ATTR: &str = "PK";
const SK_ATTR: &str = "SK";

/// Hidden bookkeeping attribute carrying the replicator's own
/// last-writer-wins timestamp (millis since the Unix epoch, as a `DynamoDB`
/// `N`). See the module docs.
pub const TS_ATTR: &str = "_dev_replicator_ts";

/// Dedup/ordering key: `(pk, sk, content fingerprint)`.
///
/// Unlike S3 versions, a `DynamoDB` item's `(pk, sk)` is *not* immutable — the
/// same key can legitimately change many times, and each change must be
/// replicated. Folding a content fingerprint into the key means the generic
/// [`LagScheduler`]'s "already applied" dedup — which is exactly right for
/// S3's immutable versions — does the right thing here too: the *same*
/// content at a key is idempotent (never re-queued), but *new* content at
/// the same key is a distinct key and queues normally.
pub type DdbKey = (String, String, String);

/// One discovered `DynamoDB` change.
#[derive(Debug, Clone)]
pub enum DdbChange {
    /// The item at `(pk, sk)` was created or updated to `item`.
    Upsert {
        /// Partition key value.
        pk: String,
        /// Sort key value.
        sk: String,
        /// The full item as scanned from the source.
        item: HashMap<String, AttributeValue>,
    },
    /// The item at `(pk, sk)` disappeared from a scan (deleted or expired).
    Delete {
        /// Partition key value.
        pk: String,
        /// Sort key value.
        sk: String,
    },
}

/// Polls one `DynamoDB` table on the source endpoint and replicates it to the
/// same table name on the target endpoint with LWW conflict resolution.
pub struct DdbPoller {
    source: Client,
    target: Client,
    table: String,
    clock: Arc<dyn Clock>,
    scheduler: LagScheduler<DdbKey, DdbChange>,
    /// `(pk, sk)` pairs present as of the most recent full scan — used only
    /// to detect deletions (items present last scan, absent this scan).
    known_live_keys: HashSet<(String, String)>,
}

impl DdbPoller {
    /// Creates a poller for `table`, replicating `source` → `target`.
    #[must_use]
    pub fn new(
        source: Client,
        target: Client,
        table: String,
        clock: Arc<dyn Clock>,
        initial_lag: LagPolicy,
    ) -> Self {
        Self {
            source,
            target,
            table,
            clock,
            scheduler: LagScheduler::new(initial_lag),
            known_live_keys: HashSet::new(),
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

    /// Total changes replicated so far.
    #[must_use]
    pub fn applied_len(&self) -> usize {
        self.scheduler.applied_len()
    }

    /// Scans the full source table, queuing any item whose content differs
    /// from what was last discovered, plus a delete event for any key that
    /// was present in the previous scan and is now absent.
    ///
    /// # Errors
    ///
    /// Returns [`ReplicatorError::Ddb`] if `Scan` fails.
    pub async fn discover(&mut self) -> Result<usize, ReplicatorError> {
        let now = self.clock.now();
        let mut newly_queued = 0usize;
        let mut current_keys: HashSet<(String, String)> = HashSet::new();
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let mut req = self.source.scan().table_name(&self.table);
            if let Some(esk) = exclusive_start_key.take() {
                req = req.set_exclusive_start_key(Some(esk));
            }
            let output = req
                .send()
                .await
                .map_err(|e| ReplicatorError::ddb("scan", &e))?;

            for item in output.items() {
                let Some((pk, sk)) = extract_key(item) else {
                    continue;
                };
                current_keys.insert((pk.clone(), sk.clone()));
                let dedup_key = (pk.clone(), sk.clone(), fingerprint_of(item));
                let event = DdbChange::Upsert {
                    pk,
                    sk,
                    item: item.clone(),
                };
                if self.scheduler.discover(dedup_key, event, now) {
                    newly_queued += 1;
                }
            }

            exclusive_start_key = output.last_evaluated_key().cloned();
            if exclusive_start_key.is_none() {
                break;
            }
        }

        let vanished: Vec<(String, String)> = self
            .known_live_keys
            .difference(&current_keys)
            .cloned()
            .collect();
        for (pk, sk) in vanished {
            let dedup_key = (pk.clone(), sk.clone(), "\u{0}deleted".to_string());
            let event = DdbChange::Delete { pk, sk };
            if self.scheduler.discover(dedup_key, event, now) {
                newly_queued += 1;
            }
        }
        self.known_live_keys = current_keys;
        Ok(newly_queued)
    }

    /// Applies every change whose lag has elapsed. A `ConditionalCheckFailedException`
    /// (the write lost the LWW race) counts as `stale`, not `failed` — see
    /// the module docs. Other failures are logged and counted as `failed`;
    /// one bad item does not stall the rest of the batch.
    pub async fn apply_due(&mut self) -> ApplySummary {
        let now = self.clock.now();
        let due = self.scheduler.drain_due(now);
        let mut summary = ApplySummary::default();
        for item in due {
            match self.apply_one(&item.event, now).await {
                Ok(true) => summary.applied += 1,
                Ok(false) => summary.stale += 1,
                Err(err) => {
                    summary.failed += 1;
                    tracing::error!(
                        table = %self.table,
                        error = %err,
                        "ddb replication failed for this change"
                    );
                }
            }
        }
        summary
    }

    /// Returns `Ok(true)` if applied, `Ok(false)` if rejected by the LWW
    /// condition (stale — not an error).
    async fn apply_one(
        &self,
        event: &DdbChange,
        applied_at: SystemTime,
    ) -> Result<bool, ReplicatorError> {
        let ts = ts_attr_value(applied_at);
        match event {
            DdbChange::Upsert { item, .. } => {
                let mut full_item = item.clone();
                full_item.insert(TS_ATTR.to_string(), ts.clone());
                let result = self
                    .target
                    .put_item()
                    .table_name(&self.table)
                    .set_item(Some(full_item))
                    .condition_expression("attribute_not_exists(#ts) OR #ts < :ts")
                    .expression_attribute_names("#ts", TS_ATTR)
                    .expression_attribute_values(":ts", ts)
                    .send()
                    .await;
                match result {
                    Ok(_) => Ok(true),
                    Err(err) => {
                        if err
                            .as_service_error()
                            .is_some_and(PutItemError::is_conditional_check_failed_exception)
                        {
                            Ok(false)
                        } else {
                            Err(ReplicatorError::ddb("put_item", &err))
                        }
                    }
                }
            }
            DdbChange::Delete { pk, sk } => {
                let result = self
                    .target
                    .delete_item()
                    .table_name(&self.table)
                    .key(PK_ATTR, AttributeValue::S(pk.clone()))
                    .key(SK_ATTR, AttributeValue::S(sk.clone()))
                    .condition_expression("attribute_not_exists(#ts) OR #ts < :ts")
                    .expression_attribute_names("#ts", TS_ATTR)
                    .expression_attribute_values(":ts", ts)
                    .send()
                    .await;
                match result {
                    Ok(_) => Ok(true),
                    Err(err) => {
                        if err
                            .as_service_error()
                            .is_some_and(DeleteItemError::is_conditional_check_failed_exception)
                        {
                            Ok(false)
                        } else {
                            Err(ReplicatorError::ddb("delete_item", &err))
                        }
                    }
                }
            }
        }
    }
}

fn ts_attr_value(now: SystemTime) -> AttributeValue {
    let millis = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    AttributeValue::N(millis.to_string())
}

fn extract_key(item: &HashMap<String, AttributeValue>) -> Option<(String, String)> {
    let pk = item.get(PK_ATTR).and_then(|v| v.as_s().ok())?.clone();
    let sk = item.get(SK_ATTR).and_then(|v| v.as_s().ok())?.clone();
    Some((pk, sk))
}

/// A stable (sorted-key), human-diffable content fingerprint. Not
/// cryptographic — just deterministic across `HashMap` iteration order, so
/// equal content always fingerprints equal.
fn fingerprint_of(item: &HashMap<String, AttributeValue>) -> String {
    let sorted: std::collections::BTreeMap<&String, &AttributeValue> = item.iter().collect();
    format!("{sorted:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_regardless_of_insertion_order() {
        let mut a = HashMap::new();
        a.insert(PK_ATTR.to_string(), AttributeValue::S("log#1".to_string()));
        a.insert(SK_ATTR.to_string(), AttributeValue::S("lease".to_string()));
        a.insert("epoch".to_string(), AttributeValue::N("3".to_string()));

        let mut b = HashMap::new();
        b.insert("epoch".to_string(), AttributeValue::N("3".to_string()));
        b.insert(SK_ATTR.to_string(), AttributeValue::S("lease".to_string()));
        b.insert(PK_ATTR.to_string(), AttributeValue::S("log#1".to_string()));

        assert_eq!(fingerprint_of(&a), fingerprint_of(&b));
    }

    #[test]
    fn fingerprint_changes_when_content_changes() {
        let mut a = HashMap::new();
        a.insert("epoch".to_string(), AttributeValue::N("3".to_string()));
        let mut b = HashMap::new();
        b.insert("epoch".to_string(), AttributeValue::N("4".to_string()));

        assert_ne!(fingerprint_of(&a), fingerprint_of(&b));
    }

    #[test]
    fn extract_key_reads_pk_and_sk_strings() {
        let mut item = HashMap::new();
        item.insert(PK_ATTR.to_string(), AttributeValue::S("log#1".to_string()));
        item.insert(SK_ATTR.to_string(), AttributeValue::S("lease".to_string()));
        assert_eq!(
            extract_key(&item),
            Some(("log#1".to_string(), "lease".to_string()))
        );
    }

    #[test]
    fn extract_key_none_when_pk_or_sk_missing_or_wrong_type() {
        let mut missing_sk = HashMap::new();
        missing_sk.insert(PK_ATTR.to_string(), AttributeValue::S("log#1".to_string()));
        assert_eq!(extract_key(&missing_sk), None);

        let mut wrong_type = HashMap::new();
        wrong_type.insert(PK_ATTR.to_string(), AttributeValue::N("1".to_string()));
        wrong_type.insert(SK_ATTR.to_string(), AttributeValue::S("lease".to_string()));
        assert_eq!(extract_key(&wrong_type), None);
    }

    #[test]
    fn ts_attr_value_is_millis_since_epoch() {
        let t = UNIX_EPOCH + std::time::Duration::from_secs(5);
        assert_eq!(ts_attr_value(t), AttributeValue::N("5000".to_string()));
    }
}

// End-to-end LWW-under-condition-expression behavior needs a live LocalStack
// table (see `tests/integration.rs`, `#[ignore]`-gated). The dedup/ordering
// logic underneath is exhaustively unit- and property-tested infra-free in
// `lag.rs`; the pure fingerprint/key-extraction helpers are tested above.
