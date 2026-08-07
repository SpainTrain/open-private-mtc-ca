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
//! - [`batch`] — the in-memory intake queue and batch builder (ticket
//!   `mtc-2kx`): the first, and so far only, implementation of
//!   [`EntryIntake`], draining it on a cadence/size trigger (spec §8.4,
//!   §11.1 step 2). See that module's docs for the full picture.
//!
//! # What is deliberately *not* here
//!
//! Per ticket `mtc-kjl`'s scope and spec §10.4 ("future adapters are pure
//! additions"):
//!
//! - **The tree updater onward** (index allocation through commit and
//!   delivery, spec §11.1 steps 3-9) — the downstream write-path orchestrator
//!   (ticket `mtc-22l`) that consumes [`batch`]'s emitted batches; out of
//!   scope for both this seam and the batch builder itself.
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
//! The [`batch`] module docs show the real, production-shaped intake queue;
//! this is a deliberately minimal stand-in demonstrating the trait contract
//! alone, with no batching.
//!
//! ```
//! use std::sync::Arc;
//! use std::time::UNIX_EPOCH;
//!
//! use async_trait::async_trait;
//! use mtc::Index;
//! use mtc_ca_service::{EntryIntake, IntakeError, LogEntry, SourceId, SourceType};
//!
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

pub mod batch;
mod entry;
mod intake;

pub use entry::{LogEntry, SourceId, SourceType};
pub use intake::{EntryIntake, IntakeError};
