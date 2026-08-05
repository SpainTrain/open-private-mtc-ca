//! [`RetentionPolicy`]: computes `retain_until` for the storage object
//! classes named in spec §15 — entries, tiles, and pruning checkpoints
//! (spec §8.1 S3 layout).

use std::time::{Duration, SystemTime};

use thiserror::Error;

use crate::duration::RetentionDuration;

/// Storage object classes subject to retention (spec §8.1 S3 layout, §15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectClass {
    /// A `TBSCertificateLogEntry` object under the `entries/` prefix.
    Entry,
    /// A Merkle-tree tile object under the `tiles/` prefix.
    Tile,
    /// A signed pruning-checkpoint declaration under the `checkpoints/`
    /// prefix (spec §15.2). Retained indefinitely regardless of the
    /// configured entry/tile duration (spec §15.3) — see
    /// [`RetentionPolicy::retain_until`].
    PruningCheckpoint,
}

/// The distinguished "indefinite" retention instant used for
/// [`ObjectClass::PruningCheckpoint`] (spec §15.3: "Pruning checkpoints
/// retained indefinitely").
///
/// # Why a fixed far-future instant, not "as far as representable"
///
/// [`SystemTime`] has no infinity value. The naive encoding of "indefinite" —
/// `write_time + Duration::MAX` — does not saturate; it overflows the
/// platform's bounded time representation and [`SystemTime::checked_add`]
/// returns `None` (verified in this module's tests). So "indefinite" cannot
/// mean "as far in the future as the type allows" without every call site
/// having to handle that overflow specially.
///
/// Instead this pins a fixed calendar instant: `9999-12-31T23:59:59Z`,
/// i.e. `253_402_300_799` seconds after the Unix epoch — the same
/// "end of representable calendar time" ceiling used by common date
/// libraries (e.g. `chrono::NaiveDate::MAX`, RFC 3339's four-digit-year
/// ceiling). It is comfortably beyond any plausible retention horizon (the
/// production default is 7 years) while staying representable everywhere a
/// realistic `SystemTime` is, so `ObjectLock::put_with_retention` (spec
/// §9.1, `crates/cloud-types`) accepts it directly with no special-casing at
/// call sites.
#[must_use]
pub fn indefinite_retention() -> SystemTime {
    /// `9999-12-31T23:59:59Z` in seconds since the Unix epoch.
    const INDEFINITE_RETENTION_SECS: u64 = 253_402_300_799;
    SystemTime::UNIX_EPOCH + Duration::from_secs(INDEFINITE_RETENTION_SECS)
}

/// Errors computing [`RetentionPolicy::retain_until`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RetentionError {
    /// `write_time + duration` does not fit the platform's [`SystemTime`]
    /// representation.
    ///
    /// Practically unreachable for realistic write times and the
    /// multi-year durations this policy configures (64-bit `SystemTime`
    /// implementations represent roughly ±292 billion years from the Unix
    /// epoch) — but callers get a typed error instead of a panic (rule
    /// `no-unwrap-in-prod`) rather than this crate assuming the platform's
    /// range on their behalf.
    #[error("retain_until overflowed: write_time {write_time:?} + duration {duration:?}")]
    Overflow {
        /// The write time that was supplied.
        write_time: SystemTime,
        /// The configured entry/tile retention duration.
        duration: Duration,
    },
}

/// Configured retention policy: how long entries and tiles are retained
/// before they become eligible for pruning (spec §15.1).
///
/// Pruning checkpoints ignore this duration and are always retained
/// indefinitely (spec §15.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    entry_tile_duration: RetentionDuration,
}

impl RetentionPolicy {
    /// Builds a policy directly from an already-validated
    /// [`RetentionDuration`].
    ///
    /// Prefer
    /// [`RetentionPolicyConfig::build`](crate::RetentionPolicyConfig::build)
    /// when parsing from service config — it also applies the 7-year
    /// default and the dev-mode override. This constructor is for callers
    /// that already hold a validated duration (tests, or code composing a
    /// policy from a value validated elsewhere).
    #[must_use]
    pub const fn from_duration(entry_tile_duration: RetentionDuration) -> Self {
        Self {
            entry_tile_duration,
        }
    }

    /// The configured entry/tile retention window.
    #[must_use]
    pub const fn entry_tile_duration(&self) -> RetentionDuration {
        self.entry_tile_duration
    }

    /// Computes `retain_until` for an object of the given `class` written at
    /// `write_time` — the value passed as the `retain_until` argument of
    /// `ObjectLock::put_with_retention` (spec §9.1, `crates/cloud-types`).
    ///
    /// - [`ObjectClass::Entry`] / [`ObjectClass::Tile`]: `write_time` plus
    ///   the configured duration.
    /// - [`ObjectClass::PruningCheckpoint`][]: always
    ///   [`indefinite_retention`], irrespective of `write_time` or the
    ///   configured duration (spec §15.3).
    ///
    /// `write_time` is supplied by the caller — typically the value most
    /// recently read from an injected `Arc<dyn clock::Clock>` at the moment
    /// the object is written. This function never reads wall-clock time
    /// itself (rule `no-systemtime-now-in-prod`, spec §22.11).
    ///
    /// # Errors
    ///
    /// [`RetentionError::Overflow`] if `write_time + duration` cannot be
    /// represented — see that variant's docs (practically unreachable for
    /// realistic inputs).
    pub fn retain_until(
        &self,
        class: ObjectClass,
        write_time: SystemTime,
    ) -> Result<SystemTime, RetentionError> {
        match class {
            ObjectClass::PruningCheckpoint => Ok(indefinite_retention()),
            ObjectClass::Entry | ObjectClass::Tile => {
                let duration = self.entry_tile_duration.as_duration();
                write_time
                    .checked_add(duration)
                    .ok_or(RetentionError::Overflow {
                        write_time,
                        duration,
                    })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_with_days(days: i64) -> RetentionPolicy {
        RetentionPolicy::from_duration(RetentionDuration::from_days(days).expect("valid"))
    }

    #[test]
    fn entry_retain_until_is_write_time_plus_duration() {
        let policy = policy_with_days(7);
        let write_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let expected = write_time + Duration::from_hours(7 * 24);
        assert_eq!(
            policy
                .retain_until(ObjectClass::Entry, write_time)
                .expect("no overflow"),
            expected
        );
    }

    #[test]
    fn tile_retain_until_matches_entry_retain_until() {
        let policy = policy_with_days(30);
        let write_time = SystemTime::UNIX_EPOCH + Duration::from_secs(5_000);
        assert_eq!(
            policy.retain_until(ObjectClass::Tile, write_time),
            policy.retain_until(ObjectClass::Entry, write_time)
        );
    }

    #[test]
    fn pruning_checkpoint_is_indefinite_regardless_of_write_time_or_duration() {
        let short = policy_with_days(1);
        let long = policy_with_days(3650);
        let early = SystemTime::UNIX_EPOCH;
        let late = SystemTime::UNIX_EPOCH + Duration::from_secs(999_999_999);
        for policy in [&short, &long] {
            for write_time in [early, late] {
                assert_eq!(
                    policy
                        .retain_until(ObjectClass::PruningCheckpoint, write_time)
                        .expect("indefinite retention never overflows"),
                    indefinite_retention()
                );
            }
        }
    }

    #[test]
    fn indefinite_retention_is_the_documented_sentinel() {
        assert_eq!(
            indefinite_retention(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(253_402_300_799)
        );
    }

    #[test]
    fn indefinite_retention_is_far_beyond_a_seven_year_default() {
        let policy = policy_with_days(2555); // 7 years
        let write_time = SystemTime::UNIX_EPOCH;
        let entry_retain_until = policy
            .retain_until(ObjectClass::Entry, write_time)
            .expect("no overflow");
        let checkpoint_retain_until = policy
            .retain_until(ObjectClass::PruningCheckpoint, write_time)
            .expect("no overflow");
        assert!(checkpoint_retain_until > entry_retain_until);
    }

    #[test]
    fn write_time_plus_duration_max_overflows_rather_than_saturates() {
        // Justifies why `indefinite_retention` is a fixed sentinel and not
        // `write_time + Duration::MAX` (see the function's rustdoc).
        let write_time = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
        assert_eq!(write_time.checked_add(Duration::MAX), None);
    }

    #[test]
    fn retain_until_overflow_is_a_typed_error_not_a_panic() {
        let policy = policy_with_days(1);
        // The furthest representable instant from `UNIX_EPOCH` on this
        // platform's `SystemTime`; adding even one more day must overflow.
        let far_future = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::MAX / 2);
        let one_day = Duration::from_hours(24);
        let err = far_future.checked_add(one_day);
        assert_eq!(err, None, "test setup must actually overflow");

        let result = policy.retain_until(ObjectClass::Entry, far_future);
        assert_eq!(
            result,
            Err(RetentionError::Overflow {
                write_time: far_future,
                duration: one_day,
            })
        );
    }
}
