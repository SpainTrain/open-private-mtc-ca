//! In-memory entry intake queue + batch builder (spec §8.4, §11.1 step 2;
//! ticket `mtc-2kx`) — the first (and so far only) implementation of the
//! [`crate::EntryIntake`] seam defined by ticket `mtc-kjl`.
//!
//! [`channel`] returns a paired [`IntakeQueue`] (implements
//! [`EntryIntake`](crate::EntryIntake); every adapter submits through it as
//! `Arc<dyn EntryIntake>`, exactly as the crate root docs already establish)
//! and [`BatchBuilder`] (owned by whichever task drains it — the future
//! `CaService` write-path loop, ticket `mtc-22l`, mirroring the spec §11.4
//! pseudocode's `CaService` owning an `mpsc::Receiver<LogEntry>` outright).
//! One bounded channel sits between them, sized to hold exactly one batch's
//! worth of entries (the builder drains continuously, so it never needs
//! more): that bound is what turns a full queue into
//! [`IntakeError::QueueFull`](crate::IntakeError::QueueFull) instead of an
//! unbounded backlog (spec §8.4).
//!
//! # Two triggers, one deadline
//!
//! [`BatchBuilder::next_batch`] accumulates entries until the batch reaches
//! [`BatchConfig::max_batch_size`] (spec §11.1 step 2's "full (256)") **or**
//! [`BatchConfig::cadence`] has elapsed since the batch's *first* entry
//! arrived (the "cadence (2-5s)" trigger) — whichever comes first. The
//! cadence clock never runs while the queue is idle: no entries means no
//! deadline means no empty batches. Timing is read exclusively through the
//! injected [`clock::tokio::AsyncClock`] (rule `no-systemtime-now-in-prod`):
//! `clock::SystemClock` in production, `clock::FakeClock` in tests and dev
//! mode (spec §18.4 time travel) — a real wall clock would make a
//! cadence-triggered wait take that many real seconds; a `FakeClock` resolves
//! it the instant it is advanced.
//!
//! # Completion handles
//!
//! Index allocation, the tree update, and commit (spec §11.1 steps 3-8) are
//! out of this ticket's scope (`mtc-22l`). What this module provides is the
//! *plumbing*: each [`BatchEntry`] in an emitted [`Batch`] carries the handle
//! that resolves its submitter's [`EntryIntake::submit_entry`](crate::EntryIntake::submit_entry)
//! future — [`BatchEntry::complete`]/[`BatchEntry::fail`] for one entry at a
//! time, or [`Batch::complete_sequential`]/[`Batch::fail_all`] for the whole
//! batch at once (the common case: step 3 allocates one contiguous index
//! range per batch). A submitter whose entry is drained into a batch that is
//! then dropped without being completed (e.g. a test, or a consumer that is
//! itself shutting down) observes [`IntakeError::Shutdown`] — the completion
//! channel closing with no answer is itself the shutdown signal.
//!
//! ```
//! use std::sync::Arc;
//! use std::time::UNIX_EPOCH;
//!
//! use clock::tokio::AsyncClock;
//! use clock::FakeClock;
//! use mtc::Index;
//! use mtc_ca_service::batch::{self, BatchConfig};
//! use mtc_ca_service::{EntryIntake, LogEntry, SourceId, SourceType};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let clock = Arc::new(FakeClock::default());
//! // max_batch_size 1: the size trigger fires as soon as the single entry
//! // below is submitted, so this example needs no cadence wait (the
//! // `batch::builder::tests::cadence_trigger_emits_before_size_reached` unit
//! // test, and `examples/batch_demo.rs`, exercise that trigger instead).
//! let config = BatchConfig::new(1, BatchConfig::production().cadence()).expect("1 is non-zero");
//! let (intake, mut builder) = batch::channel(config, Arc::clone(&clock) as Arc<dyn AsyncClock>);
//!
//! let entry = LogEntry::new(
//!     b"serialized TbsCertificateLogEntry".to_vec(),
//!     SourceType::NativeAcme,
//!     SourceId::from("order-01H..."),
//!     UNIX_EPOCH, // production: read from an injected `Arc<dyn clock::Clock>`
//! );
//!
//! // Downstream stand-in for the real orchestrator (ticket mtc-22l): drain
//! // the batch and resolve the entry's assigned index, concurrently with the
//! // submission that is waiting on it.
//! let (result, ()) = tokio::join!(intake.submit_entry(entry), async {
//!     let batch = builder.next_batch().await.expect("channel still open");
//!     assert_eq!(batch.len(), 1);
//!     batch.complete_sequential(Index(0));
//! });
//! assert_eq!(result, Ok(Index(0)));
//! # }
//! ```

mod builder;
mod queue;

use std::time::Duration;

use thiserror::Error;
// `BatchBuilder`, `channel`, and their `clock::tokio::AsyncClock` dependency
// are unavailable under `--cfg loom`: `clock::tokio` (the `tokio`-feature
// async-sleep helpers) is itself `#[cfg(not(loom))]` in `crates/clock` (its
// own watch-channel-based wakeups are not loom-modeled), so nothing built
// against it can compile under loom either. Only `queue`'s synchronous
// shutdown gate is loom-checked -- see `queue::loom_tests`.
#[cfg(not(loom))]
use clock::tokio::AsyncClock;
#[cfg(not(loom))]
use std::sync::Arc;
#[cfg(not(loom))]
use tokio::sync::mpsc;

#[cfg(not(loom))]
pub use builder::BatchBuilder;
pub use builder::{Batch, BatchEntry};
pub use queue::IntakeQueue;

/// Batch-emission tuning (spec §11.1 step 2, §18.1): how many entries
/// accumulate before the size trigger fires, and how long a batch's first
/// entry waits before the cadence trigger fires regardless.
///
/// Also sizes the bounded intake channel (spec §8.4): the queue never needs
/// to hold more than one batch's worth of entries at a time, since
/// [`BatchBuilder`] drains continuously, so `max_batch_size` doubles as the
/// channel capacity that backpressures
/// [`EntryIntake::submit_entry`](crate::EntryIntake::submit_entry) into
/// [`IntakeError::QueueFull`](crate::IntakeError::QueueFull).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchConfig {
    max_batch_size: usize,
    cadence: Duration,
}

impl BatchConfig {
    /// Production defaults (spec §11.1 step 2): a batch closes at 256
    /// entries or after 3 seconds — the midpoint of the spec's 2-5s cadence
    /// range — whichever comes first. Use [`BatchConfig::new`] for a
    /// different point in that range.
    #[must_use]
    pub const fn production() -> Self {
        Self {
            max_batch_size: 256,
            cadence: Duration::from_secs(3),
        }
    }

    /// Dev-mode defaults (spec §18.1: "CA service in dev mode (faster
    /// cadence, looser timing)" — the 60-second demo needs batches to close
    /// quickly). Same 256-entry cap; a much shorter cadence.
    #[must_use]
    pub const fn dev() -> Self {
        Self {
            max_batch_size: 256,
            cadence: Duration::from_millis(250),
        }
    }

    /// A custom configuration.
    ///
    /// # Errors
    ///
    /// Returns [`BatchConfigError::ZeroBatchSize`] if `max_batch_size` is 0:
    /// a zero-capacity bounded channel is a `tokio::sync::mpsc` panic (and a
    /// batch of zero entries is meaningless either way), so this is rejected
    /// here instead of surfacing as a panic when the channel is constructed.
    pub const fn new(max_batch_size: usize, cadence: Duration) -> Result<Self, BatchConfigError> {
        if max_batch_size == 0 {
            return Err(BatchConfigError::ZeroBatchSize);
        }
        Ok(Self {
            max_batch_size,
            cadence,
        })
    }

    /// The size trigger: a batch is emitted as soon as it holds this many
    /// entries, without waiting for the cadence (spec §11.1 step 2's "full
    /// (256)").
    #[must_use]
    pub const fn max_batch_size(&self) -> usize {
        self.max_batch_size
    }

    /// The cadence trigger: how long a batch's first entry waits before the
    /// batch is emitted regardless of size (spec §11.1 step 2's "cadence
    /// (2-5s)").
    #[must_use]
    pub const fn cadence(&self) -> Duration {
        self.cadence
    }
}

/// Failure modes of [`BatchConfig::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BatchConfigError {
    /// `max_batch_size` was 0: a batch (and the channel it backs) must hold
    /// at least one entry.
    #[error("batch size must be at least 1")]
    ZeroBatchSize,
}

/// Creates a paired [`IntakeQueue`]/[`BatchBuilder`] (spec §8.4).
///
/// [`IntakeQueue`] is the producer half, implementing
/// [`EntryIntake`](crate::EntryIntake); [`BatchBuilder`] is the consumer
/// half. They share one bounded channel of capacity
/// `config.max_batch_size()`.
///
/// `clock` drives the cadence trigger — inject `clock::SystemClock` in
/// production, `clock::FakeClock` in tests/dev mode (rule
/// `no-systemtime-now-in-prod`).
#[cfg(not(loom))]
#[must_use]
pub fn channel(config: BatchConfig, clock: Arc<dyn AsyncClock>) -> (IntakeQueue, BatchBuilder) {
    let (tx, rx) = mpsc::channel(config.max_batch_size());
    (IntakeQueue::new(tx), BatchBuilder::new(rx, clock, config))
}

/// Test-only helper shared by `queue`'s and `builder`'s test modules (and the
/// FIFO property test): a minimal, distinguishable [`crate::LogEntry`].
#[cfg(test)]
pub(crate) fn sample_entry(seed: u64) -> crate::LogEntry {
    crate::LogEntry::new(
        format!("tbs-cert-{seed}").into_bytes(),
        crate::SourceType::NativeAcme,
        crate::SourceId::from(format!("order-{seed}")),
        std::time::UNIX_EPOCH,
    )
}

#[cfg(all(test, not(loom)))]
mod config_tests {
    use super::{BatchConfig, BatchConfigError};
    use std::time::Duration;

    #[test]
    fn dev_config_keeps_the_256_cap_with_a_short_cadence() {
        let dev = BatchConfig::dev();
        assert_eq!(dev.max_batch_size(), 256);
        assert_eq!(dev.cadence(), Duration::from_millis(250));
    }

    #[test]
    fn new_rejects_a_zero_batch_size_and_accepts_a_positive_one() {
        assert!(matches!(
            BatchConfig::new(0, Duration::from_secs(3)),
            Err(BatchConfigError::ZeroBatchSize)
        ));
        assert!(BatchConfig::new(1, Duration::from_secs(3)).is_ok());
    }
}
