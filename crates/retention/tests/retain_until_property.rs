//! Property test (ticket `prune-retention-policy` testing AC): `retain_until`
//! is monotonic in write time.
//!
//! Run via `cargo test -p retention retention_policy` (matches this file's
//! test names) or `cargo test -p retention` for the whole crate.

use std::time::{Duration, SystemTime};

use proptest::prelude::*;
use retention::{ObjectClass, RetentionDuration, RetentionPolicy};

/// Write-time seconds-since-epoch bounded to within ~1000 years — far more
/// range than any realistic CA deployment needs, and safely clear of the
/// platform `SystemTime` overflow boundary this crate's unit tests probe
/// directly (`retain_until_overflow_is_a_typed_error_not_a_panic` in
/// `src/policy.rs`), so the property below exercises the ordinary
/// non-overflowing path deterministically rather than needing to special-case
/// [`retention::RetentionError::Overflow`] on every generated pair.
fn write_time_secs_strategy() -> impl Strategy<Value = u64> {
    0u64..31_536_000_000u64 // ~1000 years, in seconds
}

/// Retention durations bounded to 1 day..=100 years, in days.
fn retention_days_strategy() -> impl Strategy<Value = i64> {
    1i64..=36_500i64
}

fn object_class_strategy() -> impl Strategy<Value = ObjectClass> {
    prop_oneof![
        Just(ObjectClass::Entry),
        Just(ObjectClass::Tile),
        Just(ObjectClass::PruningCheckpoint),
    ]
}

fn write_time(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

proptest! {
    /// retention_policy: for any two write times `t1 <= t2`, `retain_until(t1)
    /// <= retain_until(t2)` — monotonic non-decreasing in write time, for
    /// every object class and every valid policy duration. Trivially holds
    /// for `PruningCheckpoint` (constant sentinel, spec §15.3); the
    /// substantive case is `Entry`/`Tile`, where `retain_until = write_time +
    /// duration` and addition by a fixed positive duration preserves order.
    #[test]
    fn retention_policy_retain_until_is_monotonic_in_write_time(
        days in retention_days_strategy(),
        class in object_class_strategy(),
        secs_a in write_time_secs_strategy(),
        secs_b in write_time_secs_strategy(),
    ) {
        let policy = RetentionPolicy::from_duration(
            RetentionDuration::from_days(days).expect("strategy yields valid days"),
        );
        let (earlier_secs, later_secs) = if secs_a <= secs_b {
            (secs_a, secs_b)
        } else {
            (secs_b, secs_a)
        };
        let earlier = write_time(earlier_secs);
        let later = write_time(later_secs);

        let retain_earlier = policy
            .retain_until(class, earlier)
            .expect("bounded write times never overflow");
        let retain_later = policy
            .retain_until(class, later)
            .expect("bounded write times never overflow");

        prop_assert!(retain_earlier <= retain_later);
    }

    /// retention_policy: strictly monotonic (not just non-decreasing) for
    /// the finite classes, since a strictly-positive duration is added to a
    /// strictly-later write time.
    #[test]
    fn retention_policy_finite_classes_are_strictly_monotonic_for_distinct_write_times(
        days in retention_days_strategy(),
        secs_a in write_time_secs_strategy(),
        secs_b in write_time_secs_strategy(),
    ) {
        prop_assume!(secs_a != secs_b);
        let policy = RetentionPolicy::from_duration(
            RetentionDuration::from_days(days).expect("strategy yields valid days"),
        );
        let (earlier_secs, later_secs) = if secs_a < secs_b {
            (secs_a, secs_b)
        } else {
            (secs_b, secs_a)
        };
        let earlier = write_time(earlier_secs);
        let later = write_time(later_secs);

        for class in [ObjectClass::Entry, ObjectClass::Tile] {
            let retain_earlier = policy
                .retain_until(class, earlier)
                .expect("bounded write times never overflow");
            let retain_later = policy
                .retain_until(class, later)
                .expect("bounded write times never overflow");
            prop_assert!(retain_earlier < retain_later);
        }
    }

    /// retention_policy: `PruningCheckpoint` is constant in write time
    /// (weakest form of monotonic — equal, never decreasing), independent of
    /// the configured duration.
    #[test]
    fn retention_policy_pruning_checkpoint_is_constant_in_write_time(
        days in retention_days_strategy(),
        secs_a in write_time_secs_strategy(),
        secs_b in write_time_secs_strategy(),
    ) {
        let policy = RetentionPolicy::from_duration(
            RetentionDuration::from_days(days).expect("strategy yields valid days"),
        );
        let time_a = write_time(secs_a);
        let time_b = write_time(secs_b);

        let retain_a = policy
            .retain_until(ObjectClass::PruningCheckpoint, time_a)
            .expect("indefinite retention never overflows");
        let retain_b = policy
            .retain_until(ObjectClass::PruningCheckpoint, time_b)
            .expect("indefinite retention never overflows");

        prop_assert_eq!(retain_a, retain_b);
        prop_assert_eq!(retain_a, retention::indefinite_retention());
    }
}
