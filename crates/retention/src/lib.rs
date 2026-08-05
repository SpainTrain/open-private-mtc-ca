//! Retention policy configuration and `retain_until` computation (spec §15
//! "Pruning and Retention", §9.1 the `ObjectLock` interface).
//!
//! Ticket `prune-retention-policy`: config + duration math only — this
//! crate has no crypto surface, does not touch the cloud abstraction traits
//! in production code, and does not compute *which* objects are prunable
//! (that is the separate `prune-planner` ticket). Its whole job is: given a
//! [`RetentionPolicy`] and the time an object was written, compute the
//! `retain_until` instant that
//! [`ObjectLock::put_with_retention`][put_with_retention] (spec §9.1,
//! `crates/cloud-types`) needs.
//!
//! [put_with_retention]: https://docs.rs/cloud-types (in-repo: `crates/cloud-types/src/object_lock.rs`)
//!
//! # Quick start
//!
//! ```
//! use std::time::{Duration, SystemTime};
//! use retention::{ObjectClass, RetentionPolicyConfig};
//!
//! // Parsed from service config (spec §15.1); empty config -> 7-year default.
//! let config = RetentionPolicyConfig::default();
//! let policy = config.build().expect("default config is always valid");
//!
//! // `write_time` comes from the caller's injected `Arc<dyn clock::Clock>`
//! // (rule no-systemtime-now-in-prod) — never `SystemTime::now()` directly.
//! let write_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
//!
//! // Entries and tiles: write_time + the configured duration.
//! let entry_retain_until = policy
//!     .retain_until(ObjectClass::Entry, write_time)
//!     .expect("no overflow for realistic write times");
//! assert!(entry_retain_until > write_time);
//!
//! // Pruning checkpoints: always the indefinite sentinel (spec §15.3),
//! // regardless of write_time or the configured duration.
//! let checkpoint_retain_until = policy
//!     .retain_until(ObjectClass::PruningCheckpoint, write_time)
//!     .expect("indefinite retention never overflows");
//! assert_eq!(checkpoint_retain_until, retention::indefinite_retention());
//! ```
//!
//! # Dev-mode override (spec §18.4)
//!
//! ```
//! use retention::RetentionPolicyConfig;
//!
//! // A local/dev config sets `dev_override_minutes` so pruning is demoable
//! // without waiting out a 7-year retention window on the fake clock.
//! let dev_config = RetentionPolicyConfig {
//!     retention_days: 2555,
//!     dev_override_minutes: Some(5),
//! };
//! let policy = dev_config.build().expect("valid dev config");
//! assert_eq!(
//!     policy.entry_tile_duration().as_duration(),
//!     std::time::Duration::from_secs(5 * 60),
//! );
//! ```

mod config;
mod duration;
mod policy;

pub use config::{RetentionPolicyConfig, DEFAULT_RETENTION_DAYS};
pub use duration::{RetentionConfigError, RetentionDuration};
pub use policy::{indefinite_retention, ObjectClass, RetentionError, RetentionPolicy};
