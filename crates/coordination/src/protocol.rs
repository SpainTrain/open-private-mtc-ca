//! The primary-region lease item and its operations (spec §8.2/§8.3).
//!
//! The lease lives at one KV item — partition key `log#{logId}`, sort key
//! `primary-region-lease` (spec §8.2) — rendered into the single opaque
//! [`Key`] the [`ReplicatedKv`] trait uses as `log#{logId}/primary-region-lease`.
//! Its value is a [`Value::Map`] of the four §8.2 attributes: `region`,
//! `expires_at`, `epoch`, `holder_id`.
//!
//! # Why the takeover is safe without a read-then-write race
//!
//! [`LeaseCoordinator::claim_lease`] reads the lease to learn its epoch, then
//! performs the takeover as **one** [`ReplicatedKv::atomic_update`]: it writes
//! the advanced epoch and the new holder guarded by [`epoch_condition`] for the
//! observed epoch. That condition is evaluated
//! atomically with the write, so of N racing claimants exactly one passes
//! (its epoch matches) and installs epoch `E+1`; the losers' condition now
//! sees `E+1` and fails with [`LeaseError::EpochAdvanced`] (spec §8.3, §9.5).
//! The expiry check that gates the attempt is made against the injected
//! [`Clock`] with a safety margin; even if the incumbent renews in the gap
//! between the read and the write, the epoch CAS still admits a single winner
//! and the epoch-fencing on every subsequent write (spec §8.3) keeps the
//! demoted primary from mutating state.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clock::Clock;
use cloud_types::{CloudError, Condition, Item, Key, ReplicatedKv, UpdateExpression, Value};
use mtc::{Epoch, LogId};

use crate::errors::LeaseError;
use crate::ids::{EpochExt, HolderId, Region, INITIAL_EPOCH};
use crate::{LEASE_TTL, TAKEOVER_SAFETY_MARGIN};

/// Sort key of the lease item (spec §8.2).
const LEASE_SORT_KEY: &str = "primary-region-lease";

/// Top-level map attribute names of the lease item (spec §8.2).
const ATTR_REGION: &str = "region";
const ATTR_EXPIRES_AT: &str = "expires_at";
const ATTR_EPOCH: &str = "epoch";
const ATTR_HOLDER_ID: &str = "holder_id";

/// Renders the composite `(log#{logId}, primary-region-lease)` key into the
/// single `/`-segmented [`Key`] the KV abstraction uses (spec §8.2, and the
/// [`Key`] convention in `cloud-types`).
fn lease_key(log_id: &LogId) -> Key {
    Key::new(format!("log#{log_id}/{LEASE_SORT_KEY}"))
}

/// The epoch-equality condition every coordination write must carry (spec
/// §8.3: "Every write includes `epoch` in `ConditionExpression` — old
/// primaries cannot write after epoch advance").
///
/// This is the fencing token in condition form: a write guarded by
/// `epoch_condition(e)` applies only while the lease still sits at epoch `e`,
/// so a primary demoted by a takeover (which advanced the epoch) can no longer
/// mutate counter, checkpoint, or batch items. Reuse it for **every** such
/// write, not just the lease's own.
///
/// ```
/// use coordination::{epoch_condition, Epoch};
/// use cloud_types::{Condition, Value};
///
/// assert_eq!(
///     epoch_condition(Epoch(7)),
///     Condition::AttributeEquals {
///         attribute: "epoch".to_string(),
///         expected: Value::U64(7),
///     },
/// );
/// ```
#[must_use]
pub fn epoch_condition(epoch: Epoch) -> Condition {
    Condition::AttributeEquals {
        attribute: ATTR_EPOCH.to_string(),
        expected: Value::U64(epoch.0),
    }
}

/// Whole milliseconds since the Unix epoch for `t`, saturating at both ends
/// (before the epoch → `0`; beyond `u64::MAX` ms → `u64::MAX`). No `unwrap`
/// (rule no-unwrap-in-prod).
fn to_unix_millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The [`SystemTime`] `ms` whole milliseconds after the Unix epoch, or `None`
/// if that instant is not representable on this platform.
fn from_unix_millis(ms: u64) -> Option<SystemTime> {
    UNIX_EPOCH.checked_add(Duration::from_millis(ms))
}

/// Whole milliseconds in `d`, saturating at `u64::MAX`.
fn duration_millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Pure takeover-eligibility test (spec §8.3): a lease is takeover-eligible
/// once the clock has passed `expires_at` by at least the safety margin.
///
/// Integer-only and `saturating` so it never panics — the core of the
/// no-panic Kani harness (`proofs/lease_epoch.rs`). `pub` for the harness and
/// unit tests, but the enclosing module is private, so it is crate-internal.
pub const fn takeover_eligible_millis(
    now_ms: u64,
    expires_at_ms: u64,
    safety_margin_ms: u64,
) -> bool {
    now_ms >= expires_at_ms.saturating_add(safety_margin_ms)
}

/// A decoded primary-region lease (spec §8.2).
///
/// The read/return view of the lease item, with `expires_at` as a
/// [`SystemTime`] and `epoch` as the typed fencing [`Epoch`]. Obtained from
/// [`LeaseCoordinator::read_lease`] and returned by the mutating operations as
/// the post-write state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lease {
    /// Region the current holder runs in (`region` attribute).
    pub region: Region,
    /// Identity of the current holder (`holder_id` attribute).
    pub holder_id: HolderId,
    /// Current fencing epoch (`epoch` attribute) — advanced by every takeover.
    pub epoch: Epoch,
    /// Instant the lease expires (`expires_at` attribute); the holder renews
    /// before this, challengers may take over past it plus the safety margin.
    pub expires_at: SystemTime,
}

/// Encodes the four §8.2 lease attributes into a KV [`Value::Map`].
fn encode_lease(region: &Region, holder: &HolderId, epoch: Epoch, expires_at_ms: u64) -> Value {
    Value::Map(BTreeMap::from([
        (
            ATTR_REGION.to_string(),
            Value::String(region.as_str().to_string()),
        ),
        (
            ATTR_HOLDER_ID.to_string(),
            Value::String(holder.as_str().to_string()),
        ),
        (ATTR_EPOCH.to_string(), Value::U64(epoch.0)),
        (ATTR_EXPIRES_AT.to_string(), Value::U64(expires_at_ms)),
    ]))
}

/// Builds a [`LeaseError::MalformedLease`] with `reason`.
fn malformed(reason: impl Into<String>) -> LeaseError {
    LeaseError::MalformedLease {
        reason: reason.into(),
    }
}

/// Reads a required `U64` attribute, or a [`LeaseError::MalformedLease`].
fn require_u64(map: &BTreeMap<String, Value>, attr: &str) -> Result<u64, LeaseError> {
    match map.get(attr) {
        Some(Value::U64(n)) => Ok(*n),
        Some(_) => Err(malformed(format!("attribute {attr:?} is not a U64"))),
        None => Err(malformed(format!("attribute {attr:?} is missing"))),
    }
}

/// Reads a required `String` attribute, or a [`LeaseError::MalformedLease`].
fn require_string(map: &BTreeMap<String, Value>, attr: &str) -> Result<String, LeaseError> {
    match map.get(attr) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(malformed(format!("attribute {attr:?} is not a String"))),
        None => Err(malformed(format!("attribute {attr:?} is missing"))),
    }
}

/// Decodes a stored [`Item`] into a [`Lease`], validating the §8.2 schema.
fn decode_lease(item: Item) -> Result<Lease, LeaseError> {
    let Value::Map(map) = item.value else {
        return Err(malformed("lease item value is not a Map"));
    };
    let region = Region::new(require_string(&map, ATTR_REGION)?);
    let holder_id = HolderId::new(require_string(&map, ATTR_HOLDER_ID)?);
    let epoch = Epoch(require_u64(&map, ATTR_EPOCH)?);
    let expires_at_ms = require_u64(&map, ATTR_EXPIRES_AT)?;
    let expires_at = from_unix_millis(expires_at_ms)
        .ok_or_else(|| malformed("expires_at is outside the representable range"))?;
    Ok(Lease {
        region,
        holder_id,
        epoch,
        expires_at,
    })
}

/// Coordinator for one region's view of the primary-region lease (spec §8.2/§8.3).
///
/// Holds the cloud-agnostic [`ReplicatedKv`] backend and the injected
/// [`Clock`] as `Arc<dyn _>` — the sanctioned dynamic-dispatch seam for
/// swappable backends and injected time (rule prefer-generics-on-hot-paths;
/// this is not a hot path). One coordinator represents this region's identity
/// (`holder_id`, `region`) against one log's lease item.
pub struct LeaseCoordinator {
    kv: Arc<dyn ReplicatedKv>,
    clock: Arc<dyn Clock>,
    key: Key,
    identity: HolderId,
    region: Region,
}

impl LeaseCoordinator {
    /// Creates a coordinator for `log_id`'s lease, acting as `identity` in
    /// `region`.
    #[must_use]
    pub fn new(
        kv: Arc<dyn ReplicatedKv>,
        clock: Arc<dyn Clock>,
        log_id: &LogId,
        identity: HolderId,
        region: Region,
    ) -> Self {
        Self {
            kv,
            clock,
            key: lease_key(log_id),
            identity,
            region,
        }
    }

    /// The rendered coordination key this coordinator reads and writes.
    #[must_use]
    pub const fn key(&self) -> &Key {
        &self.key
    }

    /// This coordinator's holder identity.
    #[must_use]
    pub const fn holder_id(&self) -> &HolderId {
        &self.identity
    }

    /// Current time in whole Unix milliseconds, via the injected clock.
    fn now_millis(&self) -> u64 {
        to_unix_millis(self.clock.now())
    }

    /// Builds the post-write [`Lease`] view for a write this coordinator just
    /// made (so its `region`/`holder_id` are this coordinator's identity).
    fn own_lease(&self, epoch: Epoch, expires_at_ms: u64) -> Result<Lease, LeaseError> {
        Ok(Lease {
            region: self.region.clone(),
            holder_id: self.identity.clone(),
            epoch,
            expires_at: from_unix_millis(expires_at_ms)
                .ok_or_else(|| malformed("expires_at is outside the representable range"))?,
        })
    }

    /// Reads the current lease.
    ///
    /// # Errors
    ///
    /// - [`LeaseError::NoLease`] if no lease item exists yet (bootstrap).
    /// - [`LeaseError::MalformedLease`] if the item violates the §8.2 schema.
    /// - [`LeaseError::Backend`] on a transport/service failure.
    pub async fn read_lease(&self) -> Result<Lease, LeaseError> {
        match self.kv.get(&self.key).await {
            Ok(item) => decode_lease(item),
            Err(CloudError::NotFound { key }) => Err(LeaseError::NoLease { key }),
            Err(other) => Err(LeaseError::Backend(other)),
        }
    }

    /// Acquires the lease from the unheld (no-item) state, recording
    /// [`INITIAL_EPOCH`] and an expiry one [`LEASE_TTL`] ahead.
    ///
    /// Insert-only ([`Condition::NotExists`]): if any lease already exists —
    /// even an expired one — this fails with [`LeaseError::LeaseHeld`] and the
    /// caller must [`claim_lease`](Self::claim_lease) instead. Concurrent
    /// bootstrap acquirers race on the insert; exactly one wins.
    ///
    /// # Errors
    ///
    /// - [`LeaseError::LeaseHeld`] if a lease item already exists.
    /// - [`LeaseError::Backend`] on a transport/service failure.
    pub async fn acquire(&self) -> Result<Lease, LeaseError> {
        let expires_at_ms = self.now_millis().saturating_add(duration_millis(LEASE_TTL));
        let value = encode_lease(&self.region, &self.identity, INITIAL_EPOCH, expires_at_ms);
        match self.kv.put(&self.key, value, &[Condition::NotExists]).await {
            Ok(()) => self.own_lease(INITIAL_EPOCH, expires_at_ms),
            Err(CloudError::ConditionFailed { .. }) => Err(LeaseError::LeaseHeld),
            Err(other) => Err(LeaseError::Backend(other)),
        }
    }

    /// Renews the lease this coordinator holds at `epoch`, extending expiry to
    /// one [`LEASE_TTL`] ahead of now (spec §8.3: renewed every 20s, 60s TTL).
    ///
    /// Conditioned on both `holder_id` **and** `epoch` still matching (spec
    /// §8.3): the epoch is unchanged, only `expires_at` advances. If a takeover
    /// has occurred, the epoch (and holder) no longer match and this fails,
    /// telling the demoted primary to stand down.
    ///
    /// # Errors
    ///
    /// - [`LeaseError::LostLease`] if the holder id or epoch no longer matches
    ///   (another region took over).
    /// - [`LeaseError::NoLease`] if the lease item is absent.
    /// - [`LeaseError::MalformedLease`] if the stored item violates the schema.
    /// - [`LeaseError::Backend`] on a transport/service failure.
    pub async fn renew(&self, epoch: Epoch) -> Result<Lease, LeaseError> {
        let expires_at_ms = self.now_millis().saturating_add(duration_millis(LEASE_TTL));
        let expr = UpdateExpression::new().set(ATTR_EXPIRES_AT, Value::U64(expires_at_ms));
        let conditions = [
            Condition::AttributeEquals {
                attribute: ATTR_HOLDER_ID.to_string(),
                expected: Value::String(self.identity.as_str().to_string()),
            },
            epoch_condition(epoch),
        ];
        match self.kv.atomic_update(&self.key, expr, &conditions).await {
            Ok(item) => decode_lease(item),
            Err(CloudError::ConditionFailed { .. }) => Err(LeaseError::LostLease),
            Err(CloudError::NotFound { key }) => Err(LeaseError::NoLease { key }),
            Err(other) => Err(LeaseError::Backend(other)),
        }
    }

    /// Takes over an expired lease, atomically advancing the fencing epoch
    /// (spec §8.3: "Every takeover atomically increments `epoch`").
    ///
    /// Reads the lease for its current epoch, checks it is expired past the
    /// [`TAKEOVER_SAFETY_MARGIN`] against the injected clock, then installs
    /// this coordinator as holder at epoch `E+1` in one
    /// [`ReplicatedKv::atomic_update`] guarded by [`epoch_condition`] on the
    /// observed epoch `E`.
    /// That single conditional write is the linearization point: concurrent
    /// challengers all guard on the same `E`, so exactly one wins.
    ///
    /// # Errors
    ///
    /// - [`LeaseError::LeaseHeld`] if the lease has not expired past the safety
    ///   margin (not yet takeover-eligible).
    /// - [`LeaseError::EpochAdvanced`] if the epoch changed between the read
    ///   and the write (a concurrent challenger won).
    /// - [`LeaseError::NoLease`] if no lease exists to take over (acquire
    ///   first).
    /// - [`LeaseError::EpochOverflow`] if the epoch is already `u64::MAX`.
    /// - [`LeaseError::MalformedLease`] if the stored item violates the schema.
    /// - [`LeaseError::Backend`] on a transport/service failure.
    pub async fn claim_lease(&self) -> Result<Lease, LeaseError> {
        // 1. Observe the current lease — we need its epoch to fence the CAS.
        //    A missing item is `NoLease`: there is nothing to take over.
        let current = self.read_lease().await?;

        // 2. Eligibility: expired past the safety margin, judged by the
        //    injected clock (spec §8.3). Not yet eligible -> the lease is still
        //    validly held.
        let now_ms = self.now_millis();
        let expires_at_ms = to_unix_millis(current.expires_at);
        if !takeover_eligible_millis(
            now_ms,
            expires_at_ms,
            duration_millis(TAKEOVER_SAFETY_MARGIN),
        ) {
            return Err(LeaseError::LeaseHeld);
        }

        // 3. Advance the epoch. Expressed as a conditional Set to the checked
        //    successor rather than a raw Increment: identical atomic effect
        //    under `epoch_condition(E)` (E -> E+1, single winner), and it lets
        //    the unreachable u64 overflow surface as a typed error instead of
        //    an opaque backend ConditionFailed (rule no-unwrap-in-prod).
        let next_epoch = current.epoch.checked_next()?;
        let new_expires_at_ms = now_ms.saturating_add(duration_millis(LEASE_TTL));
        let expr = UpdateExpression::new()
            .set(ATTR_EPOCH, Value::U64(next_epoch.0))
            .set(
                ATTR_HOLDER_ID,
                Value::String(self.identity.as_str().to_string()),
            )
            .set(ATTR_REGION, Value::String(self.region.as_str().to_string()))
            .set(ATTR_EXPIRES_AT, Value::U64(new_expires_at_ms));

        // 4. One atomic conditional write guarded by the observed epoch.
        match self
            .kv
            .atomic_update(&self.key, expr, &[epoch_condition(current.epoch)])
            .await
        {
            Ok(item) => decode_lease(item),
            Err(CloudError::ConditionFailed { .. }) => Err(LeaseError::EpochAdvanced),
            Err(CloudError::NotFound { key }) => Err(LeaseError::NoLease { key }),
            Err(other) => Err(LeaseError::Backend(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_lease, encode_lease, epoch_condition, from_unix_millis, lease_key, require_u64,
        takeover_eligible_millis, to_unix_millis, ATTR_EPOCH, ATTR_EXPIRES_AT,
    };
    use cloud_types::{Condition, Item, Key, Value};
    use mtc::{Epoch, LogId};
    use std::collections::BTreeMap;
    use std::time::{Duration, UNIX_EPOCH};

    use crate::ids::{HolderId, Region};

    #[test]
    fn lease_key_renders_pk_and_sk() {
        let log = LogId::new("prod-1").unwrap();
        assert_eq!(lease_key(&log), Key::new("log#prod-1/primary-region-lease"));
    }

    #[test]
    fn epoch_condition_targets_the_epoch_attribute() {
        assert_eq!(
            epoch_condition(Epoch(9)),
            Condition::AttributeEquals {
                attribute: ATTR_EPOCH.to_string(),
                expected: Value::U64(9),
            }
        );
    }

    #[test]
    fn takeover_eligibility_respects_the_margin() {
        // expires at 1000ms, margin 60_000ms -> eligible only at/after 61_000.
        assert!(!takeover_eligible_millis(1_000, 1_000, 60_000)); // exactly expired, within margin
        assert!(!takeover_eligible_millis(60_999, 1_000, 60_000)); // one ms short
        assert!(takeover_eligible_millis(61_000, 1_000, 60_000)); // exactly at the boundary
        assert!(takeover_eligible_millis(999_999, 1_000, 60_000));
        // saturating: never panics even at the extremes.
        assert!(!takeover_eligible_millis(u64::MAX - 1, u64::MAX, 60_000));
        assert!(takeover_eligible_millis(u64::MAX, 0, u64::MAX));
    }

    #[test]
    fn millis_round_trip() {
        let t = UNIX_EPOCH + Duration::from_millis(1_700_000_123_456);
        assert_eq!(to_unix_millis(t), 1_700_000_123_456);
        assert_eq!(from_unix_millis(1_700_000_123_456), Some(t));
        // Before the epoch saturates to 0.
        assert_eq!(to_unix_millis(UNIX_EPOCH), 0);
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let value = encode_lease(
            &Region::new("us-east-1"),
            &HolderId::new("inst-1"),
            Epoch(4),
            123_000,
        );
        let lease = decode_lease(Item {
            key: Key::new("k"),
            value,
        })
        .expect("well-formed");
        assert_eq!(lease.region, Region::new("us-east-1"));
        assert_eq!(lease.holder_id, HolderId::new("inst-1"));
        assert_eq!(lease.epoch, Epoch(4));
        assert_eq!(lease.expires_at, from_unix_millis(123_000).unwrap());
    }

    #[test]
    fn decode_rejects_non_map_item() {
        let err = decode_lease(Item {
            key: Key::new("k"),
            value: Value::U64(1),
        })
        .unwrap_err();
        assert!(matches!(err, crate::LeaseError::MalformedLease { .. }));
    }

    #[test]
    fn decode_rejects_missing_and_mistyped_attributes() {
        // Missing expires_at.
        let mut map = BTreeMap::new();
        map.insert("region".to_string(), Value::String("r".to_string()));
        map.insert("holder_id".to_string(), Value::String("h".to_string()));
        map.insert(ATTR_EPOCH.to_string(), Value::U64(1));
        let err = decode_lease(Item {
            key: Key::new("k"),
            value: Value::Map(map.clone()),
        })
        .unwrap_err();
        assert!(matches!(err, crate::LeaseError::MalformedLease { .. }));

        // Wrong type for epoch.
        map.insert(ATTR_EXPIRES_AT.to_string(), Value::U64(1));
        map.insert(ATTR_EPOCH.to_string(), Value::String("nope".to_string()));
        let err = require_u64(&map, ATTR_EPOCH).unwrap_err();
        assert!(matches!(err, crate::LeaseError::MalformedLease { .. }));
    }
}
