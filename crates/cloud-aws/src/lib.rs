//! `cloud-aws`: AWS-backed implementations of the `cloud-types` traits, the
//! v1 production backend (spec §9.3). This crate currently implements
//! [`ObjectStore`](cloud_types::ObjectStore) and
//! [`ObjectLock`](cloud_types::ObjectLock) over `aws-sdk-s3`
//! ([`S3ObjectStore`]); `DynamoDB` `ReplicatedKv` and `CloudHSM` land in
//! follow-on tickets in this same crate (spec §9.3 crate layout:
//! `ddb_replicated_kv.rs`, `cloudhsm.rs`).
//!
//! Exercised exclusively against `LocalStack` -- no real AWS spend (spec §1
//! non-goals). [`S3Config`] takes an injected endpoint/credentials pair so
//! `LocalStack` and (hypothetically) real AWS run the exact same code path
//! (ticket aws-backend AC); see [`S3Config::localstack`] for the dev-mode
//! convenience constructor and `deploy/local/docker-compose.yml` for the
//! container this crate's integration tests target.
//!
//! # `S3ObjectStore` / `S3ObjectLock` share one type
//!
//! Mirrors `cloud-memory`'s `MemoryObjectStore`/`MemoryObjectLock` pattern
//! (see that crate's docs): one struct implements both
//! [`ObjectStore`](cloud_types::ObjectStore) and
//! [`ObjectLock`](cloud_types::ObjectLock), because real S3 couples them the
//! same way -- one bucket with Object Lock enabled backs both capabilities.
//! [`S3ObjectLock`] is a type alias for [`S3ObjectStore`].
//!
//! # Bucket prerequisites (out of this crate's scope)
//!
//! The target bucket must already have **versioning** and **Object Lock**
//! enabled -- this crate never creates or configures a bucket (ticket
//! aws-backend Out of Scope: "S3 bucket layout/schema", storage-facade
//! epic). `deploy/local/localstack/init/ready.d/01-init-mtc.sh` provisions
//! the CA's real `mtc-log-local` bucket this way; this crate's own
//! integration tests provision a *separate*, freshly-named bucket per test
//! run instead (`tests/support/mod.rs::provision_test_bucket`), for two
//! reasons:
//!
//! - `mtc-log-local` also sets a **bucket-level default retention rule**
//!   (Compliance, 1 day), which would attach Object Lock retention to every
//!   plain [`ObjectStore::put`](cloud_types::ObjectStore::put), breaking the
//!   shared suite's `test_delete_removes_object` case (a plain put followed
//!   by an immediate delete must succeed). A test bucket without a
//!   default-retention rule keeps `ObjectStore::put` genuinely unretained,
//!   matching the `cloud-types` contract exactly.
//! - The shared suites use fixed key names, and the `ObjectLock` suite
//!   genuinely, permanently retains some of them
//!   (`put_with_retention`). Re-running the suite against a bucket left over
//!   from a previous run would collide with those still-locked objects, so
//!   each test run gets its own bucket instead of reusing one.
//!
//! # S3 Object Lock semantics vs. the `cloud-types` contract
//!
//! S3's object model is fundamentally *versioned*: a "key" is really a
//! stack of immutable versions, and Object Lock retention protects
//! individual *versions*, not keys. The `cloud-types` `ObjectStore`/
//! `ObjectLock` contract, by contrast, models a single mutable-or-locked
//! object per key. Reconciling the two required three deliberate choices,
//! each also documented at its call site:
//!
//! - **[`ObjectStore::delete`](cloud_types::ObjectStore::delete) uses a
//!   *versioned* `DeleteObject`.** A plain, unversioned S3 `DELETE` on a
//!   locked key does not fail -- it adds a delete marker, making the object
//!   invisible to unversioned `GET`/`HEAD` while the locked bytes are still
//!   physically present underneath. That would make `delete` on a retained
//!   object look like it succeeded when the storage layer actually refused
//!   it -- the opposite of the §9.5 bar ("cannot delete during retention
//!   window even by admins"). Instead, `delete` resolves the key's current
//!   `version_id` via `HeadObject` and issues a `DeleteObject` *for that
//!   specific version*: S3 permanently removes it if unlocked, or rejects
//!   with `AccessDenied` (mapped to
//!   [`RetentionViolation`](cloud_types::CloudError::RetentionViolation)) if
//!   the version is still under active Compliance retention -- both
//!   outcomes evaluated by S3 against its own real clock, so retention
//!   *expiry* (not exercised by the shared suite -- see
//!   `cloud_test_suite::run_object_lock_suite`'s docs) unblocks deletion
//!   correctly with no client-side clock needed.
//! - **`put` under [`PutMode::Overwrite`](cloud_types::PutMode::Overwrite)
//!   refuses any key that has *ever* carried Object Lock retention.** A
//!   plain S3 `PutObject` never touches an existing version -- it just
//!   creates a new *current* version -- so it would silently "succeed" over
//!   a locked object without S3 rejecting anything at all, defeating the
//!   §9.5 bar from the other direction. This crate closes that gap with an
//!   app-level check (`GetObjectRetention` before the write): once a key has
//!   *any* Object Lock retention configuration, `Overwrite` mode refuses it
//!   permanently -- this crate does not attempt to re-derive "has retention
//!   expired" client-side (that would need a locally-tracked "now", which
//!   production code cannot obtain outside an injected `Clock`, and which
//!   would disagree with S3's own clock anyway). In this system that is
//!   also the *correct* behavior regardless of expiry: objects written via
//!   [`ObjectLock::put_with_retention`](cloud_types::ObjectLock::put_with_retention)
//!   are append-only log content by construction (spec §8) and are never
//!   legitimate targets for `PutMode::Overwrite` -- pruning goes through
//!   `ObjectStore::delete` (above), which *does* defer to S3's live,
//!   time-accurate enforcement.
//! - **`put_with_retention` sets `x-amz-object-lock-mode` /
//!   `-retain-until-date` in the *same* `PutObject` call as the
//!   `If-None-Match: *` conditional create.** One atomic request, so there
//!   is no window where the object exists unretained.
//!
//! # `LocalStack` emulation caveats (community edition, image pinned in
//! `deploy/local/docker-compose.yml`)
//!
//! This crate's `--features integration` tests run the exact scenarios
//! above against a real `LocalStack` container, and both suites pass in
//! full -- **no suite case is skipped for this backend.** Two real,
//! empirically-observed characteristics (S3 platform behavior, not
//! `LocalStack` shortcuts) shaped the implementation and are worth calling
//! out for anyone extending this crate:
//!
//! - **`HeadObject` on a missing key reports error code `NotFound`, not
//!   `NoSuchKey`.** A `HEAD` response has no body to carry an XML `<Code>`
//!   element, so the SDK synthesizes a generic code from the bare HTTP 404
//!   instead. [`ObjectStore::head`](cloud_types::ObjectStore::head) and
//!   [`ObjectStore::delete`](cloud_types::ObjectStore::delete) (which heads
//!   the key first) both map `"NotFound"` to
//!   [`CloudError::NotFound`](cloud_types::CloudError::NotFound) alongside
//!   `"NoSuchKey"`/`"NoSuchVersion"` -- see `src/error.rs::classify`.
//! - **Object Lock retain-until dates carry only second precision on the
//!   wire.** `x-amz-object-lock-retain-until-date` (and the value
//!   `GetObjectRetention` echoes back) has no fractional-second component.
//!   [`S3ObjectLock::put_with_retention`]/`get_retention` round-trip exactly
//!   at that precision -- callers supplying a `SystemTime` with a
//!   fractional-second component get back a `SystemTime` truncated to the
//!   second. This crate's integration test therefore injects a
//!   whole-second-truncating clock rather than `clock::SystemClock` (below);
//!   real callers of [`ObjectLock::put_with_retention`](cloud_types::ObjectLock::put_with_retention)
//!   should not assume sub-second retention precision survives.
//!
//! See `tests/object_store_suite.rs` and `tests/object_lock_suite.rs`,
//! which run the identical `cloud-test-suite` conformance suites
//! `cloud-memory` runs against the pure-memory backend.
//!
//! # Clock choice for the shared `ObjectLock` suite
//!
//! `cloud_test_suite::run_object_lock_suite` takes an `Arc<dyn Clock>` to
//! generate `retain_until` instants (`clock.now() + Duration::from_hours(1)`,
//! spec §22.11). `cloud-memory`'s own tests inject a `FakeClock`; this
//! crate's integration tests inject a real clock instead -- S3 evaluates
//! retention windows against its own real wall-clock time, so a `FakeClock`
//! seeded near `UNIX_EPOCH` would generate `retain_until` dates deep in the
//! past, which S3 rejects as invalid at write time. Concretely, the test
//! wraps `clock::SystemClock` in a whole-second-truncating adapter (see
//! `tests/object_lock_suite.rs::SecondPrecisionClock`) so the suite's own
//! round-trip assertion (`put_with_retention` then `get_retention` returns
//! exactly what was sent) is meaningful against S3's second-precision
//! retention dates (previous section).

#![warn(missing_docs)]

mod config;
mod error;
mod s3_object_lock;
mod s3_object_store;

pub use config::{S3Config, StaticCredentials};
pub use s3_object_lock::S3ObjectLock;
pub use s3_object_store::S3ObjectStore;
