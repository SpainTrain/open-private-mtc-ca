//! `BackendConfig` / `Provider` parsing tests (ticket `cloud-backend-factory`
//! Testing AC: "config deserialization ... unknown-provider rejection").

use cloud_backend::{BackendConfig, ConfigError, Provider};

#[test]
fn deserializes_each_known_provider() {
    for (raw, expected) in [
        (r#"{"provider":"memory"}"#, Provider::Memory),
        (r#"{"provider":"aws"}"#, Provider::Aws),
        (r#"{"provider":"localstack"}"#, Provider::Localstack),
    ] {
        let cfg: BackendConfig = serde_json::from_str(raw).expect("valid config");
        assert_eq!(cfg.provider, expected);
    }
}

#[test]
fn unknown_provider_string_is_rejected_with_an_actionable_message() {
    let err = serde_json::from_str::<BackendConfig>(r#"{"provider":"azure"}"#)
        .expect_err("unrecognized provider must be rejected");
    let message = err.to_string();
    assert!(message.contains("azure"), "message: {message}");
    assert!(message.contains("memory"), "message: {message}");
    assert!(message.contains("aws"), "message: {message}");
    assert!(message.contains("localstack"), "message: {message}");
}

#[test]
fn rejects_unknown_fields() {
    let err = serde_json::from_str::<BackendConfig>(r#"{"provider":"memory","typo":true}"#)
        .expect_err("typo'd field must be rejected");
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn from_lookup_defaults_to_memory_when_unset() {
    let cfg = BackendConfig::from_lookup(|_| None).expect("unset falls back to the default");
    assert_eq!(cfg.provider, Provider::Memory);
}

#[test]
fn from_lookup_parses_a_set_value() {
    let cfg = BackendConfig::from_lookup(|name| {
        (name == BackendConfig::ENV_PROVIDER).then(|| "aws".to_string())
    })
    .expect("valid override");
    assert_eq!(cfg.provider, Provider::Aws);
}

#[test]
fn from_lookup_rejects_blank_value_instead_of_silently_defaulting() {
    let err = BackendConfig::from_lookup(|name| {
        (name == BackendConfig::ENV_PROVIDER).then(|| "   ".to_string())
    })
    .expect_err("blank override must error");
    assert_eq!(
        err,
        ConfigError::EmptyEnvVar {
            var: BackendConfig::ENV_PROVIDER,
        }
    );
}

#[test]
fn from_lookup_rejects_unknown_value() {
    let err = BackendConfig::from_lookup(|name| {
        (name == BackendConfig::ENV_PROVIDER).then(|| "azure".to_string())
    })
    .expect_err("unknown override must error");
    assert_eq!(
        err,
        ConfigError::UnknownProvider {
            found: "azure".to_string(),
        }
    );
}

#[test]
fn from_env_reads_the_documented_variable() {
    // Exercises the real from_env -> env::var path once (not just
    // from_lookup) without mutating global process state for other tests:
    // the documented variable is read but never set here, so this only
    // proves from_env delegates to from_lookup with std::env::var, matching
    // whatever the ambient environment happens to provide (typically unset
    // in CI, which from_lookup's own tests already show falls back to
    // Provider::Memory).
    let by_lookup = BackendConfig::from_lookup(|name| std::env::var(name).ok());
    assert_eq!(BackendConfig::from_env(), by_lookup);
}
