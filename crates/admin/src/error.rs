//! Shared JSON error model for the admin API (spec §17.2 acceptance
//! criterion: "Shared JSON error model (problem-details style) with typed
//! handler errors").
//!
//! [`AdminApiError`] is a small `thiserror` enum (rule
//! `thiserror-for-libs-eyre-for-bins`) that renders as the `OpenAPI`-generated
//! [`ErrorResponse`] envelope (`{code, message}`, the `/status` operation's
//! `default` response schema in `api/admin.openapi.yaml`) plus an HTTP
//! status. It is the `E` type parameter every generated `apis::*` trait
//! carries for this crate's handlers (see
//! [`crate::handlers::health::HealthApi`]): axum's generated dispatch code
//! calls `ErrorHandler::handle_error` on any `Err(e)`, which
//! `HealthApi` overrides to render this type via [`IntoResponse`] instead of
//! the generated default (an unlabeled bare 500).
//!
//! This ticket's own handlers (`/healthz`, `/readyz`, `/status`) are
//! infallible, so no call site constructs [`AdminApiError`] today; the type
//! exists so later, fallible business-operation handlers (spec §17.5) have
//! a typed error channel and a working `ErrorHandler` override to plug into
//! from day one, rather than every future ticket re-deriving this plumbing.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use mtc_admin_api_server::models::ErrorResponse;

/// Typed handler errors for the admin API.
///
/// Kept exhaustive (no `#[non_exhaustive]`): adding a variant is a
/// conscious decision every handler and `match` must account for, mirroring
/// `cloud_types::CloudError`'s stated default (spec §22.3, "exhaustive
/// matching is the language default and we keep it").
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdminApiError {
    /// An unexpected, unclassified failure. Renders as HTTP 500 with a
    /// generic message -- the detail is logged, not echoed to callers.
    #[error("internal error: {0}")]
    Internal(String),
}

impl AdminApiError {
    /// The stable, machine-readable error code (the `ErrorResponse.code`
    /// wire field).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Internal(_) => "internal_error",
        }
    }

    /// The HTTP status this error renders as.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AdminApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ErrorResponse::new(self.code().to_string(), self.to_string());
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn internal_error_renders_as_500_with_code_and_message() {
        let err = AdminApiError::Internal("boom".to_string());
        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.code(), "internal_error");
        assert_eq!(err.to_string(), "internal error: boom");
    }

    #[tokio::test]
    async fn into_response_carries_the_error_response_json_shape() {
        let err = AdminApiError::Internal("boom".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
        assert_eq!(json["code"], "internal_error");
        assert_eq!(json["message"], "internal error: boom");
    }

    #[test]
    fn error_trait_is_implemented() {
        // thiserror derives std::error::Error; keep it that way so callers
        // can box/chain this through generic error-handling layers.
        fn assert_error<E: std::error::Error + Send + Sync + 'static>(_: &E) {}
        assert_error(&AdminApiError::Internal("x".to_string()));
    }
}
