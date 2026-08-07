//! Property test (ticket `mtc-2kx`, spec §19.2): for an arbitrary sequence of
//! entries submitted *concurrently* (many independent tasks racing for
//! channel capacity, retrying on backpressure), every entry lands in exactly
//! one emitted batch and every submitter is assigned a unique index — no
//! entry is lost or duplicated, and no index is assigned twice, regardless
//! of how the sequence happens to fall across batch boundaries.
//!
//! This deliberately does not assert that drain order matches the entries'
//! original array order: with many concurrent, independently-scheduled
//! submitters racing for a bounded channel and retrying on
//! [`IntakeError::QueueFull`], there is no single well-defined "submission
//! order" to preserve in the first place (two calls issued "concurrently"
//! have no canonical relative order, and a retry can let a
//! later-constructed submission enqueue before an earlier one that lost the
//! initial race). FIFO order *is* well-defined -- and is separately pinned
//! -- for the case where it actually applies: a single caller submitting
//! entries one at a time, see
//! `batch::builder::tests::fifo_order_preserved_across_multiple_batches`.

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use clock::tokio::AsyncClock;
use clock::FakeClock;
use mtc::Index;
use mtc_ca_service::batch::{self, BatchConfig, IntakeQueue};
use mtc_ca_service::{EntryIntake, IntakeError, LogEntry, SourceId, SourceType};
use proptest::prelude::*;

/// A small strategy for arbitrary [`LogEntry`] values: only `tbs_cert` and
/// `source_id` vary (enough to distinguish entries from one another), since
/// `SourceType`/`submitted_at` are not this property's concern.
fn arb_log_entry() -> impl Strategy<Value = LogEntry> {
    (proptest::collection::vec(any::<u8>(), 0..32), 0u64..10_000).prop_map(|(bytes, id)| {
        LogEntry::new(
            bytes,
            SourceType::NativeAcme,
            SourceId::from(format!("order-{id}")),
            UNIX_EPOCH,
        )
    })
}

/// A key that sorts [`LogEntry`] values deterministically, for multiset
/// comparison (drain order need not match submission order here -- see the
/// module docs). `LogEntry` itself derives neither `Ord` nor `Hash`, and
/// adding either just for this test helper is not worth widening its public
/// derive surface.
fn sort_key(entry: &LogEntry) -> (String, Vec<u8>) {
    (entry.source_id.as_str().to_string(), entry.tbs_cert.clone())
}

/// Submits `entry`, retrying on [`IntakeError::QueueFull`] (yielding between
/// attempts) until it is accepted or terminally rejected -- the backoff the
/// [`IntakeError::QueueFull`] docs ask callers for. Needed here because this
/// property submits far more entries concurrently than the deliberately
/// small test channel capacity (see `submit_and_drain_all`): without a
/// retry, entries that lose the initial race for a channel slot would be
/// dropped on the floor rather than merely delayed, and the property this
/// test checks -- *every* entry lands in *some* batch -- would not hold.
async fn submit_with_retry(intake: &IntakeQueue, entry: &LogEntry) -> Result<Index, IntakeError> {
    loop {
        match intake.submit_entry(entry.clone()).await {
            Err(IntakeError::QueueFull) => tokio::task::yield_now().await,
            result => return result,
        }
    }
}

/// Submits every entry in `entries` (concurrently, one spawned task each)
/// against a fresh batch channel, drains batches until every entry has come
/// back out, and returns them in drain order alongside each submitter's
/// resolved index.
///
/// Test-only helper (this whole file only compiles under `cargo test`); it
/// deliberately avoids `.unwrap()`/`.expect()` (rather than reaching for the
/// documented non-`#[test]`-helper scoped-allow, `docs/lint-policy.md`) since
/// plain `match`/`let-else` reads just as clearly here.
async fn submit_and_drain_all(
    entries: Vec<LogEntry>,
) -> (Vec<LogEntry>, Vec<Result<Index, IntakeError>>) {
    let clock = Arc::new(FakeClock::default());
    // Small max_batch_size (well under the production 256) so a sequence of
    // a few hundred entries exercises many batch boundaries without this
    // property test becoming slow; the boundary itself is separately pinned
    // at exactly 256 by a dedicated unit test
    // (`batch::builder::tests::size_trigger_emits_at_exactly_max_batch_size_without_advancing_the_clock`).
    let config = match BatchConfig::new(8, Duration::from_secs(3)) {
        Ok(config) => config,
        Err(err) => panic!("BatchConfig::new(8, 3s) must be valid: {err}"),
    };
    let (intake, mut builder) = batch::channel(config, Arc::clone(&clock) as Arc<dyn AsyncClock>);
    let intake = Arc::new(intake);

    let total = entries.len();
    let mut handles = Vec::with_capacity(total);
    for entry in entries {
        let intake = Arc::clone(&intake);
        handles.push(tokio::spawn(async move {
            submit_with_retry(&intake, &entry).await
        }));
    }

    // Background ticker: nudges the fake clock forward so a leftover
    // (below-`max_batch_size`) batch closes via the cadence trigger instead
    // of waiting forever. This property is about loss/duplication, not
    // timing, so it does not matter that the "elapsed" time is simulated
    // rather than real -- and because it is simulated, this never costs real
    // wall-clock time.
    let ticker = tokio::spawn({
        let clock = Arc::clone(&clock);
        async move {
            loop {
                tokio::task::yield_now().await;
                clock.advance(Duration::from_millis(500));
            }
        }
    });

    // The drain loop runs as its own spawned task rather than inline here:
    // `next_batch` needs to suspend and later be woken by the submitter and
    // ticker tasks above, and a `current_thread` runtime's `block_on` root
    // future (this async fn, driven via `rt.block_on` below) is not a
    // reliable place to drive that kind of cross-task wakeup -- see the
    // `batch` module docs' "Driving `next_batch`" section. Every
    // `batch::builder::tests` case that waits on the cadence trigger spawns
    // `next_batch` for the same reason.
    let drain = tokio::spawn(async move {
        let mut drained = Vec::with_capacity(total);
        let mut next_index = 0u64;
        while drained.len() < total {
            let Some(batch) = builder.next_batch().await else {
                panic!("channel closed with entries still outstanding -- entries were lost");
            };
            for entry in batch.into_entries() {
                drained.push(entry.log_entry.clone());
                entry.complete(Index(next_index));
                next_index += 1;
            }
        }
        drained
    });
    let drained = match drain.await {
        Ok(drained) => drained,
        Err(err) => panic!("drain task panicked: {err}"),
    };
    ticker.abort();

    let mut results = Vec::with_capacity(total);
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(err) => panic!("submitter task panicked: {err}"),
        }
    }
    (drained, results)
}

proptest! {
    #[test]
    fn every_entry_lands_in_exactly_one_batch(
        entries in proptest::collection::vec(arb_log_entry(), 0..600),
    ) {
        let rt = match tokio::runtime::Builder::new_current_thread().build() {
            Ok(rt) => rt,
            Err(err) => panic!("current-thread runtime: {err}"),
        };
        let mut expected = entries.clone();
        let (mut drained, results) = rt.block_on(submit_and_drain_all(entries));

        // Multiset equality (sorted comparison, not a straight `Vec` compare
        // -- see the module docs on why drain order is not expected to match
        // array order here): every submitted entry appears in the drained
        // output exactly as many times as it was submitted.
        drained.sort_by_key(sort_key);
        expected.sort_by_key(sort_key);
        prop_assert_eq!(
            drained, expected,
            "every submitted entry must appear in exactly one batch, no loss or duplication"
        );

        // Every submitter must resolve `Ok`, and the assigned indices must
        // be a permutation of 0..len -- no index assigned twice, none
        // skipped.
        let mut indices = Vec::with_capacity(results.len());
        for (i, result) in results.into_iter().enumerate() {
            match result {
                Ok(index) => indices.push(index.0),
                Err(err) => prop_assert!(false, "submitter {} did not resolve Ok: {:?}", i, err),
            }
        }
        indices.sort_unstable();
        let want_indices: Vec<u64> = (0..u64::try_from(indices.len()).unwrap_or(u64::MAX)).collect();
        prop_assert_eq!(
            indices, want_indices,
            "assigned indices must be a permutation of 0..N -- no duplicate or missing index"
        );
    }
}

/// Sanity check that an empty submission sequence never blocks or emits a
/// spurious batch (the degenerate case of the property above, called out
/// explicitly since `proptest`'s shrinker tends to land here first anyway).
#[tokio::test]
async fn no_submissions_means_no_batches() {
    let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
    let config = BatchConfig::new(8, Duration::from_secs(3)).unwrap_or_else(|err| {
        panic!("BatchConfig::new(8, 3s) must be valid: {err}");
    });
    let (intake, mut builder) = batch::channel(config, clock);

    intake.shutdown();
    let entry = LogEntry::new(
        b"tbs".to_vec(),
        SourceType::NativeAcme,
        SourceId::from("order-0"),
        UNIX_EPOCH,
    );
    assert_eq!(intake.submit_entry(entry).await, Err(IntakeError::Shutdown));
    assert!(builder.next_batch().await.is_none());
}
