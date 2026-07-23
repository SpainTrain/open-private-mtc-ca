//! Environment-driven configuration for one replication link.
//!
//! One `dev-replicator` process instance is one directed link (source →
//! target), replicating an S3 bucket, a `DynamoDB` table, or both between the
//! two — the ticket's "N instances for arbitrary directed-link topologies"
//! is realized by running one process per edge of the desired topology graph
//! (`dev-multiregion-harness` wires the compose services; see the crate's
//! top-level docs for the env-var contract table).

use std::net::SocketAddr;
use std::time::Duration;

use crate::lag::LagPolicy;

/// Endpoint + region for one side (source or target) of a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointConfig {
    /// `LocalStack` endpoint URL, e.g. `http://127.0.0.1:4566`.
    pub endpoint_url: String,
    /// AWS region string the client presents (`LocalStack` accepts any value
    /// that looks like a region; it does not have to match a real one).
    pub region: String,
}

/// Full configuration for one link, parsed from environment variables (see
/// [`LinkConfig::from_env`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkConfig {
    /// Human-readable link name, used in log lines and the control
    /// endpoint's `/status` (e.g. `us-east-1-to-us-west-2`).
    pub link_name: String,
    /// The region this link replicates *from*.
    pub source: EndpointConfig,
    /// The region this link replicates *to*.
    pub target: EndpointConfig,
    /// S3 bucket to replicate (same name on both sides — production CRR
    /// buckets are identically named across regions, spec §8.1). `None`
    /// disables S3 replication on this link.
    pub s3_bucket: Option<String>,
    /// `DynamoDB` table to replicate (same name on both sides, spec §8.2).
    /// `None` disables `DynamoDB` replication on this link.
    pub ddb_table: Option<String>,
    /// How often each poller checks its source for new changes.
    pub poll_interval: Duration,
    /// Lag policy at startup; runtime-adjustable via the control endpoint
    /// (mr-replication-sim AC).
    pub initial_lag: LagPolicy,
    /// Local address the control HTTP endpoint binds (spec §18.3 "the
    /// partition hook"; also serves `/status`).
    pub control_addr: SocketAddr,
}

/// Errors parsing [`LinkConfig`] from the environment.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    /// A required environment variable was not set.
    #[error("missing required environment variable {0}")]
    MissingEnv(&'static str),
    /// An environment variable was set but could not be parsed.
    #[error("invalid value for {var}={value:?}: {reason}")]
    InvalidValue {
        /// The environment variable name.
        var: &'static str,
        /// The value that failed to parse.
        value: String,
        /// Why it was rejected.
        reason: String,
    },
    /// Neither `REPL_S3_BUCKET` nor `REPL_DDB_TABLE` was set — the link
    /// would replicate nothing.
    #[error(
        "at least one of REPL_S3_BUCKET or REPL_DDB_TABLE must be set — a link with neither \
         replicates nothing"
    )]
    NoResourceConfigured,
}

const DEFAULT_REGION: &str = "us-east-1";
const DEFAULT_POLL_INTERVAL_MS: u64 = 500;
const DEFAULT_CONTROL_ADDR: &str = "127.0.0.1:9300";

impl LinkConfig {
    /// Parses a [`LinkConfig`] from the process environment.
    ///
    /// | Variable                    | Required | Default             |
    /// |------------------------------|----------|---------------------|
    /// | `REPL_LINK_NAME`              | yes      | —                   |
    /// | `REPL_SOURCE_ENDPOINT_URL`    | yes      | —                   |
    /// | `REPL_SOURCE_REGION`          | no       | `us-east-1`         |
    /// | `REPL_TARGET_ENDPOINT_URL`    | yes      | —                   |
    /// | `REPL_TARGET_REGION`          | no       | `us-east-1`         |
    /// | `REPL_S3_BUCKET`              | no*      | unset (S3 off)      |
    /// | `REPL_DDB_TABLE`               | no*      | unset (DDB off)     |
    /// | `REPL_LAG_MS`                  | no       | `0`                 |
    /// | `REPL_STALL`                   | no       | unset (not stalled) |
    /// | `REPL_POLL_INTERVAL_MS`        | no       | `500`               |
    /// | `REPL_CONTROL_ADDR`            | no       | `127.0.0.1:9300`    |
    ///
    /// \* at least one of `REPL_S3_BUCKET` / `REPL_DDB_TABLE` is required.
    /// `REPL_STALL=1` (or `true`) starts the link stalled regardless of
    /// `REPL_LAG_MS`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if a required variable is missing, a value
    /// fails to parse, or neither resource is configured.
    pub fn from_env() -> Result<Self, ConfigError> {
        // Wrapped in a closure (not passed as `std::env::var` directly):
        // the free function's monomorphized-per-call-site signature doesn't
        // satisfy the higher-ranked `Fn(&str) -> _` bound `from_env_lookup`
        // needs; a closure adapts to any lifetime.
        Self::from_env_lookup(|key| std::env::var(key))
    }

    /// Testable variant taking an explicit lookup function instead of the
    /// real process environment.
    pub(crate) fn from_env_lookup(
        lookup: impl Fn(&str) -> Result<String, std::env::VarError>,
    ) -> Result<Self, ConfigError> {
        let required = |var: &'static str| lookup(var).map_err(|_| ConfigError::MissingEnv(var));
        let optional = |var: &str| lookup(var).ok();

        let link_name = required("REPL_LINK_NAME")?;
        let source = EndpointConfig {
            endpoint_url: required("REPL_SOURCE_ENDPOINT_URL")?,
            region: optional("REPL_SOURCE_REGION").unwrap_or_else(|| DEFAULT_REGION.to_string()),
        };
        let target = EndpointConfig {
            endpoint_url: required("REPL_TARGET_ENDPOINT_URL")?,
            region: optional("REPL_TARGET_REGION").unwrap_or_else(|| DEFAULT_REGION.to_string()),
        };
        let s3_bucket = optional("REPL_S3_BUCKET");
        let ddb_table = optional("REPL_DDB_TABLE");
        if s3_bucket.is_none() && ddb_table.is_none() {
            return Err(ConfigError::NoResourceConfigured);
        }

        let poll_interval_ms = match optional("REPL_POLL_INTERVAL_MS") {
            Some(raw) => parse_u64("REPL_POLL_INTERVAL_MS", &raw)?,
            None => DEFAULT_POLL_INTERVAL_MS,
        };

        let stalled = match optional("REPL_STALL") {
            Some(raw) => parse_bool("REPL_STALL", &raw)?,
            None => false,
        };
        let initial_lag = if stalled {
            LagPolicy::Stalled
        } else {
            let lag_ms = match optional("REPL_LAG_MS") {
                Some(raw) => parse_u64("REPL_LAG_MS", &raw)?,
                None => 0,
            };
            LagPolicy::Fixed(Duration::from_millis(lag_ms))
        };

        let control_addr_raw =
            optional("REPL_CONTROL_ADDR").unwrap_or_else(|| DEFAULT_CONTROL_ADDR.to_string());
        let control_addr =
            control_addr_raw
                .parse::<SocketAddr>()
                .map_err(|e| ConfigError::InvalidValue {
                    var: "REPL_CONTROL_ADDR",
                    value: control_addr_raw,
                    reason: e.to_string(),
                })?;

        Ok(Self {
            link_name,
            source,
            target,
            s3_bucket,
            ddb_table,
            poll_interval: Duration::from_millis(poll_interval_ms),
            initial_lag,
            control_addr,
        })
    }
}

fn parse_u64(var: &'static str, raw: &str) -> Result<u64, ConfigError> {
    raw.parse::<u64>().map_err(|e| ConfigError::InvalidValue {
        var,
        value: raw.to_string(),
        reason: e.to_string(),
    })
}

fn parse_bool(var: &'static str, raw: &str) -> Result<bool, ConfigError> {
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" | "" => Ok(false),
        _ => Err(ConfigError::InvalidValue {
            var,
            value: raw.to_string(),
            reason: "expected 1/true/yes or 0/false/no".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from<'a>(
        vars: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Result<String, std::env::VarError> + 'a {
        let map: HashMap<&str, &str> = vars.iter().copied().collect();
        move |key| {
            map.get(key)
                .map(|v| (*v).to_string())
                .ok_or(std::env::VarError::NotPresent)
        }
    }

    fn base_vars() -> Vec<(&'static str, &'static str)> {
        vec![
            ("REPL_LINK_NAME", "us-east-1-to-us-west-2"),
            ("REPL_SOURCE_ENDPOINT_URL", "http://127.0.0.1:4566"),
            ("REPL_TARGET_ENDPOINT_URL", "http://127.0.0.1:4567"),
            ("REPL_S3_BUCKET", "mtc-log-local"),
        ]
    }

    #[test]
    fn parses_minimal_valid_config_with_defaults() {
        let cfg = LinkConfig::from_env_lookup(lookup_from(&base_vars())).unwrap();
        assert_eq!(cfg.link_name, "us-east-1-to-us-west-2");
        assert_eq!(cfg.source.region, DEFAULT_REGION);
        assert_eq!(cfg.target.region, DEFAULT_REGION);
        assert_eq!(cfg.s3_bucket.as_deref(), Some("mtc-log-local"));
        assert_eq!(cfg.ddb_table, None);
        assert_eq!(
            cfg.poll_interval,
            Duration::from_millis(DEFAULT_POLL_INTERVAL_MS)
        );
        assert_eq!(cfg.initial_lag, LagPolicy::Fixed(Duration::ZERO));
        assert_eq!(cfg.control_addr, DEFAULT_CONTROL_ADDR.parse().unwrap());
    }

    #[test]
    fn missing_required_var_is_reported_by_name() {
        let mut vars = base_vars();
        vars.retain(|(k, _)| *k != "REPL_LINK_NAME");
        let err = LinkConfig::from_env_lookup(lookup_from(&vars)).unwrap_err();
        assert_eq!(err, ConfigError::MissingEnv("REPL_LINK_NAME"));
    }

    #[test]
    fn requires_at_least_one_resource() {
        let mut vars = base_vars();
        vars.retain(|(k, _)| *k != "REPL_S3_BUCKET");
        let err = LinkConfig::from_env_lookup(lookup_from(&vars)).unwrap_err();
        assert_eq!(err, ConfigError::NoResourceConfigured);
    }

    #[test]
    fn ddb_table_alone_is_sufficient() {
        let mut vars = base_vars();
        vars.retain(|(k, _)| *k != "REPL_S3_BUCKET");
        vars.push(("REPL_DDB_TABLE", "mtc-log-coordination"));
        let cfg = LinkConfig::from_env_lookup(lookup_from(&vars)).unwrap();
        assert_eq!(cfg.ddb_table.as_deref(), Some("mtc-log-coordination"));
        assert_eq!(cfg.s3_bucket, None);
    }

    #[test]
    fn parses_explicit_lag_ms() {
        let mut vars = base_vars();
        vars.push(("REPL_LAG_MS", "5000"));
        let cfg = LinkConfig::from_env_lookup(lookup_from(&vars)).unwrap();
        assert_eq!(cfg.initial_lag, LagPolicy::Fixed(Duration::from_secs(5)));
    }

    #[test]
    fn stall_overrides_lag_ms() {
        let mut vars = base_vars();
        vars.push(("REPL_LAG_MS", "5000"));
        vars.push(("REPL_STALL", "true"));
        let cfg = LinkConfig::from_env_lookup(lookup_from(&vars)).unwrap();
        assert_eq!(cfg.initial_lag, LagPolicy::Stalled);
    }

    #[test]
    fn invalid_lag_ms_is_a_typed_error() {
        let mut vars = base_vars();
        vars.push(("REPL_LAG_MS", "not-a-number"));
        let err = LinkConfig::from_env_lookup(lookup_from(&vars)).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidValue {
                var: "REPL_LAG_MS",
                ..
            }
        ));
    }

    #[test]
    fn invalid_control_addr_is_a_typed_error() {
        let mut vars = base_vars();
        vars.push(("REPL_CONTROL_ADDR", "not-an-addr"));
        let err = LinkConfig::from_env_lookup(lookup_from(&vars)).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidValue {
                var: "REPL_CONTROL_ADDR",
                ..
            }
        ));
    }
}
