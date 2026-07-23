//! Thread-safe fake clock with controllable, monotonic advancement
//! (spec §22.11, §18.4 time travel).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(loom)]
use loom::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicU64, Ordering};

use crate::Clock;

/// Error returned by [`FakeClock::set`] when the requested time is earlier
/// than the clock's current time.
///
/// Monotonicity is enforced: a fake clock never goes backward (§18.4 —
/// time travel only fast-forwards), so a backward `set` is rejected and the
/// clock is left unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("FakeClock cannot go backward: current time {current:?}, requested {requested:?}")]
pub struct ClockWentBackward {
    /// The clock's time at the moment of the rejected call.
    pub current: SystemTime,
    /// The (earlier) time that was requested.
    pub requested: SystemTime,
}

/// A thread-safe fake [`Clock`] with controllable advancement
/// (spec §22.11, §18.4).
///
/// Share it as `Arc<FakeClock>`: keep one strong reference for the test (or
/// the dev-mode admin surface) to drive time, and hand clones — coerced to
/// `Arc<dyn Clock>` — to the components under test. All methods take `&self`
/// and are safe to call concurrently from any thread; updates are lock-free
/// (a single atomic word) and monotonic: `now()` never observes time moving
/// backward.
///
/// Time is represented as whole nanoseconds since [`UNIX_EPOCH`] in a `u64`:
/// representable times span [`UNIX_EPOCH`] through roughly the year 2554.
/// Conversions saturate at those bounds (documented on each method).
///
/// ```
/// use std::sync::Arc;
/// use std::thread;
/// use std::time::{Duration, UNIX_EPOCH};
/// use clock::{Clock, FakeClock};
///
/// let clock = Arc::new(FakeClock::new(UNIX_EPOCH));
/// let worker = {
///     let clock = Arc::clone(&clock);
///     thread::spawn(move || clock.advance(Duration::from_secs(300)))
/// };
/// worker.join().expect("worker panicked");
/// assert_eq!(clock.now(), UNIX_EPOCH + Duration::from_secs(300));
/// ```
#[derive(Debug)]
pub struct FakeClock {
    /// Nanoseconds since [`UNIX_EPOCH`].
    nanos: AtomicU64,
    /// Wakes async sleepers (see `crate::tokio`) whenever time moves.
    #[cfg(all(feature = "tokio", not(loom)))]
    tick: ::tokio::sync::watch::Sender<()>,
}

impl FakeClock {
    /// Creates a fake clock reading `start`.
    ///
    /// `start` saturates to the representable range: times before
    /// [`UNIX_EPOCH`] clamp to [`UNIX_EPOCH`]; times beyond `u64::MAX`
    /// nanoseconds after it clamp to that maximum.
    #[must_use]
    pub fn new(start: SystemTime) -> Self {
        Self {
            nanos: AtomicU64::new(time_to_nanos(start)),
            #[cfg(all(feature = "tokio", not(loom)))]
            tick: ::tokio::sync::watch::channel(()).0,
        }
    }

    /// Advances the clock by `delta` and returns the new time.
    ///
    /// Concurrent advances compose: every nanosecond of every `advance` is
    /// applied exactly once, so the final time is independent of interleaving
    /// order. Saturates at the maximum representable time (`delta` values
    /// beyond `u64::MAX` nanoseconds — about 584 years — are clamped).
    pub fn advance(&self, delta: Duration) -> SystemTime {
        let delta = u64::try_from(delta.as_nanos()).unwrap_or(u64::MAX);
        let new = self.update(|current| current.saturating_add(delta));
        self.notify_waiters();
        nanos_to_time(new)
    }

    /// Sets the clock to `to`, which must not be earlier than the current
    /// time.
    ///
    /// Monotonicity is enforced: if `to` is earlier than [`Clock::now`], the
    /// clock is left unchanged and [`ClockWentBackward`] is returned (setting
    /// the clock to exactly its current time is a no-op and succeeds). `to`
    /// saturates to the representable range as in [`FakeClock::new`].
    ///
    /// # Errors
    ///
    /// Returns [`ClockWentBackward`] if `to` is earlier than the current
    /// time (after saturation — e.g. any time before [`UNIX_EPOCH`] is
    /// earlier than every reachable clock value except [`UNIX_EPOCH`]
    /// itself).
    pub fn set(&self, to: SystemTime) -> Result<(), ClockWentBackward> {
        let target = time_to_nanos(to);
        let mut current = self.nanos.load(Ordering::SeqCst);
        loop {
            if target < current {
                return Err(ClockWentBackward {
                    current: nanos_to_time(current),
                    requested: to,
                });
            }
            match self
                .nanos
                .compare_exchange(current, target, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    self.notify_waiters();
                    return Ok(());
                }
                Err(actual) => current = actual,
            }
        }
    }

    /// Applies `f` atomically via a compare-exchange loop; returns the value
    /// stored. `f` must be monotonic (never map a value to a smaller one) to
    /// preserve the clock's monotonicity guarantee.
    fn update(&self, f: impl Fn(u64) -> u64) -> u64 {
        let mut current = self.nanos.load(Ordering::SeqCst);
        loop {
            let next = f(current);
            match self
                .nanos
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return next,
                Err(actual) => current = actual,
            }
        }
    }

    /// Subscribes to time-change notifications (for async sleep helpers).
    #[cfg(all(feature = "tokio", not(loom)))]
    pub(crate) fn subscribe(&self) -> ::tokio::sync::watch::Receiver<()> {
        self.tick.subscribe()
    }

    /// Wakes any async sleepers so they re-check their deadlines.
    #[cfg(all(feature = "tokio", not(loom)))]
    fn notify_waiters(&self) {
        self.tick.send_replace(());
    }

    /// No async sleepers without the `tokio` feature (or under loom).
    #[cfg(not(all(feature = "tokio", not(loom))))]
    fn notify_waiters(&self) {}
}

/// Starts at [`UNIX_EPOCH`] — a fixed, deterministic origin.
impl Default for FakeClock {
    fn default() -> Self {
        Self::new(UNIX_EPOCH)
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        nanos_to_time(self.nanos.load(Ordering::SeqCst))
    }
}

/// Saturating conversion: times before [`UNIX_EPOCH`] clamp to 0; times
/// beyond `u64::MAX` nanoseconds after it (~year 2554) clamp to `u64::MAX`.
fn time_to_nanos(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

fn nanos_to_time(nanos: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(nanos)
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::Arc;
    use std::thread;

    fn start() -> SystemTime {
        // 2023-11-14T22:13:20Z — an arbitrary fixed, deterministic origin.
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    #[test]
    fn advance_moves_now_forward_and_returns_new_time() {
        let clock = FakeClock::new(start());
        assert_eq!(clock.now(), start());

        let returned = clock.advance(Duration::from_millis(1500));
        let expected = start() + Duration::from_millis(1500);
        assert_eq!(returned, expected);
        assert_eq!(clock.now(), expected);
    }

    #[test]
    fn advance_by_zero_is_a_no_op() {
        let clock = FakeClock::new(start());
        assert_eq!(clock.advance(Duration::ZERO), start());
        assert_eq!(clock.now(), start());
    }

    #[test]
    fn set_forward_moves_the_clock() {
        let clock = FakeClock::new(start());
        let target = start() + Duration::from_secs(86_400 * 400); // 400-day time travel (§18.4)
        clock.set(target).expect("forward set must succeed");
        assert_eq!(clock.now(), target);
    }

    #[test]
    fn set_to_current_time_is_ok() {
        let clock = FakeClock::new(start());
        clock.set(start()).expect("no-op set must succeed");
        assert_eq!(clock.now(), start());
    }

    #[test]
    fn set_backward_errors_and_leaves_clock_unchanged() {
        let clock = FakeClock::new(start());
        let requested = start() - Duration::from_secs(1);

        let err = clock.set(requested).expect_err("backward set must fail");
        assert_eq!(err.current, start());
        assert_eq!(err.requested, requested);
        assert_eq!(clock.now(), start(), "clock must be unchanged after error");
    }

    #[test]
    fn times_before_epoch_clamp_to_epoch() {
        if let Some(pre_epoch) = UNIX_EPOCH.checked_sub(Duration::from_secs(5)) {
            let clock = FakeClock::new(pre_epoch);
            assert_eq!(clock.now(), UNIX_EPOCH);
        }
    }

    #[test]
    fn default_starts_at_unix_epoch() {
        assert_eq!(FakeClock::default().now(), UNIX_EPOCH);
    }

    #[test]
    fn advance_saturates_instead_of_wrapping() {
        let clock = FakeClock::new(start());
        let end_of_range = clock.advance(Duration::from_secs(u64::MAX));
        // A further advance stays pinned at the maximum — no wraparound.
        assert_eq!(clock.advance(Duration::from_secs(1)), end_of_range);
    }

    #[test]
    fn advances_are_visible_across_threads() {
        let clock = Arc::new(FakeClock::new(start()));

        let worker = {
            let clock = Arc::clone(&clock);
            thread::spawn(move || {
                clock.advance(Duration::from_secs(60));
            })
        };
        worker.join().expect("worker thread panicked");

        assert_eq!(clock.now(), start() + Duration::from_secs(60));
    }

    #[test]
    fn concurrent_advances_from_many_threads_all_land() {
        const THREADS: u64 = 8;
        const ADVANCES_PER_THREAD: u64 = 1000;

        let clock = Arc::new(FakeClock::new(start()));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let clock = Arc::clone(&clock);
                thread::spawn(move || {
                    for _ in 0..ADVANCES_PER_THREAD {
                        clock.advance(Duration::from_millis(1));
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("advancing thread panicked");
        }

        let expected = start() + Duration::from_millis(THREADS * ADVANCES_PER_THREAD);
        assert_eq!(clock.now(), expected);
    }

    proptest! {
        /// A sequence of advances yields the same final `now()` regardless of
        /// the interleaving order in which threads apply them: the scheduler
        /// picks an arbitrary interleaving on every run, and the shuffled
        /// round-robin partition varies the per-thread order, yet the final
        /// time is always `start + sum(durations)`.
        #[test]
        fn concurrent_advance_order_does_not_matter(
            nanos in prop::collection::vec(0u64..10_000_000_000, 1..16),
            shuffle in any::<prop::sample::Index>(),
        ) {
            const THREADS: usize = 3;

            let total: u64 = nanos.iter().sum();
            let mut shuffled = nanos.clone();
            shuffled.rotate_left(shuffle.index(nanos.len()));

            let clock = Arc::new(FakeClock::new(start()));
            let handles: Vec<_> = (0..THREADS)
                .map(|t| {
                    let share: Vec<u64> = shuffled
                        .iter()
                        .copied()
                        .skip(t)
                        .step_by(THREADS)
                        .collect();
                    let clock = Arc::clone(&clock);
                    thread::spawn(move || {
                        for n in share {
                            clock.advance(Duration::from_nanos(n));
                        }
                    })
                })
                .collect();
            for handle in handles {
                handle.join().expect("advancing thread panicked");
            }

            prop_assert_eq!(clock.now(), start() + Duration::from_nanos(total));
        }
    }
}
