//! The source-agnostic Stage-1/Stage-2 issuance intake seam (ticket
//! `mtc-kjl`; spec §10.2-§10.4 "Issuance Pipeline: Adapter Pattern", §25.9).
//!
//! The architecture splits certificate issuance into two stages (spec §10):
//!
//! ```text
//! ┌─────────────────────────────┐    ┌───────────────────────────────┐
//! │ Stage 1: Source-Specific     │    │ Stage 2: Log-the-Entry         │
//! │ Intake (Adapters)            │    │ Pipeline (Common)              │
//! │                               │    │                                 │
//! │ Native ACME endpoint  ───────┼───▶│ Entry intake queue             │
//! │ AWS Private CA adapter ──────┼───▶│  -> Batch builder               │
//! │ Keyfactor adapter ───────────┼───▶│  -> Tree updater                │
//! │ Cloudflare PCA adapter ──────┼───▶│  -> HSM signer                  │
//! │ ... future adapters ...      │    │  -> Commit (linearization point)│
//! └─────────────────────────────┘    └───────────────────────────────┘
//! ```
//!
//! This crate defines exactly the boundary between the two stages, and
//! nothing on either side of it:
//!
//! - [`LogEntry`] — the source-agnostic submission envelope (spec §10.2).
//! - [`SourceType`] / [`SourceId`] — provenance, carried but not interpreted,
//!   so batch state can later persist it for audit traceability (spec §8.2,
//!   §10.2).
//! - [`EntryIntake`] — the async trait every adapter submits through. See its
//!   rustdoc for the full adapter contract (spec §10.3).
//!
//! # What is deliberately *not* here
//!
//! Per ticket `mtc-kjl`'s scope and spec §10.4 ("future adapters are pure
//! additions"):
//!
//! - **The intake queue / batch-builder implementation** (ticket `mtc-2kx`)
//!   — the thing that actually implements [`EntryIntake`], drains the queue
//!   on a cadence/size trigger, and hands entries to the tree updater (spec
//!   §8.4, §11.1 steps 2-3).
//! - **The native ACME HTTP endpoint** (`acme-core`, and the future
//!   finalize/issuance wiring ticket) — the first *consumer* of
//!   [`EntryIntake`], not defined here.
//! - **Non-ACME adapters** (AWS Private CA, Keyfactor, Cloudflare PCA, ...) —
//!   post-v1 work; this seam exists so they are additions, not refactors.
//!
//! No ACME-specific (or any other adapter-specific) type appears anywhere in
//! this crate: [`SourceType::NativeAcme`] is a plain data label, not a
//! dependency on `acme-core` or any other adapter crate.
//!
//! # A worked example (mock adapter)
//!
//! ```
//! use std::sync::Arc;
//! use std::time::UNIX_EPOCH;
//!
//! use async_trait::async_trait;
//! use mtc::Index;
//! use mtc_ca_service::{EntryIntake, IntakeError, LogEntry, SourceId, SourceType};
//!
//! // A stand-in for the real intake queue (ticket mtc-2kx): assigns
//! // sequential indices in-memory, no batching.
//! struct SequentialIntake {
//!     next: std::sync::atomic::AtomicU64,
//! }
//!
//! #[async_trait]
//! impl EntryIntake for SequentialIntake {
//!     async fn submit_entry(&self, _entry: LogEntry) -> Result<Index, IntakeError> {
//!         let assigned = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
//!         Ok(Index(assigned))
//!     }
//! }
//!
//! # #[tokio::main]
//! # async fn main() {
//! // Adapters depend only on `Arc<dyn EntryIntake>` — this is the shape
//! // native ACME and every future adapter is wired against.
//! let intake: Arc<dyn EntryIntake> = Arc::new(SequentialIntake {
//!     next: std::sync::atomic::AtomicU64::new(0),
//! });
//!
//! let entry = LogEntry::new(
//!     b"serialized TbsCertificateLogEntry".to_vec(),
//!     SourceType::NativeAcme,
//!     SourceId::from("order-01H..."),
//!     UNIX_EPOCH, // production: read from an injected `Arc<dyn clock::Clock>`
//! );
//!
//! let assigned = intake.submit_entry(entry).await.expect("queue accepts");
//! assert_eq!(assigned, Index(0));
//! # }
//! ```

#![warn(missing_docs)]

mod entry;
mod intake;

pub use entry::{LogEntry, SourceId, SourceType};
pub use intake::{EntryIntake, IntakeError};
