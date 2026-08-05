//! [`RetentionDuration`]: a validated, strictly-positive retention window.
//!
//! Newtype per rule `use-newtypes` (spec §22.1): wrapping [`Duration`]
//! directly would let an unvalidated (possibly zero) duration flow into a
//! [`RetentionPolicy`](crate::RetentionPolicy) unchecked. Every
//! [`RetentionDuration`] in existence has already passed the "reject
//! zero/negative durations" check (ticket `prune-retention-policy` AC), so
//! holding one is proof the value was validated.

use std::time::Duration;

use thiserror::Error;

/// A validated, strictly-positive retention window.
///
/// Constructed only through [`RetentionDuration::from_days`] or
/// [`RetentionDuration::from_minutes`], both of which reject zero and
/// negative inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RetentionDuration(Duration);

/// Errors constructing a [`RetentionDuration`] — and, by extension, errors
/// building a [`RetentionPolicy`](crate::RetentionPolicy) from
/// [`RetentionPolicyConfig`](crate::RetentionPolicyConfig).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RetentionConfigError {
    /// `retention_days` was zero or negative.
    #[error("retention_days must be positive, got {0}")]
    NonPositiveDays(i64),
    /// `dev_override_minutes` was zero or negative.
    #[error("dev_override_minutes must be positive, got {0}")]
    NonPositiveMinutes(i64),
    /// The requested duration overflowed converting to seconds (an
    /// absurdly large `retention_days`/`dev_override_minutes`).
    #[error("duration overflowed converting {unit} value {value} to seconds")]
    DurationOverflow {
        /// Which field overflowed: `"retention_days"` or
        /// `"dev_override_minutes"`.
        unit: &'static str,
        /// The raw input value that overflowed.
        value: i64,
    },
}

const SECS_PER_DAY: i64 = 24 * 60 * 60;
const SECS_PER_MINUTE: i64 = 60;

impl RetentionDuration {
    /// Builds a retention window from a whole number of days — the unit the
    /// production `retention_days` config field is expressed in (spec
    /// §15.1: default 7 years).
    ///
    /// # Errors
    ///
    /// [`RetentionConfigError::NonPositiveDays`] if `days <= 0`;
    /// [`RetentionConfigError::DurationOverflow`] if `days` is so large the
    /// day-to-second conversion overflows.
    pub fn from_days(days: i64) -> Result<Self, RetentionConfigError> {
        if days <= 0 {
            return Err(RetentionConfigError::NonPositiveDays(days));
        }
        let secs =
            days.checked_mul(SECS_PER_DAY)
                .ok_or(RetentionConfigError::DurationOverflow {
                    unit: "retention_days",
                    value: days,
                })?;
        Ok(Self(Duration::from_secs(positive_i64_to_u64(secs))))
    }

    /// Builds a retention window from a whole number of minutes — the unit
    /// the dev-mode override is expressed in (spec §18.4: pruning demoable
    /// on a laptop with the fake clock).
    ///
    /// # Errors
    ///
    /// [`RetentionConfigError::NonPositiveMinutes`] if `minutes <= 0`;
    /// [`RetentionConfigError::DurationOverflow`] on an absurdly large input.
    pub fn from_minutes(minutes: i64) -> Result<Self, RetentionConfigError> {
        if minutes <= 0 {
            return Err(RetentionConfigError::NonPositiveMinutes(minutes));
        }
        let secs =
            minutes
                .checked_mul(SECS_PER_MINUTE)
                .ok_or(RetentionConfigError::DurationOverflow {
                    unit: "dev_override_minutes",
                    value: minutes,
                })?;
        Ok(Self(Duration::from_secs(positive_i64_to_u64(secs))))
    }

    /// The underlying [`Duration`].
    #[must_use]
    pub const fn as_duration(self) -> Duration {
        self.0
    }
}

/// Converts an `i64` already proven positive (both callers check `<= 0`
/// first) to `u64`. The `unwrap_or` fallback is unreachable in practice —
/// `i64::MAX` always fits `u64` — but keeps this free of `unwrap()`/
/// `expect()` (rule `no-unwrap-in-prod`) rather than asserting it.
fn positive_i64_to_u64(secs: i64) -> u64 {
    u64::try_from(secs).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_days_seven_years_matches_expected_seconds() {
        let d = RetentionDuration::from_days(2555).expect("valid");
        assert_eq!(d.as_duration(), Duration::from_hours(2555 * 24));
    }

    #[test]
    fn from_days_rejects_zero() {
        assert_eq!(
            RetentionDuration::from_days(0),
            Err(RetentionConfigError::NonPositiveDays(0))
        );
    }

    #[test]
    fn from_days_rejects_negative() {
        assert_eq!(
            RetentionDuration::from_days(-1),
            Err(RetentionConfigError::NonPositiveDays(-1))
        );
    }

    #[test]
    fn from_minutes_rejects_zero() {
        assert_eq!(
            RetentionDuration::from_minutes(0),
            Err(RetentionConfigError::NonPositiveMinutes(0))
        );
    }

    #[test]
    fn from_minutes_rejects_negative() {
        assert_eq!(
            RetentionDuration::from_minutes(-5),
            Err(RetentionConfigError::NonPositiveMinutes(-5))
        );
    }

    #[test]
    fn from_minutes_five_is_three_hundred_seconds() {
        let d = RetentionDuration::from_minutes(5).expect("valid");
        assert_eq!(d.as_duration(), Duration::from_mins(5));
    }

    #[test]
    fn from_days_overflow_is_reported() {
        let err = RetentionDuration::from_days(i64::MAX).expect_err("must overflow");
        assert_eq!(
            err,
            RetentionConfigError::DurationOverflow {
                unit: "retention_days",
                value: i64::MAX,
            }
        );
    }
}
