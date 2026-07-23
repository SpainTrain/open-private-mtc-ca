//! ACME problem documents (RFC 8555 §6.7, RFC 7807).
//!
//! Every ACME-level failure is rendered as an `application/problem+json`
//! body whose `type` is a `urn:ietf:params:acme:error:*` URN.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// The ACME error URNs this server emits (RFC 8555 §6.7 registry subset).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ProblemType {
    /// The client sent an unacceptable anti-replay nonce.
    BadNonce,
    /// The JWS was signed with an algorithm the server does not support.
    BadSignatureAlgorithm,
    /// The request message was malformed.
    Malformed,
    /// No account exists with the provided key (`onlyReturnExisting`).
    AccountDoesNotExist,
    /// The client lacks sufficient authorization (e.g. `url` mismatch).
    Unauthorized,
    /// The JWK is unusable (wrong key type, invalid curve point, ...).
    BadPublicKey,
    /// The server experienced an internal error.
    ServerInternal,
}

impl ProblemType {
    /// The registered `urn:ietf:params:acme:error:*` identifier.
    #[must_use]
    pub const fn urn(self) -> &'static str {
        match self {
            Self::BadNonce => "urn:ietf:params:acme:error:badNonce",
            Self::BadSignatureAlgorithm => "urn:ietf:params:acme:error:badSignatureAlgorithm",
            Self::Malformed => "urn:ietf:params:acme:error:malformed",
            Self::AccountDoesNotExist => "urn:ietf:params:acme:error:accountDoesNotExist",
            Self::Unauthorized => "urn:ietf:params:acme:error:unauthorized",
            Self::BadPublicKey => "urn:ietf:params:acme:error:badPublicKey",
            Self::ServerInternal => "urn:ietf:params:acme:error:serverInternal",
        }
    }

    /// Default HTTP status for this problem type.
    #[must_use]
    pub const fn status(self) -> StatusCode {
        match self {
            Self::BadNonce
            | Self::BadSignatureAlgorithm
            | Self::Malformed
            | Self::AccountDoesNotExist
            | Self::BadPublicKey => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::ServerInternal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// An RFC 7807 problem document specialized for ACME.
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct ProblemDocument {
    /// ACME error URN.
    #[serde(rename = "type")]
    pub problem_type: &'static str,
    /// Human-readable explanation.
    pub detail: String,
    /// HTTP status code, duplicated into the body per RFC 7807.
    pub status: u16,
    /// For `badSignatureAlgorithm`: the algorithms the server supports
    /// (RFC 8555 §6.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub algorithms: Option<Vec<&'static str>>,
}

impl ProblemDocument {
    /// Builds a problem document with the type's default status.
    #[must_use]
    pub const fn new(problem_type: ProblemType, detail: String) -> Self {
        Self {
            problem_type: problem_type.urn(),
            detail,
            status: problem_type.status().as_u16(),
            algorithms: None,
        }
    }

    /// Attaches the supported-algorithms list (`badSignatureAlgorithm`).
    #[must_use]
    pub fn with_algorithms(mut self, algorithms: Vec<&'static str>) -> Self {
        self.algorithms = Some(algorithms);
        self
    }
}

impl IntoResponse for ProblemDocument {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_string(&self).unwrap_or_else(|_| String::from("{}"));
        (
            status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            body,
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn urns_are_registered_acme_errors() {
        assert_eq!(
            ProblemType::BadNonce.urn(),
            "urn:ietf:params:acme:error:badNonce"
        );
        assert_eq!(
            ProblemType::BadSignatureAlgorithm.urn(),
            "urn:ietf:params:acme:error:badSignatureAlgorithm"
        );
        assert_eq!(
            ProblemType::Malformed.urn(),
            "urn:ietf:params:acme:error:malformed"
        );
    }

    #[test]
    fn serializes_type_detail_and_status() {
        let doc = ProblemDocument::new(ProblemType::BadNonce, "nonce reused".into());
        let value = serde_json::to_value(&doc).expect("serializable");
        assert_eq!(
            value,
            serde_json::json!({
                "type": "urn:ietf:params:acme:error:badNonce",
                "detail": "nonce reused",
                "status": 400,
            })
        );
    }

    #[test]
    fn algorithms_serialized_when_present() {
        let doc = ProblemDocument::new(ProblemType::BadSignatureAlgorithm, "RS256".into())
            .with_algorithms(vec!["ES256"]);
        let value = serde_json::to_value(&doc).expect("serializable");
        assert_eq!(value["algorithms"], serde_json::json!(["ES256"]));
    }
}
