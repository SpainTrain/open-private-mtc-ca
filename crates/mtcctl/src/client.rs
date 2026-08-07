//! Builds the generated admin API client's [`Configuration`] from
//! [`GlobalArgs`] and classifies its errors into [`CliError`].
//!
//! Ticket mtc-no9 AC: "Uses the OpenAPI-generated Rust client (§17.2) for
//! all API calls; no hand-rolled HTTP". This module is the one place that
//! touches `mtc_admin_api_client::apis::configuration` / `apis::Error`
//! directly; command handlers under [`crate::commands`] call generated
//! `*_api` functions and this module's [`classify`], never `reqwest`.

use mtc_admin_api_client::apis::configuration::Configuration;
use mtc_admin_api_client::apis::Error;

use crate::cli::GlobalArgs;
use crate::error::CliError;

/// Builds a client [`Configuration`] pointed at `global.endpoint`.
#[must_use]
pub fn configuration(global: &GlobalArgs) -> Configuration {
    Configuration {
        base_path: global.endpoint.clone(),
        ..Configuration::new()
    }
}

/// Classifies a generated-client [`Error`] into a [`CliError`], so callers
/// get a distinct exit code (ticket mtc-no9 AC: "Errors map to distinct
/// exit codes") instead of one opaque failure.
#[must_use]
pub fn classify<T>(endpoint: &str, err: Error<T>) -> CliError {
    match err {
        Error::ResponseError(resp) => CliError::Api {
            status: resp.status.as_u16(),
            message: resp.content,
        },
        Error::Reqwest(source) => CliError::Connection {
            endpoint: endpoint.to_string(),
            source: Box::new(source),
        },
        Error::Serde(source) => CliError::Connection {
            endpoint: endpoint.to_string(),
            source: Box::new(source),
        },
        Error::Io(source) => CliError::Connection {
            endpoint: endpoint.to_string(),
            source: Box::new(source),
        },
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{classify, configuration};
    use crate::cli::{GlobalArgs, OutputFormat};
    use crate::error::CliError;

    fn global(endpoint: &str) -> GlobalArgs {
        GlobalArgs {
            output: OutputFormat::Human,
            endpoint: endpoint.to_string(),
            yes: false,
            confirm: false,
        }
    }

    #[test]
    fn configuration_uses_the_given_endpoint() {
        let config = configuration(&global("http://example:9000"));
        assert_eq!(config.base_path, "http://example:9000");
    }

    #[test]
    fn classify_maps_response_errors_to_api() {
        let err: mtc_admin_api_client::apis::Error<()> =
            mtc_admin_api_client::apis::Error::ResponseError(
                mtc_admin_api_client::apis::ResponseContent {
                    status: reqwest::StatusCode::from_u16(503).expect("503 is a valid status code"),
                    content: "service unavailable".to_string(),
                    entity: None,
                },
            );

        let classified = classify("http://localhost:8080", err);
        match classified {
            CliError::Api { status, message } => {
                assert_eq!(status, 503);
                assert_eq!(message, "service unavailable");
            }
            other => panic!("expected CliError::Api, got {other:?}"),
        }
    }

    #[test]
    fn classify_maps_serde_errors_to_connection() {
        // Any real `serde_json::Error` demonstrates the mapping without
        // needing a network call.
        let serde_err = serde_json::from_str::<serde_json::Value>("not json")
            .expect_err("malformed JSON fails to parse");
        let err: mtc_admin_api_client::apis::Error<()> =
            mtc_admin_api_client::apis::Error::Serde(serde_err);

        let classified = classify("http://localhost:8080", err);
        assert!(matches!(classified, CliError::Connection { .. }));
        assert_eq!(classified.exit_code(), 5);
    }
}
