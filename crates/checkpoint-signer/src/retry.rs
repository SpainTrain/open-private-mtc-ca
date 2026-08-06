//! The retry policy and exponential backoff schedule for transient HSM
//! failures (spec §11.3 row 7: "Retry with backoff; alert if persistent").
//!
//! The schedule is a **pure, clock-free** function of the retry index — the
//! injected [`Clock`](clock::Clock) does the actual waiting in the signer (rule
//! `no-systemtime-now-in-prod`) — so it is deterministically testable without
//! any time source.

use std::time::Duration;

/// How the checkpoint signer retries a **transient** HSM failure before giving
/// up (spec §11.3 row 7).
///
/// Only `CloudError::Transport { retryable: true }` is retried; every other
/// error is terminal and surfaces immediately (see
/// [`CloudError::is_retryable`](cloud_types::CloudError::is_retryable)). The
/// backoff is exponential (`base_backoff * 2^retry_index`) capped at
/// `max_backoff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Maximum number of retries **after** the first attempt. `0` disables
    /// retry entirely (the first transient failure surfaces immediately).
    pub max_retries: u32,
    /// The first backoff delay; each subsequent retry doubles it up to
    /// [`max_backoff`](Self::max_backoff).
    pub base_backoff: Duration,
    /// Upper bound on any single backoff delay.
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    /// Five retries, a 50 ms base doubling to a 5 s cap. The happy path adds no
    /// delay (backoff only runs on failure), keeping a successful sign well
    /// inside the <100 ms p99 target (spec §14.3) while still riding out a brief
    /// HSM blip.
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_secs(5),
        }
    }
}

impl RetryPolicy {
    /// The backoff delay before retry number `retry_index` (0-based): an
    /// exponential `base_backoff * 2^retry_index`, saturating and capped at
    /// `max_backoff`.
    ///
    /// Pure — no clock, no randomness — so the schedule is deterministically
    /// testable. Arithmetic is done in `u128` nanoseconds and the doubling
    /// shift is clamped so it can never overflow or panic; any index past 63
    /// already saturates far beyond `max_backoff`.
    #[must_use]
    pub fn backoff_for(&self, retry_index: u32) -> Duration {
        let base_nanos = self.base_backoff.as_nanos();
        let doublings: u128 = 1 << retry_index.min(63);
        let scaled = base_nanos.saturating_mul(doublings);
        let capped = scaled.min(self.max_backoff.as_nanos());
        // `capped <= max_backoff.as_nanos()`, which came from a `Duration`, so
        // it always fits a `u64`; `unwrap_or` keeps this total without a panic.
        Duration::from_nanos(u64::try_from(capped).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::RetryPolicy;
    use std::time::Duration;

    fn policy() -> RetryPolicy {
        RetryPolicy {
            max_retries: 5,
            base_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_secs(5),
        }
    }

    #[test]
    fn backoff_doubles_from_the_base() {
        let p = policy();
        assert_eq!(p.backoff_for(0), Duration::from_millis(50));
        assert_eq!(p.backoff_for(1), Duration::from_millis(100));
        assert_eq!(p.backoff_for(2), Duration::from_millis(200));
        assert_eq!(p.backoff_for(3), Duration::from_millis(400));
    }

    #[test]
    fn backoff_saturates_at_the_cap() {
        let p = policy();
        // 50ms * 2^7 = 6400ms, past the 5s cap.
        assert_eq!(p.backoff_for(7), Duration::from_secs(5));
        // A huge index cannot overflow or panic; it stays at the cap.
        assert_eq!(p.backoff_for(1000), Duration::from_secs(5));
        assert_eq!(p.backoff_for(u32::MAX), Duration::from_secs(5));
    }

    #[test]
    fn default_policy_matches_the_documented_schedule() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_retries, 5);
        assert_eq!(p.backoff_for(0), Duration::from_millis(50));
        assert_eq!(p.max_backoff, Duration::from_secs(5));
    }
}
