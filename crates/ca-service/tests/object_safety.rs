//! Object-safety, `Arc<dyn EntryIntake>` usability, and source-provenance
//! flow-through assertions (spec §10.2-§10.4).
//!
//! Mirrors `crates/cloud-types/tests/object_safety.rs`'s pattern for the four
//! cloud-capability traits, applied to the one architectural-seam trait this
//! crate defines. `RecordingIntake` is a mock implementation only — the real
//! intake queue is ticket `mtc-2kx`.

// Test-only file: the production unwrap/expect ban does not apply here
// (docs/lint-policy.md).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use mtc::Index;
use mtc_ca_service::{EntryIntake, IntakeError, LogEntry, SourceId, SourceType};

/// A mock [`EntryIntake`] that assigns sequential indices and records every
/// entry it was handed, verbatim — standing in for the real intake queue
/// (ticket `mtc-2kx`) just enough to prove the trait's contract is usable.
struct RecordingIntake {
    next_index: AtomicU64,
    received: Mutex<Vec<LogEntry>>,
}

impl RecordingIntake {
    const fn new() -> Self {
        Self {
            next_index: AtomicU64::new(0),
            received: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl EntryIntake for RecordingIntake {
    async fn submit_entry(&self, entry: LogEntry) -> Result<Index, IntakeError> {
        let assigned = self.next_index.fetch_add(1, Ordering::SeqCst);
        self.received
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(entry);
        Ok(Index(assigned))
    }
}

const fn assert_send_sync<T: Send + Sync + ?Sized>() {}

/// Compile-time proof: `EntryIntake` is object-safe (`dyn EntryIntake` is a
/// valid type) and its trait object is `Send + Sync`, so `Arc<dyn
/// EntryIntake>` can be shared across adapter tasks (spec §10.3: adapters run
/// as independent Lambda/Fargate tasks, all funneling through one seam).
#[test]
fn entry_intake_is_object_safe_and_trait_object_is_send_sync() {
    assert_send_sync::<dyn EntryIntake>();
    assert_send_sync::<Arc<dyn EntryIntake>>();
}

#[tokio::test]
async fn submit_entry_is_callable_through_arc_dyn() {
    let intake: Arc<dyn EntryIntake> = Arc::new(RecordingIntake::new());

    let entry = LogEntry::new(
        vec![0x01, 0x02, 0x03],
        SourceType::NativeAcme,
        SourceId::from("order-1"),
        UNIX_EPOCH,
    );
    let assigned = intake
        .submit_entry(entry)
        .await
        .expect("recording intake always admits");
    assert_eq!(assigned, Index(0));
}

/// Spec AC: "`source_type`/`source_id` must flow through so batch state can
/// later persist them" (spec §8.2, §10.2 audit traceability). This proves
/// the envelope crosses the trait boundary unmodified — the seam does not
/// interpret, rewrite, or drop the provenance fields.
#[tokio::test]
async fn source_type_and_source_id_flow_through_unmodified() {
    let concrete = Arc::new(RecordingIntake::new());
    let intake: Arc<dyn EntryIntake> = concrete.clone();

    let entry = LogEntry::new(
        b"tbs-cert-bytes".to_vec(),
        SourceType::Adapter("aws-pca-adapter".to_string()),
        SourceId::from("event-99"),
        UNIX_EPOCH,
    );
    intake
        .submit_entry(entry)
        .await
        .expect("recording intake always admits");

    // Extract an owned copy while the guard is held, then drop it immediately
    // (clippy::significant_drop_tightening) — the assertions below don't need
    // the lock.
    let recorded = {
        let received = concrete
            .received
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(received.len(), 1);
        received[0].clone()
    };
    assert_eq!(
        recorded.source_type,
        SourceType::Adapter("aws-pca-adapter".to_string())
    );
    assert_eq!(recorded.source_id, SourceId::from("event-99"));
    assert_eq!(recorded.tbs_cert, b"tbs-cert-bytes".to_vec());
}

/// `Arc<dyn EntryIntake>` must be shareable across concurrently running
/// tokio tasks — the deployment shape of spec §10.3 (many adapter
/// Lambda/Fargate tasks, one shared intake seam) and the usage pattern of
/// the future intake queue (ticket `mtc-2kx`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn arc_dyn_entry_intake_is_shareable_across_tasks() {
    let intake: Arc<dyn EntryIntake> = Arc::new(RecordingIntake::new());

    let mut joins = Vec::new();
    for task in 0..8u8 {
        let intake = Arc::clone(&intake);
        joins.push(tokio::spawn(async move {
            let entry = LogEntry::new(
                vec![task],
                SourceType::NativeAcme,
                SourceId::from(format!("order-{task}")),
                UNIX_EPOCH,
            );
            intake
                .submit_entry(entry)
                .await
                .expect("recording intake always admits")
        }));
    }

    let mut assigned = Vec::new();
    for join in joins {
        assigned.push(join.await.expect("task completes"));
    }
    assigned.sort_by_key(|index| index.0);
    let expected: Vec<Index> = (0..8u64).map(Index).collect();
    assert_eq!(assigned, expected, "every task got a distinct index");
}
