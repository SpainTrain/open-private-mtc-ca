//! Minimal local time seam.
//!
//! The shared clock crate (`crates/clock`, ticket `ca-clock`, spec §22.11) is
//! being built in parallel with this ticket. Until it lands, this module is
//! the smallest possible injectable clock so nothing in this crate reads time
//! ambiently (rule `no-systemtime-now-in-prod`). Swapping to the shared
//! `Clock` trait later is a mechanical replacement of this module: the only
//! consumer is nonce expiry, which needs monotonic elapsed time, not wall
//! time.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Milliseconds read from a monotonic clock.
///
/// Only differences between two readings from the *same* [`Clock`] are
/// meaningful; the origin is arbitrary (newtype per rule `use-newtypes`).
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MonotonicMillis(pub u64);

impl MonotonicMillis {
    /// Adds `millis`, saturating at `u64::MAX`.
    #[must_use]
    pub fn saturating_add(self, millis: u64) -> Self {
        Self(self.0.saturating_add(millis))
    }
}

/// Injected time source (see module docs for the relationship to
/// `crates/clock`).
pub trait Clock: Send + Sync {
    /// Current reading of the monotonic clock.
    fn monotonic_now(&self) -> MonotonicMillis;
}

/// Production clock backed by [`std::time::Instant`] (monotonic, immune to
/// wall-clock steps; deliberately *not* `SystemTime`).
#[derive(Debug)]
pub struct MonotonicClock {
    origin: Instant,
}

impl MonotonicClock {
    /// Creates a clock whose readings count from now.
    #[must_use]
    pub fn new() -> Self {
        // Sanctioned ambient-time read (see clippy.toml disallowed-methods):
        // this constructor is the crate-local stand-in for the shared
        // `clock::SystemClock` wrapper until crates/clock (ticket ca-clock)
        // lands; everything else in this crate injects `Clock`.
        #[allow(clippy::disallowed_methods)]
        let origin = Instant::now();
        Self { origin }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn monotonic_now(&self) -> MonotonicMillis {
        // u64 millis overflow ~584 million years after process start.
        MonotonicMillis(u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
}

/// Manually advanced clock for deterministic tests.
#[derive(Debug, Default)]
pub struct ManualClock {
    now: AtomicU64,
}

impl ManualClock {
    /// Creates a clock reading zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances the clock by `millis`.
    pub fn advance(&self, millis: u64) {
        self.now.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn monotonic_now(&self) -> MonotonicMillis {
        MonotonicMillis(self.now.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_advances() {
        let clock = ManualClock::new();
        assert_eq!(clock.monotonic_now(), MonotonicMillis(0));
        clock.advance(250);
        assert_eq!(clock.monotonic_now(), MonotonicMillis(250));
    }

    #[test]
    fn monotonic_clock_never_goes_backwards() {
        let clock = MonotonicClock::new();
        let a = clock.monotonic_now();
        let b = clock.monotonic_now();
        assert!(b >= a);
    }

    #[test]
    fn saturating_add_saturates() {
        assert_eq!(
            MonotonicMillis(u64::MAX).saturating_add(1),
            MonotonicMillis(u64::MAX)
        );
    }
}
