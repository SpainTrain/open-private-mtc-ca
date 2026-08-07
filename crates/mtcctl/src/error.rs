//! Typed CLI errors.
//!
//! Rule `thiserror-for-libs-eyre-for-bins`: [`crate::run`] and everything it
//! calls is this crate's "library" surface, kept typed so `main.rs` can map
//! errors to distinct exit codes (ticket mtc-no9 AC: "Errors map to
//! distinct exit codes; stderr for diagnostics, stdout for payload").
//!
//! clap owns exit code `2` for usage errors: `Cli::parse()` in `main.rs`
//! calls `std::process::exit(2)` itself on a parse failure, before any
//! [`CliError`] is ever constructed. The codes below are this crate's own,
//! chosen to be distinct from clap's and from each other.

/// Errors surfaced by [`crate::run`] and the command handlers it dispatches
/// to.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// A leaf declared in the spec §17.3 tree whose operation isn't
    /// implemented yet (per-operation tickets; see ticket mtc-no9's Out of
    /// Scope).
    #[error("{0} is not yet implemented")]
    NotImplemented(&'static str),

    /// The admin API responded with a non-2xx status.
    #[error("admin API error ({status}): {message}")]
    Api {
        /// HTTP status code returned by the admin API.
        status: u16,
        /// Response body (raw; the admin API's error envelope is not
        /// always the typed shape a given operation declares).
        message: String,
    },

    /// Could not complete the request to the admin API at all: transport,
    /// TLS, or (de)serialization failure below the HTTP-status level.
    #[error("could not reach the admin API at {endpoint}: {source}")]
    Connection {
        /// The endpoint that was unreachable.
        endpoint: String,
        /// The underlying transport/serialization failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Rendering the response in the requested `--output` format failed.
    ///
    /// Expected to be unreachable for the generated response types this
    /// crate renders today (plain structs of strings/numbers/timestamps),
    /// but `serde_json`/`serde_yaml` are fallible APIs and rule
    /// `no-unwrap-in-prod` forbids assuming otherwise.
    #[error("failed to render output: {0}")]
    Render(String),
}

impl CliError {
    /// The process exit code this error maps to (distinct per variant, per
    /// ticket mtc-no9 AC).
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Render(_) => 1,
            Self::NotImplemented(_) => 3,
            Self::Api { .. } => 4,
            Self::Connection { .. } => 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::CliError;

    #[test]
    fn exit_codes_are_distinct_per_variant() {
        let render = CliError::Render("boom".to_string());
        let not_implemented = CliError::NotImplemented("batch list");
        let api = CliError::Api {
            status: 500,
            message: "internal error".to_string(),
        };
        let connection = CliError::Connection {
            endpoint: "http://localhost:8080".to_string(),
            source: Box::new(std::io::Error::other("connection refused")),
        };

        let codes = [
            render.exit_code(),
            not_implemented.exit_code(),
            api.exit_code(),
            connection.exit_code(),
        ];
        let mut unique = codes.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), codes.len(), "exit codes must be distinct");

        // None of them collide with clap's own usage-error exit code (2),
        // or with success (0).
        assert!(codes.iter().all(|c| *c != 0 && *c != 2));
    }

    #[test]
    fn not_implemented_message_names_the_command() {
        let err = CliError::NotImplemented("cert revoke");
        assert_eq!(err.to_string(), "cert revoke is not yet implemented");
    }
}
