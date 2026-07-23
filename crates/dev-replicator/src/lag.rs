//! Pure, infra-free lag scheduling and idempotent dedup for one replication
//! link (spec §18.3 "configurable lag").
//!
//! [`LagScheduler`] knows nothing about S3 or `DynamoDB`: it takes a stream of
//! `(dedup key, payload, discovered_at)` triples and decides which are due to
//! apply at a given `now`, honoring the current [`LagPolicy`]. Both the S3
//! and `DynamoDB` pollers (`s3.rs`, `ddb.rs`) are thin IO adapters around one
//! of these — the ordering, dedup, and lag-timing logic is written and tested
//! exactly once here.
//!
//! # Idempotency (ticket dev-crr-replication-sim, property test requirement)
//!
//! A key is applied at most once: [`LagScheduler::discover`] is a no-op for a
//! key that is already pending or already applied, and [`LagScheduler::drain_due`]
//! marks every drained key as applied. Replaying the exact same discovery
//! stream twice therefore produces the same drained sequence on the first
//! pass and nothing on the second — see `proptest_replaying_the_same_stream_is_idempotent`.
//!
//! # Lag policy changes and the "catch-up burst"
//!
//! [`LagPolicy::Stalled`] (the mr-replication-sim "infinite lag" requirement)
//! never drains anything — items keep queuing (observable via
//! [`LagScheduler::pending_len`] / [`LagScheduler::oldest_pending_age`] for
//! the control endpoint's `/status`), but nothing is ever due while stalled.
//! When the policy moves back to [`LagPolicy::Fixed`], every item queued
//! during the stall is immediately due (its age already exceeds the new
//! lag), so the next `drain_due` releases them all at once, in original
//! discovery order — a deliberate, documented simulation of CRR "catching
//! up" after a stall (spec §19.9 `chaos-crr-stall`).

use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

/// How long a discovered change sits before [`LagScheduler::drain_due`]
/// releases it.
///
/// Runtime-adjustable per link (mr-replication-sim AC: "Lag configurable per
/// link at runtime, including stall"). `Stalled` is the infinite-lag case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LagPolicy {
    /// Release a discovered item `lag` after it was discovered.
    Fixed(Duration),
    /// Never release anything until the policy changes (infinite lag).
    Stalled,
}

impl LagPolicy {
    /// The zero-lag policy: items are due as soon as they are discovered.
    #[must_use]
    pub const fn immediate() -> Self {
        Self::Fixed(Duration::ZERO)
    }
}

impl Default for LagPolicy {
    /// Zero lag by default — a scheduler with no configured lag behaves
    /// transparently (every discovery is immediately due).
    fn default() -> Self {
        Self::immediate()
    }
}

/// One discovered source-side change, tagged with when it was first noticed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered<K, E> {
    /// Idempotency / ordering key (e.g. `(object key, version id)` for S3,
    /// `(pk, sk)` for `DynamoDB`).
    pub key: K,
    /// The payload to apply once due.
    pub event: E,
    /// When the replicator's poller first observed this change (its own
    /// clock — see the `clock` crate dependency — not the source's reported
    /// write time; see the crate-level docs' "known limitations" section).
    pub at: SystemTime,
}

/// Lag scheduling and idempotent dedup for one resource on one link (either
/// the S3 side or the `DynamoDB` side — a [`crate::link::Link`] owns one of
/// each).
///
/// Generic over the dedup key `K` and payload `E` so the exact same logic is
/// used, and tested, for both resource kinds (§22.7 — pure logic favors
/// generics; the concrete `K`/`E` are resolved once per call site).
#[derive(Debug)]
pub struct LagScheduler<K, E> {
    policy: LagPolicy,
    pending: VecDeque<Discovered<K, E>>,
    applied: std::collections::HashSet<K>,
}

impl<K: Clone + Eq + std::hash::Hash, E> LagScheduler<K, E> {
    /// Creates a scheduler starting with `policy`.
    #[must_use]
    pub fn new(policy: LagPolicy) -> Self {
        Self {
            policy,
            pending: VecDeque::new(),
            applied: std::collections::HashSet::new(),
        }
    }

    /// The scheduler's current lag policy.
    #[must_use]
    pub const fn policy(&self) -> LagPolicy {
        self.policy
    }

    /// Updates the lag policy, effective on the next [`Self::drain_due`]
    /// call. Does not itself release or drop anything already pending.
    pub const fn set_policy(&mut self, policy: LagPolicy) {
        self.policy = policy;
    }

    /// Records a newly discovered change. Returns `true` if it was newly
    /// queued, `false` if the call was a no-op.
    ///
    /// No-op (idempotent) if `key` is already pending or was already
    /// applied by an earlier [`Self::drain_due`] — re-discovering the same
    /// key (e.g. re-listing an S3 version the poller already replicated)
    /// never re-queues it.
    pub fn discover(&mut self, key: K, event: E, discovered_at: SystemTime) -> bool {
        if self.applied.contains(&key) {
            return false;
        }
        if self.pending.iter().any(|d| d.key == key) {
            return false;
        }
        self.pending.push_back(Discovered {
            key,
            event,
            at: discovered_at,
        });
        true
    }

    /// Drains every pending item whose lag has elapsed as of `now`, in FIFO
    /// (discovery) order, marking each as applied.
    ///
    /// Under [`LagPolicy::Stalled`] this always returns an empty `Vec` —
    /// nothing is ever due — without dropping anything from the pending
    /// queue.
    pub fn drain_due(&mut self, now: SystemTime) -> Vec<Discovered<K, E>> {
        let LagPolicy::Fixed(lag) = self.policy else {
            return Vec::new();
        };
        let mut due = Vec::new();
        while let Some(front) = self.pending.front() {
            if now.duration_since(front.at).unwrap_or_default() < lag {
                break;
            }
            if let Some(item) = self.pending.pop_front() {
                self.applied.insert(item.key.clone());
                due.push(item);
            }
        }
        due
    }

    /// Number of items discovered but not yet drained.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Age of the oldest pending item as of `now`, if any is pending — the
    /// observable "how far behind is this link" signal for the control
    /// endpoint's `/status` (mr-replication-sim AC: "replication position
    /// observable for readiness checks").
    #[must_use]
    pub fn oldest_pending_age(&self, now: SystemTime) -> Option<Duration> {
        self.pending
            .front()
            .map(|d| now.duration_since(d.at).unwrap_or_default())
    }

    /// Total number of items ever drained (applied) by this scheduler.
    #[must_use]
    pub fn applied_len(&self) -> usize {
        self.applied.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn t(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn immediate_policy_drains_everything_discovered_so_far() {
        let mut sched: LagScheduler<&str, &str> = LagScheduler::new(LagPolicy::immediate());
        sched.discover("a", "event-a", t(0));
        sched.discover("b", "event-b", t(0));

        let due = sched.drain_due(t(0));
        assert_eq!(due.iter().map(|d| d.key).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn fixed_lag_withholds_until_lag_elapses() {
        let mut sched: LagScheduler<&str, ()> =
            LagScheduler::new(LagPolicy::Fixed(Duration::from_secs(5)));
        sched.discover("a", (), t(0));

        assert!(sched.drain_due(t(4)).is_empty(), "not due yet at t=4");
        let due = sched.drain_due(t(5));
        assert_eq!(due.len(), 1, "due exactly at t=5 (lag elapsed)");
    }

    #[test]
    fn stalled_policy_never_drains_but_keeps_queuing() {
        let mut sched: LagScheduler<&str, ()> = LagScheduler::new(LagPolicy::Stalled);
        sched.discover("a", (), t(0));
        sched.discover("b", (), t(100));

        assert!(sched.drain_due(t(1_000_000)).is_empty());
        assert_eq!(sched.pending_len(), 2);
    }

    #[test]
    fn unstalling_releases_a_catchup_burst_in_discovery_order() {
        let mut sched: LagScheduler<&str, ()> = LagScheduler::new(LagPolicy::Stalled);
        sched.discover("a", (), t(0));
        sched.discover("b", (), t(10));
        sched.discover("c", (), t(20));
        assert!(sched.drain_due(t(1000)).is_empty());

        sched.set_policy(LagPolicy::Fixed(Duration::from_secs(5)));
        let due = sched.drain_due(t(1000));
        assert_eq!(
            due.iter().map(|d| d.key).collect::<Vec<_>>(),
            ["a", "b", "c"],
            "all three were already older than the new 5s lag: released together, in order"
        );
    }

    #[test]
    fn rediscovering_a_pending_key_does_not_duplicate_it() {
        let mut sched: LagScheduler<&str, &str> = LagScheduler::new(LagPolicy::immediate());
        sched.discover("a", "first", t(0));
        sched.discover("a", "second", t(1)); // e.g. re-listed before it drained

        let due = sched.drain_due(t(1));
        assert_eq!(due.len(), 1);
        assert_eq!(
            due[0].event, "first",
            "the original event wins, not the re-discovery"
        );
    }

    #[test]
    fn rediscovering_an_already_applied_key_is_a_no_op() {
        let mut sched: LagScheduler<&str, ()> = LagScheduler::new(LagPolicy::immediate());
        sched.discover("a", (), t(0));
        assert_eq!(sched.drain_due(t(0)).len(), 1);

        sched.discover("a", (), t(5)); // re-discovered, e.g. still present in a later listing
        assert!(
            sched.drain_due(t(5)).is_empty(),
            "already-applied keys never re-queue"
        );
        assert_eq!(sched.pending_len(), 0);
    }

    #[test]
    fn ordering_is_preserved_across_out_of_order_discovery_calls() {
        // Discovery order (call order), not key order, determines drain order.
        let mut sched: LagScheduler<u32, ()> = LagScheduler::new(LagPolicy::immediate());
        sched.discover(3, (), t(0));
        sched.discover(1, (), t(0));
        sched.discover(2, (), t(0));

        let due = sched.drain_due(t(0));
        assert_eq!(due.iter().map(|d| d.key).collect::<Vec<_>>(), [3, 1, 2]);
    }

    #[test]
    fn oldest_pending_age_reports_none_when_empty_and_grows_otherwise() {
        let mut sched: LagScheduler<&str, ()> = LagScheduler::new(LagPolicy::Stalled);
        assert_eq!(sched.oldest_pending_age(t(0)), None);

        sched.discover("a", (), t(10));
        assert_eq!(
            sched.oldest_pending_age(t(15)),
            Some(Duration::from_secs(5))
        );
    }

    proptest! {
        /// Ticket testing AC: "applying the same event stream twice is
        /// idempotent." Feed the identical `(key, discovered_at)` stream
        /// through two schedulers, one running it once and one running it
        /// twice back-to-back; both end up in the same state and the second
        /// pass drains nothing new.
        #[test]
        fn replaying_the_same_stream_is_idempotent(
            keys in prop::collection::vec(0u8..20, 1..30),
        ) {
            let mut once: LagScheduler<u8, ()> = LagScheduler::new(LagPolicy::immediate());
            for &k in &keys {
                once.discover(k, (), t(0));
            }
            let first_pass = once.drain_due(t(0));

            let mut twice: LagScheduler<u8, ()> = LagScheduler::new(LagPolicy::immediate());
            for &k in &keys {
                twice.discover(k, (), t(0));
            }
            let _ = twice.drain_due(t(0));
            for &k in &keys {
                twice.discover(k, (), t(1)); // replay the exact same discoveries
            }
            let second_pass = twice.drain_due(t(1));

            prop_assert!(second_pass.is_empty(), "replayed stream must not re-apply anything");
            prop_assert_eq!(once.applied_len(), twice.applied_len());
            prop_assert_eq!(
                first_pass.len(),
                once.applied_len(),
                "every unique key drains exactly once"
            );
        }

        /// Discovery order determines drain order under a constant policy,
        /// regardless of how many distinct keys are interleaved.
        #[test]
        fn drain_order_matches_discovery_order(
            keys in prop::collection::vec(0u8..50, 1..40),
        ) {
            // De-duplicate while preserving first-seen order, mirroring what
            // the scheduler itself does internally.
            let mut seen = std::collections::HashSet::new();
            let expected: Vec<u8> = keys.iter().copied().filter(|k| seen.insert(*k)).collect();

            let mut sched: LagScheduler<u8, ()> = LagScheduler::new(LagPolicy::immediate());
            for &k in &keys {
                sched.discover(k, (), t(0));
            }
            let due = sched.drain_due(t(0));

            prop_assert_eq!(due.into_iter().map(|d| d.key).collect::<Vec<_>>(), expected);
        }
    }
}
