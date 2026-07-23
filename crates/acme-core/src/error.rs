//! Typed ACME error enum (`thiserror`, per `thiserror-for-libs`).
//!
//! Each variant maps onto exactly one ACME problem document
//! ([`crate::problem`]); handlers return `Result<_, AcmeError>` and axum
//! renders the problem via [`IntoResponse`].

use axum::response::{IntoResponse, Response};

use crate::problem::{ProblemDocument, ProblemType};

/// Supported JWS algorithm (the only one this server accepts).
pub const ES256: &str = "ES256";

/// ACME request-processing failures.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum AcmeError {
    /// Missing, unknown, expired, or already-consumed anti-replay nonce.
    #[error("bad anti-replay nonce")]
    BadNonce,
    /// JWS `alg` other than ES256.
    #[error("unsupported JWS signature algorithm {alg:?}")]
    BadSignatureAlgorithm {
        /// The algorithm the client offered.
        alg: String,
    },
    /// Structurally invalid request (body, JWS envelope, header, payload) or
    /// failed signature verification.
    #[error("malformed request: {0}")]
    Malformed(String),
    /// `onlyReturnExisting` was set but no account matches the key.
    #[error("no account exists with the provided key")]
    AccountDoesNotExist,
    /// JWS protected `url` does not match the request URL (RFC 8555 §6.4).
    #[error("JWS url does not match the request URL")]
    UrlMismatch,
    /// The offered JWK cannot be used (kty/crv/point problems).
    #[error("unusable account public key: {0}")]
    BadPublicKey(String),
    /// Server-side failure (e.g. RNG unavailable).
    #[error("internal server error: {0}")]
    Internal(String),
}

impl AcmeError {
    /// The problem document this error renders as.
    #[must_use]
    pub fn problem(&self) -> ProblemDocument {
        match self {
            Self::BadNonce => ProblemDocument::new(ProblemType::BadNonce, self.to_string()),
            Self::BadSignatureAlgorithm { .. } => {
                ProblemDocument::new(ProblemType::BadSignatureAlgorithm, self.to_string())
                    .with_algorithms(vec![ES256])
            }
            Self::Malformed(_) => ProblemDocument::new(ProblemType::Malformed, self.to_string()),
            Self::AccountDoesNotExist => {
                ProblemDocument::new(ProblemType::AccountDoesNotExist, self.to_string())
            }
            Self::UrlMismatch => ProblemDocument::new(ProblemType::Unauthorized, self.to_string()),
            Self::BadPublicKey(_) => {
                ProblemDocument::new(ProblemType::BadPublicKey, self.to_string())
            }
            Self::Internal(_) => {
                // Do not echo internal details to clients.
                ProblemDocument::new(ProblemType::ServerInternal, "internal server error".into())
            }
        }
    }
}

impl IntoResponse for AcmeError {
    fn into_response(self) -> Response {
        self.problem().into_response()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn bad_signature_algorithm_lists_supported_algorithms() {
        let err = AcmeError::BadSignatureAlgorithm {
            alg: "RS256".into(),
        };
        assert_eq!(err.problem().algorithms, Some(vec!["ES256"]));
    }

    #[test]
    fn internal_errors_do_not_leak_detail() {
        let err = AcmeError::Internal("rng exploded at 0xdeadbeef".into());
        let problem = err.problem();
        assert_eq!(problem.detail, "internal server error");
        assert_eq!(problem.status, 500);
    }

    #[test]
    fn url_mismatch_is_unauthorized() {
        // RFC 8555 §6.4: a url mismatch is rejected as unauthorized.
        let problem = AcmeError::UrlMismatch.problem();
        assert_eq!(
            problem.problem_type,
            "urn:ietf:params:acme:error:unauthorized"
        );
        assert_eq!(problem.status, 401);
    }
}
