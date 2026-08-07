//! [`StorageConfig`]: runtime configuration for
//! [`S3DdbStorage`](crate::S3DdbStorage) (ticket `mtc-f35` AC).
//!
//! [`StorageConfig`] derives [`serde::Deserialize`] but, like
//! `cloud_backend::BackendConfig`, does not prescribe a config *format*
//! (YAML/JSON/TOML/...) -- that is the service's config loader's concern.

use serde::{Deserialize, Serialize};

/// Configuration for [`S3DdbStorage`](crate::S3DdbStorage).
///
/// Carries the object-store bucket and `ReplicatedKv` table it writes through
/// (via the cloud-agnostic `Backend`), the log's coordination partition-key
/// prefix (spec §8.2), and a retention window.
///
/// # Scope note: `retention_days`, not `retention::RetentionPolicyConfig`
///
/// The `retention` crate (ticket `prune-retention-policy`) already owns the
/// real retention-policy shape and `retain_until` math (spec §15.1, §15.3).
/// This crate's ticket scopes its dependency graph to `cloud-types` +
/// `cloud-backend` + `mtc` only, so `StorageConfig` carries a plain day-count
/// here rather than embedding that crate's type. A future ticket wiring
/// retention-aware writes (e.g. `write_entries`/`write_tiles` calling
/// `ObjectLock::put_with_retention`) may deliberately widen this boundary and
/// replace the field with `retention::RetentionPolicyConfig` directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    /// Object-store bucket (or bucket-shaped namespace) entries, tiles, and
    /// checkpoints are written to (spec §9.1, §11.4).
    pub bucket: String,
    /// `ReplicatedKv` table (or table-shaped namespace) coordination items
    /// (lease, counter, batch state) are written to (spec §8.2, §9.1).
    pub table: String,
    /// The log identifier every coordination item's `log#{logId}`
    /// partition-key prefix is built from (spec §8.2). Kept as a raw
    /// `String` rather than `mtc::LogId` because `mtc`'s domain newtypes are
    /// not `serde`-enabled and this field must deserialize.
    pub log_id: String,
    /// Retention window, in days, for written objects (spec §15.1). See the
    /// scope note above for why this is a day-count rather than the
    /// `retention` crate's own config type.
    pub retention_days: u32,
}

impl StorageConfig {
    /// The `log#{logId}` coordination-table partition-key prefix (spec
    /// §8.2), formatted the same way `coordination::protocol::lease_key`
    /// formats it, so every coordination item under this log agrees on the
    /// prefix.
    #[must_use]
    pub fn coordination_prefix(&self) -> String {
        format!("log#{}", self.log_id)
    }
}

#[cfg(test)]
mod tests {
    use super::StorageConfig;

    fn sample() -> StorageConfig {
        StorageConfig {
            bucket: "mtc-prod-log-1".to_string(),
            table: "mtc-coordination".to_string(),
            log_id: "prod-log-1".to_string(),
            retention_days: 2555,
        }
    }

    #[test]
    fn coordination_prefix_matches_the_log_hash_convention() {
        assert_eq!(sample().coordination_prefix(), "log#prod-log-1");
    }

    #[test]
    fn round_trips_through_json() {
        let config = sample();
        let json = serde_json::to_string(&config).expect("serializes");
        let parsed: StorageConfig = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(parsed, config);
    }

    #[test]
    fn rejects_unknown_fields() {
        let raw = r#"{"bucket":"b","table":"t","log_id":"l","retention_days":1,"typo":true}"#;
        let err = serde_json::from_str::<StorageConfig>(raw).expect_err("typo must be rejected");
        assert!(err.to_string().contains("unknown field"));
    }
}
