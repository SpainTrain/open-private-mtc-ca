//! `cargo run -p mtc-ca-service --example batch_demo`
//!
//! Feeds 1000 entries through the intake queue + batch builder (ticket
//! `mtc-2kx`) in uneven bursts and prints each emitted batch's boundaries and
//! which trigger closed it — the size trigger (spec §11.1 step 2's "full
//! (256)", here a smaller round number so the trace fits on screen) or the
//! cadence trigger (the "2-5s" wall-clock cadence, simulated here via
//! `FakeClock` so the demo runs instantly instead of taking real minutes).

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use clock::tokio::AsyncClock;
use clock::FakeClock;
use eyre::Result;
use mtc::Index;
use mtc_ca_service::batch::{self, BatchConfig};
use mtc_ca_service::{EntryIntake, IntakeError, LogEntry, SourceId, SourceType};

const TOTAL: u64 = 1000;
/// Uneven on purpose, and deliberately not a multiple of the 128-entry batch
/// size below: some bursts close a batch purely on the size trigger, and
/// 1000 % 128 == 104 guarantees the final batch has no more entries coming
/// and so *must* close on the cadence trigger instead. Sums to `TOTAL`.
const BURSTS: [u64; 5] = [130, 40, 310, 90, 430];

// current_thread: deterministic, single-threaded cooperative scheduling. The
// clock ticker below advances the shared `FakeClock` from a task that is
// always immediately ready again after `yield_now`; on a genuinely parallel
// (multi-thread) runtime it can race far ahead of the submitter's spawned
// tasks in real wall-clock terms and starve them, closing every batch at
// size 1. Single-threaded round-robin scheduling keeps ticks and submissions
// interleaved fairly.
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;
    assert_eq!(
        BURSTS.iter().sum::<u64>(),
        TOTAL,
        "bursts must sum to TOTAL"
    );

    let config = BatchConfig::new(128, Duration::from_millis(800))?;
    let clock = Arc::new(FakeClock::default());
    let (intake, mut builder) = batch::channel(config, Arc::clone(&clock) as Arc<dyn AsyncClock>);
    let intake = Arc::new(intake);

    println!(
        "batch config: max_batch_size={} cadence={:?}",
        config.max_batch_size(),
        config.cadence()
    );
    println!(
        "submitting {TOTAL} entries in {} bursts: {BURSTS:?}\n",
        BURSTS.len()
    );

    // Submitter: fires each burst, then yields so the drain loop below can
    // make progress before the next burst lands. Deliberately never calls
    // `shutdown()`: with many concurrently-spawned submitters in flight, a
    // shutdown can race ahead of the last few that have not yet reached
    // their enqueue step, correctly rejecting them (see `IntakeQueue::
    // shutdown`'s docs) -- fine for a real caller that tracks its own
    // in-flight count, but this demo instead knows `TOTAL` up front and
    // simply drains until every entry has come back out (mirrors
    // `tests/batch_property.rs`'s `submit_and_drain_all`).
    //
    // Several bursts below (310, 430) are far larger than max_batch_size
    // (128): submitted this concurrently, most of a large burst loses the
    // initial race for a channel slot. Each submitter retries on
    // `IntakeError::QueueFull` -- the backoff its own docs ask callers for
    // -- rather than giving up on the first try, or entries would be
    // silently dropped instead of merely delayed.
    tokio::spawn({
        let intake = Arc::clone(&intake);
        async move {
            let mut sent = 0u64;
            for burst in BURSTS {
                for seq in sent..sent + burst {
                    let intake = Arc::clone(&intake);
                    tokio::spawn(async move {
                        let entry = LogEntry::new(
                            format!("tbs-cert-{seq}").into_bytes(),
                            SourceType::NativeAcme,
                            SourceId::from(format!("order-{seq}")),
                            UNIX_EPOCH,
                        );
                        while intake.submit_entry(entry.clone()).await
                            == Err(IntakeError::QueueFull)
                        {
                            tokio::task::yield_now().await;
                        }
                    });
                }
                sent += burst;
                tokio::task::yield_now().await;
            }
        }
    });

    // Clock ticker: advances the (simulated) clock in small steps so bursts
    // shorter than max_batch_size close on the cadence trigger instead of
    // waiting -- see the `batch` module docs on why the cadence clock is
    // injected rather than read from the wall clock directly.
    tokio::spawn({
        let clock = Arc::clone(&clock);
        async move {
            loop {
                tokio::task::yield_now().await;
                clock.advance(Duration::from_millis(100));
            }
        }
    });

    // The drain loop runs as its own spawned task rather than inline here:
    // `next_batch` needs to suspend and later be woken by the submitter and
    // ticker tasks above, and a `current_thread` runtime's `block_on` root
    // future (this `main`) is not a reliable place to drive that kind of
    // cross-task wakeup -- see the `batch` module docs' "Driving
    // `next_batch`" section. Every `batch::builder::tests` case that waits
    // on the cadence trigger spawns `next_batch` for the same reason.
    let drain = tokio::spawn(async move {
        let mut next_index = 0u64;
        let mut batch_no = 0u32;
        while next_index < TOTAL {
            let Some(batch) = builder.next_batch().await else {
                break;
            };
            batch_no += 1;
            let len = batch.len();
            let trigger = if len == config.max_batch_size() {
                "size"
            } else {
                "cadence"
            };
            println!(
                "[batch {batch_no:>3}] {len:>3} entries  (trigger: {trigger:<7})  indices [{next_index}, {})",
                next_index + len as u64
            );
            batch.complete_sequential(Index(next_index));
            next_index += len as u64;
        }
        (batch_no, next_index)
    });
    let (batch_no, next_index) = drain.await?;

    println!("\ndone: {batch_no} batches, {next_index} entries total (expected {TOTAL}).");
    assert_eq!(next_index, TOTAL);
    Ok(())
}
