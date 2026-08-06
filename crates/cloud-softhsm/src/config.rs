//! PKCS#11 connection configuration for the [`SoftHsm`](crate::SoftHsm)
//! backend (spec §4 HSM dev row, §18.1).
//!
//! The three connection parameters — module path, token label, and user PIN —
//! come from a [`Pkcs11Config`] built explicitly or resolved from the
//! `MTC_PKCS11_*` environment contract (`deploy/local/local.env`). Environment
//! values override the built-in dev defaults; an empty override is a
//! configuration error rather than a silent fall-through, so a blanked-out env
//! var fails loudly instead of connecting to the wrong token.

use std::env;

/// Environment variable naming the PKCS#11 module (shared object) to load.
pub const ENV_MODULE_PATH: &str = "MTC_PKCS11_MODULE_PATH";
/// Environment variable naming the token label to open.
pub const ENV_TOKEN_LABEL: &str = "MTC_PKCS11_TOKEN_LABEL";
/// Environment variable carrying the user PIN (dev-only, never a real secret).
pub const ENV_PIN: &str = "MTC_PKCS11_PIN";
/// Environment variable naming the pre-provisioned signing key's label.
///
/// Not part of the connection [`Pkcs11Config`] (keys are addressed per-call via
/// [`KeyHandle`](cloud_types::KeyHandle)); exported so consumers and tests can
/// reference the dev key the bootstrap script provisions (see
/// [`DEFAULT_KEY_LABEL`]).
pub const ENV_KEY_LABEL: &str = "MTC_PKCS11_KEY_LABEL";

/// Default module path inside the dev container / a Debian-style host install
/// (`deploy/local/local.env`, `deploy/local/README.md`).
pub const DEFAULT_MODULE_PATH: &str = "/usr/lib/softhsm/libsofthsm2.so";
/// Default dev token label provisioned by `scripts/softhsm-init.sh`.
pub const DEFAULT_TOKEN_LABEL: &str = "mtc-dev";
/// Default dev user PIN (dev-only; the token holds no production key material).
pub const DEFAULT_PIN: &str = "1234";
/// Default label of the pre-provisioned ECDSA P-256 signing key (spec §14.1).
pub const DEFAULT_KEY_LABEL: &str = "checkpoint-signing";

/// Failure building a [`Pkcs11Config`] from configuration input (rule
/// `thiserror-for-libs-eyre-for-bins`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// An environment variable was set but empty (or all-whitespace). Distinct
    /// from *unset* — an unset variable falls back to its documented default,
    /// while an explicitly blank one is almost always a deployment mistake.
    #[error("environment variable {var} is set but empty")]
    EmptyValue {
        /// The offending environment variable name.
        var: &'static str,
    },
}

/// Connection parameters for a `SoftHSM2` (or any PKCS#11) token.
///
/// The user PIN is held in memory for the lifetime of the backend (PKCS#11
/// requires it on every login) and is deliberately redacted from the [`Debug`]
/// representation so it never lands in a log line.
#[derive(Clone, PartialEq, Eq)]
pub struct Pkcs11Config {
    module_path: String,
    token_label: String,
    pin: String,
}

impl std::fmt::Debug for Pkcs11Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pkcs11Config")
            .field("module_path", &self.module_path)
            .field("token_label", &self.token_label)
            .field("pin", &"<redacted>")
            .finish()
    }
}

impl Pkcs11Config {
    /// Builds a config from explicit values.
    #[must_use]
    pub fn new(
        module_path: impl Into<String>,
        token_label: impl Into<String>,
        pin: impl Into<String>,
    ) -> Self {
        Self {
            module_path: module_path.into(),
            token_label: token_label.into(),
            pin: pin.into(),
        }
    }

    /// Resolves a config from the `MTC_PKCS11_*` environment contract, falling
    /// back to the dev defaults for any variable that is unset.
    ///
    /// # Errors
    ///
    /// [`ConfigError::EmptyValue`] if a variable is present but blank.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    /// Resolves a config from an arbitrary lookup function.
    ///
    /// Factored out of [`from_env`](Self::from_env) so the resolution rules are
    /// unit-testable without mutating process-global environment state.
    ///
    /// # Errors
    ///
    /// [`ConfigError::EmptyValue`] if a looked-up variable is present but blank.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        Ok(Self {
            module_path: resolve(&lookup, ENV_MODULE_PATH, DEFAULT_MODULE_PATH)?,
            token_label: resolve(&lookup, ENV_TOKEN_LABEL, DEFAULT_TOKEN_LABEL)?,
            pin: resolve(&lookup, ENV_PIN, DEFAULT_PIN)?,
        })
    }

    /// The PKCS#11 module (shared object) path to load.
    #[must_use]
    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    /// The token label to open.
    #[must_use]
    pub fn token_label(&self) -> &str {
        &self.token_label
    }

    /// The user PIN used to log in to the token.
    #[must_use]
    pub fn pin(&self) -> &str {
        &self.pin
    }
}

/// Resolves one value: an unset variable takes `default`; a present-but-blank
/// variable is a [`ConfigError::EmptyValue`].
fn resolve(
    lookup: &impl Fn(&str) -> Option<String>,
    var: &'static str,
    default: &str,
) -> Result<String, ConfigError> {
    match lookup(var) {
        Some(value) if value.trim().is_empty() => Err(ConfigError::EmptyValue { var }),
        Some(value) => Ok(value),
        None => Ok(default.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use pretty_assertions::assert_eq;

    use super::*;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    #[test]
    fn unset_variables_fall_back_to_dev_defaults() {
        let config = Pkcs11Config::from_lookup(lookup_from(&[])).unwrap();
        assert_eq!(config.module_path(), DEFAULT_MODULE_PATH);
        assert_eq!(config.token_label(), DEFAULT_TOKEN_LABEL);
        assert_eq!(config.pin(), DEFAULT_PIN);
    }

    #[test]
    fn environment_values_override_defaults() {
        let lookup = lookup_from(&[
            (ENV_MODULE_PATH, "/opt/homebrew/lib/softhsm/libsofthsm2.so"),
            (ENV_TOKEN_LABEL, "ci-token"),
            (ENV_PIN, "9999"),
        ]);
        let config = Pkcs11Config::from_lookup(lookup).unwrap();
        assert_eq!(
            config.module_path(),
            "/opt/homebrew/lib/softhsm/libsofthsm2.so"
        );
        assert_eq!(config.token_label(), "ci-token");
        assert_eq!(config.pin(), "9999");
    }

    #[test]
    fn blank_override_is_a_config_error_not_a_silent_default() {
        let err = Pkcs11Config::from_lookup(lookup_from(&[(ENV_TOKEN_LABEL, "   ")]))
            .expect_err("blank token label must error");
        assert_eq!(
            err,
            ConfigError::EmptyValue {
                var: ENV_TOKEN_LABEL
            }
        );
    }

    #[test]
    fn debug_redacts_the_pin() {
        let config = Pkcs11Config::new("/m.so", "tok", "supersecret");
        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains("supersecret"),
            "PIN must not appear in Debug output: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));
    }
}
