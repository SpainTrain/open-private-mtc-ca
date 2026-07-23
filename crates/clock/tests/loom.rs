//! Loom concurrency models for `FakeClock` internals: exhaustively explores
//! interleavings of concurrent `advance`/`set`/`now` (ticket testing
//! requirement; spec §19 test strategy).
//!
//! Run with:
//!
//! ```console
//! RUSTFLAGS="--cfg loom" cargo test -p clock --test loom --release
//! ```

#![cfg(loom)]

use std::time::{Duration, UNIX_EPOCH};

use clock::{Clock, FakeClock};
use loom::sync::Arc;
use loom::thread;

#[test]
fn concurrent_advances_always_sum() {
    loom::model(|| {
        let clock = Arc::new(FakeClock::new(UNIX_EPOCH));

        let handles: Vec<_> = [10_u64, 20]
            .iter()
            .map(|&nanos| {
                let clock = Arc::clone(&clock);
                thread::spawn(move || {
                    clock.advance(Duration::from_nanos(nanos));
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        // Every interleaving of the two advances lands both increments.
        assert_eq!(clock.now(), UNIX_EPOCH + Duration::from_nanos(30));
    });
}

#[test]
fn now_never_goes_backward_under_concurrent_writes() {
    loom::model(|| {
        let clock = Arc::new(FakeClock::new(UNIX_EPOCH));

        let writer = {
            let clock = Arc::clone(&clock);
            thread::spawn(move || {
                clock.advance(Duration::from_nanos(5));
                clock
                    .set(UNIX_EPOCH + Duration::from_nanos(20))
                    .expect("forward set");
            })
        };

        // Reads interleaved with the writes must observe monotonic time.
        let first = clock.now();
        let second = clock.now();
        assert!(
            second >= first,
            "now() went backward: {first:?} -> {second:?}"
        );

        writer.join().unwrap();
        let last = clock.now();
        assert!(last >= second);
        assert_eq!(last, UNIX_EPOCH + Duration::from_nanos(20));
    });
}

#[test]
fn concurrent_set_and_advance_compose_monotonically() {
    loom::model(|| {
        let clock = Arc::new(FakeClock::new(UNIX_EPOCH));

        let setter = {
            let clock = Arc::clone(&clock);
            thread::spawn(move || {
                // Both orders keep the target ahead of the clock, so this
                // must succeed in every interleaving.
                clock
                    .set(UNIX_EPOCH + Duration::from_nanos(100))
                    .expect("forward set");
            })
        };
        let advancer = {
            let clock = Arc::clone(&clock);
            thread::spawn(move || {
                clock.advance(Duration::from_nanos(50));
            })
        };
        setter.join().unwrap();
        advancer.join().unwrap();

        // set-then-advance => 150; advance-then-set => 100. Nothing else.
        let now = clock.now();
        assert!(
            now == UNIX_EPOCH + Duration::from_nanos(150)
                || now == UNIX_EPOCH + Duration::from_nanos(100),
            "unexpected final time: {now:?}",
        );
    });
}
