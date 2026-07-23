//! Pure in-memory implementations of the four `cloud-types` traits
//! (spec §9.3, §9.6).
//!
//! Every type here holds its state in a process-local `Mutex`-guarded
//! collection — no Docker, no `LocalStack`, no `SoftHSM`. This is the
//! backend every other crate unit-tests against (spec §9.6):
//!
//! | Type | Implements | Spec |
//! |---|---|---|
//! | [`MemoryObjectStore`] | [`ObjectStore`](cloud_types::ObjectStore) | §9.1 |
//! | [`MemoryObjectLock`] | [`ObjectLock`](cloud_types::ObjectLock) | §9.1, §9.5 |
//! | [`MemoryReplicatedKv`] | [`ReplicatedKv`](cloud_types::ReplicatedKv) | §9.1, §9.5 |
//! | [`MemoryHsm`] | [`Hsm`](cloud_types::Hsm) | §9.1, §14 |
//!
//! # `MemoryObjectStore` / `MemoryObjectLock` share one type
//!
//! [`MemoryObjectLock`] is a type alias for [`MemoryObjectStore`]: the same
//! struct implements both traits. A client-side emulation of S3 Object Lock
//! has to share the exact object namespace with the store it locks —
//! [`ObjectStore::delete`](cloud_types::ObjectStore::delete) and
//! [`ObjectStore::put`](cloud_types::ObjectStore::put) (under
//! [`PutMode::Overwrite`](cloud_types::PutMode::Overwrite)) both have to see
//! whichever retention window
//! [`ObjectLock::put_with_retention`](cloud_types::ObjectLock::put_with_retention)
//! set. Production backends get this coupling for free (one S3 bucket with
//! Object Lock enabled); the memory backend has to construct it explicitly,
//! so it shares one `Arc`-backed map rather than running two independent
//! stores that could disagree about what is retained.
//!
//! ```
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! use clock::{Clock, FakeClock};
//! use cloud_memory::MemoryObjectStore;
//! use cloud_types::{ObjectLock, ObjectStore, PutOptions};
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! let clock = Arc::new(FakeClock::default());
//! let store = MemoryObjectStore::new(clock.clone());
//!
//! store.put("entries/0001", b"leaf", PutOptions::default()).await.unwrap();
//! assert_eq!(store.get("entries/0001").await.unwrap(), b"leaf");
//!
//! // ObjectLock::put_with_retention and ObjectStore::delete agree on
//! // retention because they read the same underlying map.
//! let retain_until = clock.now() + Duration::from_hours(1);
//! store
//!     .put_with_retention("checkpoints/0001", b"cp", retain_until)
//!     .await
//!     .unwrap();
//! assert!(store.delete("checkpoints/0001").await.is_err());
//! # }
//! ```
//!
//! # Zero external dependencies (spec §9.6)
//!
//! `cargo test -p cloud-memory` runs green with Docker stopped: unit tests
//! run in well under a second, and [`MemoryHsm`] never leaves the process
//! (`RustCrypto` `p256`, not a real HSM). See
//! [`Hsm::is_fips_validated`](cloud_types::Hsm::is_fips_validated) for the
//! corresponding honesty requirement.

#![warn(missing_docs)]

mod hsm;
mod object_lock;
mod object_store;
mod replicated_kv;

pub use hsm::MemoryHsm;
pub use object_lock::MemoryObjectLock;
pub use object_store::MemoryObjectStore;
pub use replicated_kv::MemoryReplicatedKv;
