//! [`BatchBuilder`] and the [`Batch`]/[`BatchEntry`] types it emits — the
//! consumer half of the [`super`] batch channel (spec §11.1 step 2).
//!
//! [`BatchBuilder`] itself (and its `clock::tokio::AsyncClock` dependency)
//! is `#[cfg(not(loom))]`: see `super`'s module docs for why.

#[cfg(not(loom))]
use clock::tokio::AsyncClock;
use mtc::Index;
#[cfg(not(loom))]
use std::sync::Arc;
#[cfg(not(loom))]
use tokio::sync::mpsc;
use tokio::sync::oneshot;

#[cfg(not(loom))]
use super::BatchConfig;
use crate::{IntakeError, LogEntry};

/// One submitted entry, paired with the handle that resolves its
/// submitter's future.
///
/// The handle resolves
/// [`EntryIntake::submit_entry`](crate::EntryIntake::submit_entry)'s return
/// value once the downstream orchestrator (index allocation, tree update,
/// commit — spec §11.1 steps 3-9; ticket `mtc-22l`) finishes processing this
/// entry.
///
/// `log_entry` is `pub`: the batch consumer needs it to do that downstream
/// work. The completion channel is not: it is resolved exactly once, via
/// [`BatchEntry::complete`] or [`BatchEntry::fail`], which consume `self` so
/// a second resolution cannot compile.
pub struct BatchEntry {
    /// The entry as submitted (spec §10.2).
    pub log_entry: LogEntry,
    completion: oneshot::Sender<Result<Index, IntakeError>>,
}

impl BatchEntry {
    pub(super) const fn new(
        log_entry: LogEntry,
        completion: oneshot::Sender<Result<Index, IntakeError>>,
    ) -> Self {
        Self {
            log_entry,
            completion,
        }
    }

    /// Resolves the submitter's future with the index assigned during
    /// downstream processing (spec §11.1 steps 3/9).
    pub fn complete(self, index: Index) {
        // A dropped receiver means the submitter stopped waiting on its own
        // (e.g. its task was cancelled); nothing to do but let the send go
        // unobserved -- `EntryIntake::submit_entry` never panics either way.
        let _ = self.completion.send(Ok(index));
    }

    /// Resolves the submitter's future with an error (e.g. the batch was
    /// abandoned upstream — spec §11.2's lost-lease-mid-batch case).
    pub fn fail(self, error: IntakeError) {
        let _ = self.completion.send(Err(error));
    }
}

/// A group of entries ready for the downstream write path (spec §11.1 steps
/// 3 onward), emitted by [`BatchBuilder::next_batch`] once the cadence or
/// size trigger fires.
pub struct Batch {
    entries: Vec<BatchEntry>,
}

impl Batch {
    // `#[cfg(not(loom))]`: `Batch`'s only constructor is `next_batch`, which
    // is itself `#[cfg(not(loom))]` (see this module's docs) -- under loom
    // this would otherwise be dead code (it's `pub(super)`, so unlike
    // `Batch`'s fully-`pub` methods, the compiler can prove that).
    #[cfg(not(loom))]
    pub(super) const fn new(entries: Vec<BatchEntry>) -> Self {
        Self { entries }
    }

    /// Number of entries in this batch (spec §11.1 step 3 allocates exactly
    /// this many consecutive indices).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether this batch is empty. [`BatchBuilder::next_batch`] never
    /// returns an empty `Some(Batch)` (only `None`, when the channel is
    /// closed and fully drained) — provided for completeness and to satisfy
    /// `clippy::len_without_is_empty`.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Consumes the batch into its entries, in submission (FIFO) order, for
    /// the downstream orchestrator to process and then complete
    /// individually — the general case (e.g. a partially abandoned batch)
    /// that [`Batch::complete_sequential`]/[`Batch::fail_all`] don't cover.
    #[must_use]
    pub fn into_entries(self) -> Vec<BatchEntry> {
        self.entries
    }

    /// Completes every entry with sequential indices starting at `start`
    /// (`start`, `start + 1`, ... in submission order) and consumes the
    /// batch — the common case, matching spec §11.1 step 3's single
    /// contiguous `allocate_indices(batch.len(), epoch) -> (start, end)`.
    pub fn complete_sequential(self, start: Index) {
        for (offset, entry) in self.entries.into_iter().enumerate() {
            let offset = u64::try_from(offset).unwrap_or(u64::MAX);
            entry.complete(Index(start.0.saturating_add(offset)));
        }
    }

    /// Fails every entry in the batch with the same `error` and consumes the
    /// batch (e.g. the batch was abandoned upstream — spec §11.2).
    pub fn fail_all(self, error: IntakeError) {
        for entry in self.entries {
            entry.fail(error);
        }
    }
}

/// The consumer half of [`super::channel`].
///
/// Drains [`IntakeQueue`](super::IntakeQueue) submissions into [`Batch`]es,
/// emitting one when the cadence timer fires (via the injected
/// [`AsyncClock`] — rule `no-systemtime-now-in-prod`) or the batch reaches
/// [`BatchConfig::max_batch_size`], whichever comes first (spec §11.1
/// step 2).
///
/// Not `Clone`: exactly one task drains a given `BatchBuilder`, mirroring the
/// spec §11.4 pseudocode's `CaService` owning its `mpsc::Receiver<LogEntry>`
/// outright.
///
/// # Driving `next_batch`
///
/// Call [`BatchBuilder::next_batch`] from a task reached via `tokio::spawn`
/// (as every cadence-waiting case in this module's own tests does), not
/// inline as the literal root future of `#[tokio::main]`/`Runtime::block_on`
/// alongside other spawned tasks it depends on (submitters, a clock driver,
/// etc.). `next_batch` must suspend and later be woken by *those other
/// tasks*' progress; a `current_thread` runtime's `block_on` root future is
/// not a reliable place to drive that specific cross-task wakeup pattern —
/// confirmed by hand (see ticket `mtc-2kx`'s report): the identical wait,
/// run inline in `block_on`'s root future, reproducibly never wakes, while
/// the same wait spawned as its own task resolves promptly. A real
/// `CaService` write-path loop (ticket `mtc-22l`) would spawn this loop as
/// one of several concurrent service tasks anyway, so this is not expected
/// to constrain production wiring — only ad hoc `block_on`-in-`main`
/// call sites (tests, examples) that don't already do so.
#[cfg(not(loom))]
pub struct BatchBuilder {
    receiver: mpsc::Receiver<BatchEntry>,
    clock: Arc<dyn AsyncClock>,
    config: BatchConfig,
}

#[cfg(not(loom))]
impl BatchBuilder {
    pub(super) fn new(
        receiver: mpsc::Receiver<BatchEntry>,
        clock: Arc<dyn AsyncClock>,
        config: BatchConfig,
    ) -> Self {
        Self {
            receiver,
            clock,
            config,
        }
    }

    /// Waits for the next batch: accumulates submitted entries until either
    /// the configured cadence has elapsed since the *first* entry in this
    /// batch arrived, or the batch reaches [`BatchConfig::max_batch_size`] —
    /// whichever comes first (spec §11.1 step 2). The cadence deadline is
    /// fixed once, from the first entry's arrival time: it is not reset on
    /// every subsequent entry, or a steady stream would keep pushing it out
    /// and the cadence trigger would never fire. The cadence clock never
    /// runs while the queue is idle: with nothing accumulated yet, this
    /// waits indefinitely for a first entry rather than emitting empty
    /// batches on a timer.
    ///
    /// Returns `None` once the paired [`IntakeQueue`](super::IntakeQueue) has
    /// shut down and every already-submitted entry has been drained into a
    /// prior batch — the signal for the caller's loop to stop. A shutdown
    /// that lands mid-accumulation still yields `Some` first, with whatever
    /// had already accumulated (graceful drain), before a following call
    /// returns `None`.
    pub async fn next_batch(&mut self) -> Option<Batch> {
        let first = self.receiver.recv().await?;
        let max = self.config.max_batch_size();
        let mut entries = Vec::with_capacity(max);
        entries.push(first);

        // `checked_add` rather than `+`: on overflow (astronomically distant
        // cadences only), no cadence deadline is reachable, so only the size
        // trigger applies below -- mirrors `AsyncClock::sleep`'s own
        // overflow handling instead of panicking on the `SystemTime` add.
        let deadline = self.clock.now().checked_add(self.config.cadence());

        while entries.len() < max {
            let next = self.receiver.recv();
            let received = match deadline {
                Some(deadline) => {
                    tokio::select! {
                        entry = next => entry,
                        () = self.clock.sleep_until(deadline) => None,
                    }
                }
                None => next.await,
            };
            match received {
                Some(entry) => entries.push(entry),
                None => break,
            }
        }
        Some(Batch::new(entries))
    }
}

// Every case here exercises `BatchBuilder`, which is `#[cfg(not(loom))]`
// (see this module's docs).
#[cfg(all(test, not(loom)))]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    use clock::tokio::AsyncClock;
    use clock::FakeClock;
    use tokio::sync::{mpsc, oneshot};

    use super::{Batch, BatchBuilder, BatchEntry};
    use crate::batch::{sample_entry, BatchConfig};
    use crate::EntryIntake;
    use mtc::Index;

    /// Builds a disconnected `BatchBuilder` (no paired `IntakeQueue`) so
    /// tests can feed the raw channel directly and control timing precisely.
    fn builder_with_capacity(
        capacity: usize,
        config: BatchConfig,
        clock: Arc<dyn AsyncClock>,
    ) -> (mpsc::Sender<BatchEntry>, BatchBuilder) {
        let (tx, rx) = mpsc::channel(capacity);
        (tx, BatchBuilder::new(rx, clock, config))
    }

    fn queued(
        seed: u64,
    ) -> (
        BatchEntry,
        oneshot::Receiver<Result<Index, crate::IntakeError>>,
    ) {
        let (tx, rx) = oneshot::channel();
        (BatchEntry::new(sample_entry(seed), tx), rx)
    }

    #[tokio::test]
    async fn next_batch_returns_none_when_channel_closed_with_nothing_pending() {
        let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
        let (tx, mut builder) = builder_with_capacity(4, BatchConfig::production(), clock);
        drop(tx);

        assert!(builder.next_batch().await.is_none());
    }

    #[tokio::test]
    async fn size_trigger_emits_at_exactly_max_batch_size_without_advancing_the_clock() {
        let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
        let config = BatchConfig::new(256, Duration::from_secs(3)).expect("valid config");
        let (tx, mut builder) = builder_with_capacity(256, config, Arc::clone(&clock));

        let mut completions = Vec::with_capacity(256);
        for seed in 0..256u64 {
            let (entry, rx) = queued(seed);
            tx.try_send(entry).expect("capacity is exactly 256");
            completions.push(rx);
        }

        let batch = builder.next_batch().await.expect("batch emitted");
        assert_eq!(batch.len(), 256, "must fire at exactly the size trigger");
        assert_eq!(
            clock.now(),
            UNIX_EPOCH,
            "size trigger must not require the clock to advance at all"
        );

        batch.complete_sequential(Index(100));
        for (seed, rx) in completions.into_iter().enumerate() {
            let expected = Index(100 + u64::try_from(seed).expect("fits"));
            assert_eq!(rx.await.expect("sender not dropped"), Ok(expected));
        }
    }

    #[tokio::test]
    async fn cadence_trigger_emits_before_size_reached() {
        let clock = Arc::new(FakeClock::default());
        let config = BatchConfig::new(256, Duration::from_secs(3)).expect("valid config");
        let (tx, mut builder) =
            builder_with_capacity(256, config, Arc::clone(&clock) as Arc<dyn AsyncClock>);

        let (entry, completion) = queued(0);
        tx.try_send(entry).expect("capacity available");

        let batch_task = tokio::spawn(async move { builder.next_batch().await });

        // The builder has one entry (well below the 256 size trigger) and is
        // now parked on the cadence deadline. Give it several scheduler
        // turns -- it must still be waiting: a real wall clock would make
        // this test block for 3 real seconds here instead.
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(
            !batch_task.is_finished(),
            "next_batch must wait for the cadence deadline, not fire immediately"
        );

        clock.advance(Duration::from_secs(3));

        let batch = batch_task
            .await
            .expect("task did not panic")
            .expect("cadence trigger emits the batch");
        assert_eq!(batch.len(), 1);
        let mut entries = batch.into_entries();
        assert_eq!(entries.len(), 1);
        entries.remove(0).complete(Index(9));
        assert_eq!(completion.await.expect("sender not dropped"), Ok(Index(9)));
    }

    #[tokio::test]
    async fn fifo_order_preserved_across_multiple_batches() {
        let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
        let config = BatchConfig::new(4, Duration::from_secs(3)).expect("valid config");
        let (tx, mut builder) = builder_with_capacity(4, config, clock);

        // 10 entries over a 4-entry cap: batches of 4, 4, 2.
        for seed in 0..4u64 {
            let (entry, _rx) = queued(seed);
            tx.try_send(entry).expect("fits");
        }
        let first = builder.next_batch().await.expect("first batch");
        assert_eq!(first.len(), 4);
        assert_seeds(&first, &[0, 1, 2, 3]);

        for seed in 4..8u64 {
            let (entry, _rx) = queued(seed);
            tx.try_send(entry).expect("fits");
        }
        let second = builder.next_batch().await.expect("second batch");
        assert_seeds(&second, &[4, 5, 6, 7]);

        for seed in 8..10u64 {
            let (entry, _rx) = queued(seed);
            tx.try_send(entry).expect("fits");
        }
        drop(tx);
        let third = builder.next_batch().await.expect("third (partial) batch");
        assert_seeds(&third, &[8, 9]);

        assert!(builder.next_batch().await.is_none());
    }

    /// Asserts a batch's entries carry exactly the given seeds (see
    /// `sample_entry`), in order.
    fn assert_seeds(batch: &Batch, seeds: &[u64]) {
        let expected: Vec<_> = seeds.iter().copied().map(sample_entry).collect();
        let actual: Vec<_> = batch.entries.iter().map(|e| e.log_entry.clone()).collect();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn shutdown_mid_accumulation_still_emits_the_partial_batch() {
        let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
        let config = BatchConfig::new(256, Duration::from_secs(3)).expect("valid config");
        let (intake, mut builder) = crate::batch::channel(config, clock);
        let intake = Arc::new(intake);

        let submitter = {
            let intake = Arc::clone(&intake);
            tokio::spawn(async move { intake.submit_entry(sample_entry(0)).await })
        };

        // Let the submission land in the channel before shutting down.
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        intake.shutdown();

        let batch = builder
            .next_batch()
            .await
            .expect("the already-queued entry is still drained, not lost");
        assert_eq!(batch.len(), 1);
        batch.complete_sequential(Index(3));
        assert_eq!(submitter.await.unwrap(), Ok(Index(3)));

        // Nothing left: the channel is closed and fully drained.
        assert!(builder.next_batch().await.is_none());
    }

    #[tokio::test]
    async fn fail_all_fails_every_entry_with_the_same_error() {
        let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
        let config = BatchConfig::new(4, Duration::from_secs(3)).expect("valid config");
        let (tx, mut builder) = builder_with_capacity(4, config, clock);

        let mut completions = Vec::new();
        for seed in 0..3u64 {
            let (entry, rx) = queued(seed);
            tx.try_send(entry).expect("fits");
            completions.push(rx);
        }
        drop(tx);

        let batch = builder.next_batch().await.expect("partial batch drained");
        batch.fail_all(crate::IntakeError::NotPrimary);

        for rx in completions {
            assert_eq!(
                rx.await.expect("sender not dropped"),
                Err(crate::IntakeError::NotPrimary)
            );
        }
    }
}
