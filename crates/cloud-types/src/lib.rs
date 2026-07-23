//! Cloud-agnostic capability traits for the MTC CA (spec §9).
//!
//! The CA service is built against four small interfaces — not against AWS
//! SDK calls — so that "what we need" stays separate from "where we get it"
//! (spec §9.1):
//!
//! | Trait | Capability | Example backends |
//! |---|---|---|
//! | [`ObjectStore`] | Durable, immutable object storage | S3, GCS, Azure Blob, MinIO, memory |
//! | [`ObjectLock`] | Storage-layer retention locking | S3 Object Lock, GCS Object Retention, Azure Immutable Storage |
//! | [`ReplicatedKv`] | Replicated KV with conditional + transactional writes | DynamoDB Global Tables, Firestore, Cosmos DB, Etcd, Postgres+CDC |
//! | [`Hsm`] | Hardware-backed signing | CloudHSM, Cloud HSM, Managed HSM, PKCS#11/SoftHSM2, memory |
//!
//! This crate contains only the trait definitions, common DTOs, and the
//! shared error taxonomy ([`CloudError`]). Backend implementations live in
//! sibling crates (`cloud-memory`, `cloud-aws`, `cloud-localstack`,
//! `cloud-softhsm` — spec §9.3) and are wired through a `Backend` factory
//! selected by runtime configuration (spec §9.4). Consumers depend on this
//! crate and never name a concrete backend:
//!
//! ```
//! use std::sync::Arc;
//!
//! use cloud_types::{Hsm, ObjectLock, ObjectStore, ReplicatedKv};
//!
//! // The §9.4 wiring shape: trait objects selected once at startup.
//! struct Backend {
//!     object_store: Arc<dyn ObjectStore>,
//!     object_lock: Arc<dyn ObjectLock>,
//!     replicated_kv: Arc<dyn ReplicatedKv>,
//!     hsm: Arc<dyn Hsm>,
//! }
//!
//! struct CaService {
//!     backend: Arc<Backend>,
//! }
//! ```
//!
//! # Capability bar (spec §9.5)
//!
//! The traits declare the *minimum* required capabilities; any backend that
//! meets them can host the CA, and a backend that lacks one isn't a fit — the
//! abstractions never paper over missing capabilities:
//!
//! | Capability | Required behavior | Why |
//! |---|---|---|
//! | Object durability | Eleven 9s practical durability | Log integrity |
//! | Object immutability | Bytes never change after write | Append-only invariant |
//! | Object retention lock | Cannot delete during retention window even by admins | True append-only at storage layer |
//! | Cross-region replication | Replication with bounded lag | DR |
//! | Conditional KV writes | Atomic compare-and-swap on attributes | Lease/epoch protocol |
//! | KV transactional writes | Atomic multi-item update | Linearization point of write-path step 8 |
//! | KV cross-region replication | Eventually consistent multi-region | Coordination state replication |
//! | HSM signing | FIPS 140-2 Level 3 (or equivalent) | Key protection |
//! | HSM cross-region key access | Key available wherever primary is | Failover |
//!
//! Each trait method's rustdoc states the slice of this bar it depends on.
//!
//! # Design rules
//!
//! - **Object safety**: every trait is usable as `Arc<dyn Trait>` with
//!   `Send + Sync` bounds (spec §9.4, §22.7) — backends are selected at
//!   runtime, initialized once, then shared across tasks.
//! - **No vendor types**: signatures use domain DTOs only; SDK types are
//!   translated inside backend crates (spec §22.8,
//!   .claude/rules/no-sdk-types-in-domain).
//! - **Typed errors**: every method returns [`CloudError`], a `thiserror`
//!   enum distinguishing not-found, already-exists, condition-failed,
//!   retention-violation, and retryable-vs-terminal transport failures.
//!
//! What is deliberately *not* abstracted (spec §9.2): compute platform,
//! event/scheduler triggers, CDN, DNS/health checks, and IAM — those are
//! deployment-boundary concerns; the abstraction lives inside the service
//! binary only.

#![warn(missing_docs)]

pub mod errors;
pub mod hsm;
pub mod object_lock;
pub mod object_store;
pub mod replicated_kv;

pub use errors::CloudError;
pub use hsm::{Hsm, KeyHandle, KeySpec, PublicKey};
pub use object_lock::ObjectLock;
pub use object_store::{ObjectInfo, ObjectMetadata, ObjectStore, PutMode, PutOptions};
pub use replicated_kv::{
    Condition, Item, Key, Operation, ReplicatedKv, UpdateAction, UpdateExpression, Value,
};
