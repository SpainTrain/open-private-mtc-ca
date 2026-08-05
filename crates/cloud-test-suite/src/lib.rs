//! Shared factory-closure conformance suites for `cloud-types` backends
//! (spec §9.7).
//!
//! Every implementation of [`ObjectStore`](cloud_types::ObjectStore),
//! [`ObjectLock`](cloud_types::ObjectLock),
//! [`ReplicatedKv`](cloud_types::ReplicatedKv), and [`Hsm`](cloud_types::Hsm)
//! must pass the identical suite defined here: `cloud-memory` today,
//! `cloud-aws` (via `LocalStack`) and `cloud-softhsm` (via `SoftHSM2`) in
//! follow-on tickets. Spec §9.5 is blunt about why this exists: "a backend
//! that lacks any of these [capabilities] isn't a fit" -- these suites are
//! how that bar gets enforced mechanically, on every backend, instead of by
//! inspection of each backend's own bespoke tests.
//!
//! # Factory pattern (spec §9.7)
//!
//! Every suite takes an async factory closure -- `Fn() -> Fut` where
//! `Fut: Future<Output = S>` -- rather than one pre-built instance, so the
//! suite can create as many independent backend instances as its cases need
//! (some cases construct more than one instance to avoid interfering with a
//! sibling case's state under the same key):
//!
//! ```
//! use std::sync::Arc;
//!
//! use clock::FakeClock;
//! use cloud_memory::MemoryObjectStore;
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! cloud_test_suite::run_object_store_suite(|| async {
//!     MemoryObjectStore::new(Arc::new(FakeClock::default()))
//! })
//! .await;
//! # }
//! ```
//!
//! # Adding a new backend
//!
//! Implement the four `cloud-types` traits, then add one `tests/` file per
//! trait to the new crate calling the matching `run_*_suite` function --
//! see `crates/cloud-memory/tests/` for the worked example. No new
//! assertions to write, and no copy-pasting the memory backend's test
//! bodies: the same suite functions are the contract every backend proves.

#![warn(missing_docs)]

pub mod hsm;
pub mod object_lock;
pub mod object_store;
pub mod replicated_kv;

pub use hsm::run_hsm_suite;
pub use object_lock::run_object_lock_suite;
pub use object_store::run_object_store_suite;
pub use replicated_kv::run_replicated_kv_suite;
