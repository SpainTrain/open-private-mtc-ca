//! [`RetentionPolicyConfig`]: the service-config-facing shape of
//! [`RetentionPolicy`], deserialized via `serde` (ticket
//! `prune-retention-policy` AC: "`RetentionPolicy` type parsed from service
//! config with 7-year default").
//!
//! This crate does not prescribe a config *format* (YAML/JSON/TOML/env) —
//! that is the service's config loader's concern. `RetentionPolicyConfig`
//! derives [`serde::Deserialize`] so it embeds into whatever top-level
//! service config struct the CA binary defines, the same way any other
//! `serde`-based config section would.

use serde::Deserialize;

use crate::duration::{RetentionConfigError, RetentionDuration};
use crate::policy::RetentionPolicy;

/// Default retention window: 7 years (spec §15.1).
///
/// Approximated as 2555 days (365-day years — leap-day precision is not
/// meaningful for a multi-year compliance retention window, and matches how
/// S3 Object Lock itself expresses retention in whole days or years).
pub const DEFAULT_RETENTION_DAYS: i64 = 7 * 365;

/// Raw, `serde`-deserializable retention configuration (spec §15.1).
///
/// Deserializing an object that omits `retention_days` applies the 7-year
/// default ([`DEFAULT_RETENTION_DAYS`]); [`RetentionPolicyConfig::build`]
/// performs the "reject zero/negative durations" validation and produces a
/// [`RetentionPolicy`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPolicyConfig {
    /// Retention window for entries and tiles, in whole days. Defaults to
    /// [`DEFAULT_RETENTION_DAYS`] (7 years) when absent from the config
    /// source.
    #[serde(default = "default_retention_days")]
    pub retention_days: i64,
    /// Dev-mode override, in whole minutes (spec §18.4: "pruning is
    /// demoable on a laptop with the fake clock"). When set, this — not
    /// `retention_days` — becomes the effective entry/tile retention window;
    /// pruning checkpoints stay indefinite either way (spec §15.3, see
    /// [`ObjectClass::PruningCheckpoint`](crate::ObjectClass::PruningCheckpoint)).
    ///
    /// Intended for local dev/demo config only — production config leaves
    /// this unset.
    #[serde(default)]
    pub dev_override_minutes: Option<i64>,
}

const fn default_retention_days() -> i64 {
    DEFAULT_RETENTION_DAYS
}

impl Default for RetentionPolicyConfig {
    fn default() -> Self {
        Self {
            retention_days: DEFAULT_RETENTION_DAYS,
            dev_override_minutes: None,
        }
    }
}

impl RetentionPolicyConfig {
    /// Validates this config and builds a [`RetentionPolicy`].
    ///
    /// `retention_days` is always validated, even when `dev_override_minutes`
    /// is also set and becomes the effective duration — a malformed
    /// production value must not be able to hide behind an active dev
    /// override (it would otherwise resurface, unvalidated, the moment the
    /// override is removed).
    ///
    /// # Errors
    ///
    /// [`RetentionConfigError`] if `retention_days`, or `dev_override_minutes`
    /// when present, is zero or negative (or overflows the day/minute-to-
    /// second conversion).
    pub fn build(&self) -> Result<RetentionPolicy, RetentionConfigError> {
        let base = RetentionDuration::from_days(self.retention_days)?;
        let effective = match self.dev_override_minutes {
            Some(minutes) => RetentionDuration::from_minutes(minutes)?,
            None => base,
        };
        Ok(RetentionPolicy::from_duration(effective))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_seven_years_in_days() {
        assert_eq!(RetentionPolicyConfig::default().retention_days, 2555);
        assert_eq!(RetentionPolicyConfig::default().dev_override_minutes, None);
    }

    #[test]
    fn deserializes_empty_object_to_seven_year_default() {
        let cfg: RetentionPolicyConfig = serde_json::from_str("{}").expect("valid config");
        assert_eq!(cfg, RetentionPolicyConfig::default());
    }

    #[test]
    fn deserializes_explicit_retention_days() {
        let cfg: RetentionPolicyConfig =
            serde_json::from_str(r#"{"retention_days": 90}"#).expect("valid config");
        assert_eq!(cfg.retention_days, 90);
        assert_eq!(cfg.dev_override_minutes, None);
    }

    #[test]
    fn deserializes_dev_override_minutes() {
        let cfg: RetentionPolicyConfig =
            serde_json::from_str(r#"{"dev_override_minutes": 5}"#).expect("valid config");
        assert_eq!(cfg.retention_days, DEFAULT_RETENTION_DAYS);
        assert_eq!(cfg.dev_override_minutes, Some(5));
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = serde_json::from_str::<RetentionPolicyConfig>(r#"{"retention_dyas": 1}"#)
            .expect_err("typo'd field must be rejected");
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn build_default_yields_seven_year_policy() {
        let policy = RetentionPolicyConfig::default().build().expect("valid");
        assert_eq!(
            policy.entry_tile_duration().as_duration(),
            std::time::Duration::from_hours(2555 * 24)
        );
    }

    #[test]
    fn build_rejects_zero_retention_days() {
        let cfg = RetentionPolicyConfig {
            retention_days: 0,
            dev_override_minutes: None,
        };
        assert_eq!(cfg.build(), Err(RetentionConfigError::NonPositiveDays(0)));
    }

    #[test]
    fn build_rejects_negative_retention_days() {
        let cfg = RetentionPolicyConfig {
            retention_days: -30,
            dev_override_minutes: None,
        };
        assert_eq!(cfg.build(), Err(RetentionConfigError::NonPositiveDays(-30)));
    }

    #[test]
    fn build_rejects_zero_dev_override_even_with_valid_retention_days() {
        let cfg = RetentionPolicyConfig {
            retention_days: DEFAULT_RETENTION_DAYS,
            dev_override_minutes: Some(0),
        };
        assert_eq!(
            cfg.build(),
            Err(RetentionConfigError::NonPositiveMinutes(0))
        );
    }

    #[test]
    fn build_rejects_invalid_retention_days_even_when_dev_override_present() {
        // A malformed production value must not hide behind an active dev
        // override (see `build`'s rustdoc).
        let cfg = RetentionPolicyConfig {
            retention_days: -1,
            dev_override_minutes: Some(5),
        };
        assert_eq!(cfg.build(), Err(RetentionConfigError::NonPositiveDays(-1)));
    }

    #[test]
    fn dev_override_minutes_becomes_the_effective_duration() {
        let cfg = RetentionPolicyConfig {
            retention_days: DEFAULT_RETENTION_DAYS,
            dev_override_minutes: Some(3),
        };
        let policy = cfg.build().expect("valid");
        assert_eq!(
            policy.entry_tile_duration().as_duration(),
            std::time::Duration::from_mins(3)
        );
    }
}
