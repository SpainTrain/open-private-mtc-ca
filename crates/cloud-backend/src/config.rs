//! [`BackendConfig`] / [`Provider`]: the runtime configuration
//! [`build_backend`](crate::build_backend) consumes (spec §9.4).
//!
//! [`BackendConfig`] derives [`serde::Deserialize`] but does not prescribe a
//! config *format* (YAML/JSON/TOML/...) -- that is the service's config
//! loader's concern, the same convention `crates/retention`'s
//! `RetentionPolicyConfig` documents. [`BackendConfig::from_env`] covers the
//! environment-variable path directly, mirroring
//! `cloud_softhsm::Pkcs11Config::from_env`/`from_lookup`.
//!
//! Only `provider` exists today. The spec §9.4 pseudocode sketches `cfg.aws`
//! / `cfg.localstack` sections; `cloud-backend-factory-aws`'s own AC ("gains
//! `aws`/`localstack` sections") is the ticket that adds them once those
//! providers have a real shape to validate -- adding empty placeholders here
//! now would be unvalidated, speculative shape.

use std::env;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};

/// Which concrete backend [`build_backend`](crate::build_backend) wires the
/// four `cloud-types` trait objects to (spec §9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    /// Pure in-memory backend (`cloud-memory`, spec §9.6) -- zero external
    /// dependencies, and the only provider `build_backend` fully implements
    /// so far.
    Memory,
    /// AWS production backend (`cloud-aws` + `CloudHSM`). Parses
    /// successfully; `build_backend` returns
    /// [`BackendError::Unimplemented`](crate::BackendError::Unimplemented)
    /// until `cloud-backend-factory-aws` lands.
    Aws,
    /// `LocalStack`-targeted dev/test backend (`cloud-localstack` +
    /// `cloud-softhsm`). Parses successfully; `build_backend` returns
    /// [`BackendError::Unimplemented`](crate::BackendError::Unimplemented)
    /// until `cloud-backend-factory-aws` lands.
    Localstack,
}

impl Provider {
    /// The canonical lowercase spelling used in config files, the
    /// `MTC_BACKEND_PROVIDER` environment variable, and error messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Aws => "aws",
            Self::Localstack => "localstack",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Provider {
    type Err = ConfigError;

    /// # Errors
    ///
    /// [`ConfigError::UnknownProvider`] if `raw` is not one of `memory`,
    /// `aws`, `localstack`.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "memory" => Ok(Self::Memory),
            "aws" => Ok(Self::Aws),
            "localstack" => Ok(Self::Localstack),
            other => Err(ConfigError::UnknownProvider {
                found: other.to_string(),
            }),
        }
    }
}

impl<'de> Deserialize<'de> for Provider {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Bridge through FromStr so config-file parsing and from_env share
        // one parsing rule and one error message.
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Failure resolving a [`Provider`] or [`BackendConfig`] (rule
/// `thiserror-for-libs-eyre-for-bins`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// A provider string did not match `memory`, `aws`, or `localstack`.
    /// Carries the offending value and the valid set so the message is
    /// actionable without a second lookup.
    #[error("unknown provider {found:?}; expected one of: memory, aws, localstack")]
    UnknownProvider {
        /// The unrecognized value that was supplied.
        found: String,
    },
    /// [`BackendConfig::ENV_PROVIDER`] was set but empty (or all-whitespace).
    /// Distinct from *unset* -- an unset variable falls back to
    /// [`Provider::Memory`] (spec §9.6), while an explicitly blank one is
    /// almost always a deployment mistake and fails loudly instead of
    /// silently defaulting (mirrors
    /// `cloud_softhsm::config::ConfigError::EmptyValue`).
    #[error("environment variable {var} is set but empty")]
    EmptyEnvVar {
        /// The offending environment variable name.
        var: &'static str,
    },
}

/// Top-level configuration consumed by
/// [`build_backend`](crate::build_backend) (spec §9.4).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendConfig {
    /// Which backend `build_backend` should wire up.
    pub provider: Provider,
}

impl BackendConfig {
    /// Environment variable naming the provider (`memory` / `aws` /
    /// `localstack`).
    pub const ENV_PROVIDER: &'static str = "MTC_BACKEND_PROVIDER";

    /// Resolves configuration from the `MTC_BACKEND_PROVIDER` environment
    /// variable, defaulting to [`Provider::Memory`] when unset -- the
    /// zero-external-dependency backend is the safe local-dev default (spec
    /// §9.6).
    ///
    /// # Errors
    ///
    /// [`ConfigError::EmptyEnvVar`] if the variable is set but blank;
    /// [`ConfigError::UnknownProvider`] if it is set to an unrecognized
    /// value.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    /// [`from_env`](Self::from_env), factored over an arbitrary lookup
    /// function so the resolution rule is unit-testable without mutating
    /// process-global environment state (mirrors
    /// `cloud_softhsm::Pkcs11Config::from_lookup`).
    ///
    /// # Errors
    ///
    /// Same as [`from_env`](Self::from_env).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let provider = match lookup(Self::ENV_PROVIDER) {
            Some(value) if value.trim().is_empty() => {
                return Err(ConfigError::EmptyEnvVar {
                    var: Self::ENV_PROVIDER,
                });
            }
            Some(value) => value.parse()?,
            None => Provider::Memory,
        };
        Ok(Self { provider })
    }
}
