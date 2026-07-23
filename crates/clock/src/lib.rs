//! Injected time source for the MTC CA workspace (spec §22.11, §18.4).
//!
//! Production code never calls [`SystemTime::now`] (or `Instant::now`)
//! directly — rule `no-systemtime-now-in-prod` (§23.2), enforced by the
//! `disallowed-methods` list in the workspace `clippy.toml` (§22.12). Instead,
//! time is read exclusively through the [`Clock`] trait, injected as an
//! `Arc<dyn Clock>` at the architectural seam (§22.7: `Clock` is an injected
//! test dependency — initialized once, called from many call sites, so
//! dynamic dispatch is the deliberate choice). Two implementations ship here:
//!
//! - [`SystemClock`] — the one sanctioned wrapper around [`SystemTime::now`];
//!   production binaries wire this in at startup.
//! - [`FakeClock`] — a thread-safe fake with controllable, monotonic
//!   advancement: the foundation for deterministic time-dependent tests and
//!   dev-mode time travel (`make time-advance`, §18.4).
//!
//! # The injection pattern (worked example)
//!
//! A time-dependent component stores an `Arc<dyn Clock>` and reads time only
//! through it. Tests (and dev mode) inject a [`FakeClock`] and drive time
//! explicitly; production wiring swaps in [`SystemClock`] — the component
//! itself never changes:
//!
//! ```
//! use std::sync::Arc;
//! use std::time::{Duration, SystemTime, UNIX_EPOCH};
//! use clock::{Clock, FakeClock, SystemClock};
//!
//! /// Decides when the next batch is due (a batch-cadence guard).
//! struct BatchCadence {
//!     clock: Arc<dyn Clock>,
//!     next_batch_at: SystemTime,
//! }
//!
//! impl BatchCadence {
//!     fn is_due(&self) -> bool {
//!         self.clock.now() >= self.next_batch_at
//!     }
//! }
//!
//! // Test / dev-mode wiring: inject a FakeClock and advance it — no
//! // wall-clock waiting, fully deterministic.
//! let fake = Arc::new(FakeClock::new(UNIX_EPOCH));
//! let cadence = BatchCadence {
//!     clock: fake.clone(),
//!     next_batch_at: UNIX_EPOCH + Duration::from_secs(60),
//! };
//! assert!(!cadence.is_due());
//! fake.advance(Duration::from_secs(60));
//! assert!(cadence.is_due());
//!
//! // Production wiring: the only change is the injected clock.
//! let _prod = BatchCadence {
//!     clock: Arc::new(SystemClock),
//!     next_batch_at: SystemTime::UNIX_EPOCH + Duration::from_secs(60),
//! };
//! ```
//!
//! # Async cadence loops (`tokio` feature)
//!
//! With the `tokio` feature enabled, the [`tokio::AsyncClock`] extension adds
//! fake-clock-driven `sleep`/`interval` helpers so cadence-based loops fire
//! instantly under [`FakeClock`] instead of waiting wall-clock time — see the
//! [`tokio`] module docs for the batch-cadence demo.

mod fake;
#[cfg(all(feature = "tokio", not(loom)))]
pub mod tokio;

use std::sync::Arc;
use std::time::SystemTime;

pub use fake::{ClockWentBackward, FakeClock};

/// The injected time source (spec §22.11).
///
/// Production code takes this as an `Arc<dyn Clock>` (§22.7) and never calls
/// [`SystemTime::now`] directly (rule `no-systemtime-now-in-prod`). Inject
/// [`SystemClock`] in production and [`FakeClock`] in tests and dev mode.
pub trait Clock: Send + Sync {
    /// Returns the current time according to this clock.
    fn now(&self) -> SystemTime;
}

/// The production clock: the one sanctioned wrapper around
/// [`SystemTime::now`].
///
/// Rule `no-systemtime-now-in-prod` (§23.2, spec §22.11) forbids direct
/// `SystemTime::now()` / `Instant::now()` calls everywhere outside
/// `crates/clock` and test code — enforced by `disallowed-methods` in the
/// workspace `clippy.toml` (§22.12). All production time reads flow through
/// an injected `Arc<dyn Clock>` that binaries point at this type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        // The single sanctioned `SystemTime::now()` call site in the
        // workspace (rule `no-systemtime-now-in-prod`; the deny lives in the
        // root clippy.toml `disallowed-methods` list).
        #[allow(clippy::disallowed_methods)]
        SystemTime::now()
    }
}

/// `Arc<C>` (including `Arc<dyn Clock>`) is itself a [`Clock`], so generic
/// components (`<C: Clock>`) accept the injected trait object directly.
impl<C: Clock + ?Sized> Clock for Arc<C> {
    fn now(&self) -> SystemTime {
        (**self).now()
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use std::time::Duration;

    fn read_now<C: Clock>(clock: &C) -> SystemTime {
        clock.now()
    }

    #[test]
    // Test code may read ambient time directly (rule no-systemtime-now-in-prod
    // exempts tests); here it is the reference value SystemClock is checked
    // against.
    #[allow(clippy::disallowed_methods)]
    fn system_clock_tracks_system_time() {
        let before = SystemTime::now();
        let observed = SystemClock.now();
        let after = SystemTime::now();

        // SystemTime is not strictly monotonic (NTP steps), so allow slack
        // rather than asserting exact ordering.
        let slack = Duration::from_mins(1);
        assert!(observed + slack >= before, "SystemClock lags real time");
        assert!(observed <= after + slack, "SystemClock ahead of real time");
    }

    #[test]
    fn clock_is_object_safe_and_arc_injectable() {
        let system: Arc<dyn Clock> = Arc::new(SystemClock);
        let fake: Arc<dyn Clock> = Arc::new(FakeClock::default());

        // Callable through the trait object...
        let _ = system.now();
        assert_eq!(fake.now(), SystemTime::UNIX_EPOCH);
        // ...and through generic code, via the blanket Arc impl.
        assert_eq!(read_now(&fake), SystemTime::UNIX_EPOCH);
    }

    #[test]
    fn arc_dyn_clock_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        let clock: Arc<dyn Clock> = Arc::new(FakeClock::default());
        assert_send_sync(&clock);
    }
}
