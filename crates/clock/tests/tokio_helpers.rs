//! Integration tests for the `tokio`-feature async helpers: fake-clock-driven
//! sleeps and cadence intervals (spec §18.4).

#![cfg(all(feature = "tokio", not(loom)))]

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clock::tokio::{AsyncClock, Interval};
use clock::{Clock, FakeClock, SystemClock};

fn start() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

/// Wall-clock guard: the whole wait must complete in far less time than the
/// simulated duration, proving no real sleeping happened.
// Test code may read ambient time directly (rule no-systemtime-now-in-prod
// exempts tests); this measures real elapsed time.
#[allow(clippy::disallowed_methods)]
fn wall_timer() -> std::time::Instant {
    std::time::Instant::now()
}

#[tokio::test]
async fn sleep_completes_when_fake_clock_advances() {
    let clock = FakeClock::new(start());

    // `sleep` pins its deadline when called — before the advance below.
    let mut sleeper = clock.sleep(Duration::from_hours(1));

    // Poll it once so its waker is registered while it is still pending.
    let still_pending = tokio::time::timeout(Duration::from_millis(50), &mut sleeper)
        .await
        .is_err();
    assert!(still_pending, "sleeper woke before any advance");

    clock.advance(Duration::from_hours(1));

    tokio::time::timeout(Duration::from_secs(5), sleeper)
        .await
        .expect("sleeper should wake as soon as the clock reaches its deadline");
}

#[tokio::test]
async fn sleep_does_not_complete_before_the_deadline() {
    let clock = FakeClock::new(start());
    let mut sleeper = clock.sleep(Duration::from_mins(1));

    // One second short of the deadline: the sleeper must still be pending.
    clock.advance(Duration::from_secs(59));
    let still_pending = tokio::time::timeout(Duration::from_millis(50), &mut sleeper)
        .await
        .is_err();
    assert!(still_pending, "sleeper woke before its deadline");

    // The final second releases it.
    clock.advance(Duration::from_secs(1));
    tokio::time::timeout(Duration::from_secs(5), sleeper)
        .await
        .expect("sleeper should wake at the deadline");
}

#[tokio::test]
async fn sleep_until_past_deadline_completes_immediately() {
    let clock = FakeClock::new(start());
    // Deadline already reached: must complete without any advance() at all.
    clock.sleep_until(start()).await;
    clock.sleep_until(start() - Duration::from_secs(10)).await;
}

#[tokio::test]
async fn set_also_wakes_sleepers() {
    let clock = Arc::new(FakeClock::new(start()));

    let sleeper = tokio::spawn({
        let clock = Arc::clone(&clock);
        async move { clock.sleep_until(start() + Duration::from_mins(5)).await }
    });

    clock
        .set(start() + Duration::from_secs(400))
        .expect("forward set must succeed");

    tokio::time::timeout(Duration::from_secs(5), sleeper)
        .await
        .expect("sleeper should wake when set() jumps past its deadline")
        .expect("sleeper task panicked");
}

#[tokio::test]
async fn multiple_sleepers_wake_on_one_advance() {
    let clock = FakeClock::new(start());

    // Three sleeps with staggered deadlines, all pinned before the advance
    // and all polled once so their wakers are registered while pending.
    let mut sleepers: Vec<_> = (1..=3)
        .map(|minutes| clock.sleep(Duration::from_secs(60 * minutes)))
        .collect();
    for sleeper in &mut sleepers {
        let still_pending = tokio::time::timeout(Duration::from_millis(10), sleeper)
            .await
            .is_err();
        assert!(still_pending, "sleeper woke before any advance");
    }

    clock.advance(Duration::from_mins(3));

    for sleeper in sleepers {
        tokio::time::timeout(Duration::from_secs(5), sleeper)
            .await
            .expect("every sleeper should wake on the single big advance");
    }
}

#[tokio::test]
async fn cadence_loop_fires_instantly_under_fake_clock() {
    let wall = wall_timer();
    let clock = Arc::new(FakeClock::new(start()));
    let cadence = Duration::from_mins(5);

    // The batch loop only sees the injected trait object (§22.7).
    let batch_loop = tokio::spawn({
        let mut ticks = Interval::new(Arc::clone(&clock) as Arc<dyn AsyncClock>, cadence);
        async move {
            let mut fired_at = Vec::new();
            for _ in 0..3 {
                fired_at.push(ticks.tick().await);
            }
            fired_at
        }
    });

    // 15 simulated minutes; the three pending cadence ticks burst through.
    clock.advance(Duration::from_mins(15));

    let fired_at = tokio::time::timeout(Duration::from_secs(5), batch_loop)
        .await
        .expect("cadence loop should finish instantly under FakeClock")
        .expect("cadence task panicked");

    assert_eq!(
        fired_at,
        vec![
            start() + Duration::from_mins(5),
            start() + Duration::from_mins(10),
            start() + Duration::from_mins(15),
        ],
        "ticks must fire at their scheduled times",
    );
    assert!(
        wall.elapsed() < Duration::from_secs(5),
        "15 simulated minutes must not take wall-clock time",
    );
}

#[tokio::test]
async fn interval_new_at_controls_the_first_tick() {
    let clock = Arc::new(FakeClock::new(start()));
    let first = start() + Duration::from_secs(10);
    let mut ticks = Interval::new_at(
        Arc::clone(&clock) as Arc<dyn AsyncClock>,
        first,
        Duration::from_secs(30),
    );

    clock.advance(Duration::from_secs(40));
    assert_eq!(ticks.tick().await, first);
    assert_eq!(ticks.tick().await, first + Duration::from_secs(30));
}

#[tokio::test]
async fn system_clock_sleep_delegates_to_tokio_time() {
    // Zero/past deadlines complete promptly on the real clock too.
    SystemClock.sleep(Duration::ZERO).await;
    let earlier = SystemClock.now() - Duration::from_secs(10);
    SystemClock.sleep_until(earlier).await;
}
