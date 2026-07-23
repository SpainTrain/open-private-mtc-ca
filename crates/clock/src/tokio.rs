//! Fake-clock-driven async sleep/interval helpers for cadence-based loops
//! (`tokio` feature; spec §18.4 time travel).
//!
//! [`AsyncClock`] extends [`Clock`] with `sleep`/`sleep_until`, and
//! [`Interval`] builds a cadence ticker on top. Under [`SystemClock`] these
//! delegate to `tokio::time`; under [`FakeClock`] a sleeper completes the
//! moment `advance`/`set` moves the clock past its deadline — no wall-clock
//! waiting.
//!
//! # Demo: a batch-cadence loop firing instantly under [`FakeClock`]
//!
//! Three 5-minute batch cadences complete as soon as the fake clock is
//! advanced 15 minutes, instead of taking 15 minutes of wall-clock time:
//!
//! ```
//! use std::sync::Arc;
//! use std::time::{Duration, UNIX_EPOCH};
//! use clock::tokio::{AsyncClock, Interval};
//! use clock::FakeClock;
//!
//! let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
//! rt.block_on(async {
//!     let clock = Arc::new(FakeClock::new(UNIX_EPOCH));
//!     let cadence = Duration::from_secs(300);
//!
//!     // The batch loop sees only the injected trait object.
//!     let batch_loop = tokio::spawn({
//!         let mut ticks = Interval::new(
//!             Arc::clone(&clock) as Arc<dyn AsyncClock>,
//!             cadence,
//!         );
//!         async move {
//!             let mut batches_built = Vec::new();
//!             for _ in 0..3 {
//!                 batches_built.push(ticks.tick().await); // build a batch here
//!             }
//!             batches_built
//!         }
//!     });
//!
//!     // Time-travel 15 minutes; all three pending cadences fire instantly.
//!     clock.advance(Duration::from_secs(900));
//!
//!     let batches = batch_loop.await.unwrap();
//!     assert_eq!(
//!         batches,
//!         vec![
//!             UNIX_EPOCH + Duration::from_secs(300),
//!             UNIX_EPOCH + Duration::from_secs(600),
//!             UNIX_EPOCH + Duration::from_secs(900),
//!         ],
//!     );
//! });
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::{Clock, FakeClock, SystemClock};

/// Boxed future returned by [`AsyncClock`] sleep methods (boxed to keep the
/// trait object-safe, so it can ride along the injected `Arc<dyn Clock>`
/// seam as `Arc<dyn AsyncClock>`; §22.7).
pub type SleepFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// A [`Clock`] that can also wait for a point in clock time.
///
/// Inject it exactly like [`Clock`] (`Arc<dyn AsyncClock>`): production
/// wiring uses [`SystemClock`] (real `tokio::time` sleeps); tests and
/// dev-mode time travel use [`FakeClock`], whose sleepers wake as soon as
/// `advance`/`set` reaches their deadline.
pub trait AsyncClock: Clock {
    /// Completes once `self.now() >= deadline`.
    ///
    /// Completes immediately if the deadline has already passed.
    fn sleep_until(&self, deadline: SystemTime) -> SleepFuture<'_>;

    /// Completes after `duration` of *clock* time (not necessarily wall
    /// time) has passed.
    ///
    /// If `now() + duration` overflows [`SystemTime`], the future never
    /// completes.
    fn sleep(&self, duration: Duration) -> SleepFuture<'_> {
        match self.now().checked_add(duration) {
            Some(deadline) => self.sleep_until(deadline),
            None => Box::pin(std::future::pending()),
        }
    }
}

impl AsyncClock for SystemClock {
    fn sleep_until(&self, deadline: SystemTime) -> SleepFuture<'_> {
        Box::pin(async move {
            if let Ok(remaining) = deadline.duration_since(self.now()) {
                ::tokio::time::sleep(remaining).await;
            }
        })
    }
}

impl AsyncClock for FakeClock {
    fn sleep_until(&self, deadline: SystemTime) -> SleepFuture<'_> {
        Box::pin(async move {
            // Subscribing *before* the first `now()` check closes the race:
            // an advance landing between a check and the following await has
            // bumped the watch version, so `changed()` resolves immediately —
            // no lost wakeups.
            let mut ticks = self.subscribe();
            while self.now() < deadline {
                if ticks.changed().await.is_err() {
                    // Sender dropped — impossible while `self` is alive, but
                    // break rather than spin if it ever happens.
                    break;
                }
            }
        })
    }
}

/// `Arc<C>` (including `Arc<dyn AsyncClock>`) is itself an [`AsyncClock`].
impl<C: AsyncClock + ?Sized> AsyncClock for Arc<C> {
    fn sleep_until(&self, deadline: SystemTime) -> SleepFuture<'_> {
        (**self).sleep_until(deadline)
    }
}

/// A cadence ticker driven by an [`AsyncClock`] — the clock-injected
/// equivalent of `tokio::time::interval` for cadence-based loops.
///
/// [`Interval::tick`] completes once per elapsed `period` of clock time and
/// returns the scheduled tick time. If the clock jumps several periods at
/// once (dev-mode time travel, §18.4), the missed ticks fire back-to-back
/// until the schedule catches up ("burst" behavior). A zero `period` ticks
/// on every call.
///
/// Generic over the clock (§22.7 — lean toward generics); the blanket `Arc`
/// impls mean `Interval<Arc<dyn AsyncClock>>` works at the injection seam.
pub struct Interval<C: AsyncClock> {
    clock: C,
    period: Duration,
    /// Next scheduled tick; `None` once the schedule has overflowed
    /// [`SystemTime`] (ticks never fire again).
    next: Option<SystemTime>,
}

impl<C: AsyncClock> Interval<C> {
    /// Creates a ticker whose first tick is due at `clock.now() + period`.
    #[must_use]
    pub fn new(clock: C, period: Duration) -> Self {
        let next = clock.now().checked_add(period);
        Self {
            clock,
            period,
            next,
        }
    }

    /// Creates a ticker whose first tick is due at `first_tick`.
    #[must_use]
    pub fn new_at(clock: C, first_tick: SystemTime, period: Duration) -> Self {
        Self {
            clock,
            period,
            next: Some(first_tick),
        }
    }

    /// Waits for the next scheduled tick and returns its scheduled time.
    pub async fn tick(&mut self) -> SystemTime {
        match self.next {
            Some(deadline) => {
                self.clock.sleep_until(deadline).await;
                self.next = deadline.checked_add(self.period);
                deadline
            }
            // Schedule overflowed SystemTime: never ticks again.
            None => std::future::pending().await,
        }
    }
}
