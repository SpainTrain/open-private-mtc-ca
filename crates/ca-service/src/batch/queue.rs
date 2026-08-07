//! [`IntakeQueue`]: the producer half of the [`super`] batch channel — a
//! bounded, `EntryIntake`-implementing submission queue (spec §8.4).

#[cfg(loom)]
use loom::sync::{Mutex, MutexGuard};
use std::sync::PoisonError;
#[cfg(not(loom))]
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use mtc::Index;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};

use super::BatchEntry;
use crate::{EntryIntake, IntakeError, LogEntry};

/// The producer half of [`super::channel`]: a bounded, multi-producer queue
/// that implements [`EntryIntake`] (spec §8.4's "Entry intake queue:
/// in-memory, drained by batch builder").
///
/// Cheap to share: wrap in `Arc` and hand the same instance to every
/// concurrently-running adapter task — the crate root docs already establish
/// `Arc<dyn EntryIntake>` as the shape every adapter depends on.
/// `submit_entry` takes `&self` and needs no external synchronization from
/// callers.
pub struct IntakeQueue {
    /// `None` once [`IntakeQueue::shutdown`] has been called. This is the
    /// *one* hand-rolled concurrency primitive in this module — ordering,
    /// backpressure signaling, and the actual buffering all come from
    /// `tokio::sync::mpsc`, tested independently upstream, so only this
    /// mutex-guarded gate is loom-checked (see `loom_tests` below). Run with:
    /// `RUSTFLAGS="--cfg loom" cargo test -p mtc-ca-service --lib --release`.
    sender: Mutex<Option<mpsc::Sender<BatchEntry>>>,
}

impl IntakeQueue {
    // Not `const fn`: `loom::sync::Mutex::new` (used under `--cfg loom`,
    // which needs to register the mutex with its model checker) isn't
    // `const`, unlike `std::sync::Mutex::new`, so this can't be
    // unconditionally const without duplicating the fn body per `cfg`.
    #[allow(clippy::missing_const_for_fn)]
    pub(super) fn new(sender: mpsc::Sender<BatchEntry>) -> Self {
        Self {
            sender: Mutex::new(Some(sender)),
        }
    }

    /// Stops accepting new entries. Idempotent.
    ///
    /// Entries already accepted (before this call observably takes effect)
    /// are unaffected: they remain buffered in the channel for the paired
    /// [`super::BatchBuilder`] to drain into a final batch (graceful drain —
    /// ticket AC). A `submit_entry` racing this call either completes its
    /// enqueue first (and is drained normally) or observes the shutdown and
    /// fails with [`IntakeError::Shutdown`] before ever touching the channel
    /// — never both, never neither (see `loom_tests`).
    pub fn shutdown(&self) {
        *lock(&self.sender) = None;
    }

    /// The synchronous, mutex-guarded core of [`EntryIntake::submit_entry`]:
    /// decides accept-or-reject and, if accepted, hands `item` to the
    /// channel. Kept separate from the `async fn` below so it can be driven
    /// directly — without an async executor — by the loom model below.
    fn try_enqueue(&self, item: BatchEntry) -> Result<(), IntakeError> {
        let guard = lock(&self.sender);
        let sender = guard.as_ref().ok_or(IntakeError::Shutdown)?;
        let result = sender.try_send(item).map_err(|err| match err {
            TrySendError::Full(_) => IntakeError::QueueFull,
            TrySendError::Closed(_) => IntakeError::Shutdown,
        });
        drop(guard);
        result
    }
}

#[async_trait]
impl EntryIntake for IntakeQueue {
    async fn submit_entry(&self, entry: LogEntry) -> Result<Index, IntakeError> {
        let (completion, rx) = oneshot::channel();
        self.try_enqueue(BatchEntry::new(entry, completion))?;
        // A dropped sender (the batch consumer discarded this entry's
        // `BatchEntry` without completing or failing it -- e.g. it is itself
        // shutting down) is indistinguishable from "no longer accepting
        // entries" from the submitter's point of view, so it maps to the
        // same `Shutdown` error rather than a panic or a hang.
        rx.await.unwrap_or(Err(IntakeError::Shutdown))
    }
}

/// Recovers from mutex poisoning instead of propagating a panic: nothing on
/// this lock's critical path can panic today, so poisoning would only ever
/// happen if that changes, and a stale-but-never-unsound `Option` (rule
/// `no-unwrap-in-prod` treats an unhandled `Result` the same as a `panic!`)
/// is preferable to turning every future `submit_entry`/`shutdown` call into
/// a panic as well.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(all(test, not(loom)))]
mod tests {
    use std::sync::Arc;

    use tokio::sync::{mpsc, oneshot};

    use super::{EntryIntake, IntakeError, IntakeQueue};
    use crate::batch::{sample_entry, BatchEntry};

    #[tokio::test]
    async fn submit_entry_succeeds_when_capacity_available() {
        let (tx, mut rx) = mpsc::channel(4);
        let queue = IntakeQueue::new(tx);

        let submitter = tokio::spawn(async move { queue.submit_entry(sample_entry(0)).await });

        let received = rx.recv().await.expect("entry reached the channel");
        assert_eq!(received.log_entry, sample_entry(0));
        received.complete(mtc::Index(42));

        assert_eq!(submitter.await.unwrap(), Ok(mtc::Index(42)));
    }

    #[tokio::test]
    async fn submit_entry_returns_queue_full_at_capacity() {
        let (tx, _rx) = mpsc::channel(1);
        let queue = IntakeQueue::new(tx);

        let (first_tx, _first_rx) = oneshot::channel();
        queue
            .try_enqueue(BatchEntry::new(sample_entry(0), first_tx))
            .expect("first entry fits");

        let overflow = queue.submit_entry(sample_entry(1)).await;
        assert_eq!(overflow, Err(IntakeError::QueueFull));
    }

    #[tokio::test]
    async fn submit_entry_returns_shutdown_after_shutdown_called() {
        let (tx, _rx) = mpsc::channel(4);
        let queue = IntakeQueue::new(tx);

        queue.shutdown();

        assert_eq!(
            queue.submit_entry(sample_entry(0)).await,
            Err(IntakeError::Shutdown)
        );
    }

    #[test]
    fn shutdown_is_idempotent() {
        let (tx, _rx) = mpsc::channel::<BatchEntry>(4);
        let queue = IntakeQueue::new(tx);
        queue.shutdown();
        queue.shutdown();
    }

    #[tokio::test]
    async fn dropped_batch_entry_without_completion_yields_shutdown_error() {
        let (tx, mut rx) = mpsc::channel(4);
        let queue = IntakeQueue::new(tx);

        let submitter = tokio::spawn(async move { queue.submit_entry(sample_entry(0)).await });

        // Drain and drop the entry without resolving it -- simulates a
        // consumer that abandons a drained batch instead of completing it.
        let received = rx.recv().await.expect("entry reached the channel");
        drop(received);

        assert_eq!(submitter.await.unwrap(), Err(IntakeError::Shutdown));
    }

    #[tokio::test]
    async fn queue_is_shareable_across_concurrent_adapter_tasks() {
        // Mirrors the crate-root doctest's `Arc<dyn EntryIntake>` shape:
        // many concurrent submitters against one shared queue.
        let (tx, mut rx) = mpsc::channel(8);
        let queue: Arc<dyn EntryIntake> = Arc::new(IntakeQueue::new(tx));

        let mut handles = Vec::new();
        for seed in 0..8u64 {
            let queue = Arc::clone(&queue);
            handles.push(tokio::spawn(async move {
                queue.submit_entry(sample_entry(seed)).await
            }));
        }

        for _ in 0..8 {
            rx.recv()
                .await
                .expect("entry reached the channel")
                .complete(mtc::Index(0));
        }
        for handle in handles {
            assert_eq!(handle.await.unwrap(), Ok(mtc::Index(0)));
        }
    }
}

/// Loom coverage for the one hand-rolled concurrency primitive in this
/// module: the `Mutex<Option<Sender>>` shutdown gate. Run with:
///
/// ```console
/// RUSTFLAGS="--cfg loom" cargo test -p mtc-ca-service --lib --release
/// ```
///
/// Everything else in the batch channel (FIFO ordering, backpressure, the
/// actual buffering) is `tokio::sync::mpsc`/`oneshot`, whose own internal
/// synchronization loom does not model unless tokio itself is built with
/// `--cfg loom` (a tokio-internal testing feature this workspace does not
/// opt into) -- that surface is exercised by the ordinary `#[tokio::test]`s
/// above instead. This mirrors the precedent in `crates/clock`: its loom
/// suite covers only `FakeClock`'s hand-rolled atomic compare-exchange loop,
/// not the tokio-`watch`-based async wakeups in `clock::tokio`.
#[cfg(all(test, loom))]
mod loom_tests {
    use loom::sync::Arc;
    use loom::thread;

    use super::IntakeQueue;
    use crate::batch::sample_entry;
    use crate::IntakeError;

    fn entry() -> super::BatchEntry {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        super::BatchEntry::new(sample_entry(0), tx)
    }

    #[test]
    fn shutdown_concurrent_with_submit_never_panics_and_is_final() {
        loom::model(|| {
            let (tx, _rx) = tokio::sync::mpsc::channel(4);
            let queue = Arc::new(IntakeQueue::new(tx));

            let submitter = {
                let queue = Arc::clone(&queue);
                thread::spawn(move || queue.try_enqueue(entry()))
            };
            let shutdown = {
                let queue = Arc::clone(&queue);
                thread::spawn(move || queue.shutdown())
            };

            let submit_result = submitter.join().unwrap();
            shutdown.join().unwrap();

            // Whichever interleaving loom explores, `try_enqueue` either
            // beat the shutdown (accepted -- the entry is now in the channel
            // for the builder to drain) or observed it (rejected up front)
            // -- never a panic, never silently lost.
            assert!(matches!(submit_result, Ok(()) | Err(IntakeError::Shutdown)));

            // Once shutdown has run (joined above), the gate can never
            // reopen: every subsequent submission is rejected.
            assert_eq!(queue.try_enqueue(entry()), Err(IntakeError::Shutdown));
        });
    }

    #[test]
    fn concurrent_submitters_before_shutdown_never_panic() {
        loom::model(|| {
            let (tx, _rx) = tokio::sync::mpsc::channel(4);
            let queue = Arc::new(IntakeQueue::new(tx));

            let a = {
                let queue = Arc::clone(&queue);
                thread::spawn(move || queue.try_enqueue(entry()))
            };
            let b = {
                let queue = Arc::clone(&queue);
                thread::spawn(move || queue.try_enqueue(entry()))
            };

            // Capacity 4 comfortably fits both concurrent submissions
            // regardless of interleaving -- both must succeed.
            assert_eq!(a.join().unwrap(), Ok(()));
            assert_eq!(b.join().unwrap(), Ok(()));
        });
    }
}
