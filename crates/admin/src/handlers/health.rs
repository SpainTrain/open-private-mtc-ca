//! Health/version handlers (spec §17.2 acceptance criterion: "Health/version
//! endpoint implemented end-to-end as the first operation (spec -> stub ->
//! handler)").
//!
//! [`HealthApi`] implements the generated
//! `mtc_admin_api_server::apis::{health::Health, status::Status}` traits
//! (and the `ErrorHandler` supertrait both require) against
//! [`AppState`], and is the adapter [`crate::router`] hands to the
//! generated `server::new`. It is the only place `OpenAPI` wire types
//! (`mtc_admin_api_server::models`) and domain types
//! ([`crate::state`]) meet.
//!
//! `/status`'s `lease` and `checkpoint` fields are real CA-service business
//! state and stay `None` here -- out of this ticket's scope (bd mtc-gja:
//! "Business operations beyond health/version (per-operation tickets)");
//! both are optional in the `OpenAPI` schema, so omitting them is a valid
//! response, not a stub.

use std::time::SystemTime;

use async_trait::async_trait;
use axum::http::Method;
use axum_extra::extract::CookieJar;
use chrono::{DateTime, Utc};
use headers::Host;

use mtc_admin_api_server::apis::health::{GetHealthzResponse, GetReadyzResponse, Health};
use mtc_admin_api_server::apis::status::{GetStatusResponse, Status};
use mtc_admin_api_server::apis::ErrorHandler;
use mtc_admin_api_server::models;

use crate::error::AdminApiError;
use crate::state::AppState;

/// Implements the generated admin-API server traits over [`AppState`] for
/// the health/version surface (`/healthz`, `/readyz`, `/status`).
#[derive(Clone)]
pub struct HealthApi {
    state: AppState,
}

impl HealthApi {
    /// Wraps `state` for use as the generated server's API implementation.
    #[must_use]
    pub const fn new(state: AppState) -> Self {
        Self { state }
    }
}

fn to_wire_time(time: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(time)
}

fn to_wire_check(check: crate::state::DependencyCheck) -> models::ReadinessCheck {
    models::ReadinessCheck {
        name: check.name,
        ok: check.ok,
        detail: check.detail,
    }
}

#[async_trait]
impl Health<AdminApiError> for HealthApi {
    /// Liveness probe (spec §20.5): carries no dependency checks -- if this
    /// handler runs at all, the process is alive.
    async fn get_healthz(
        &self,
        _method: &Method,
        _host: &Host,
        _cookies: &CookieJar,
    ) -> Result<GetHealthzResponse, AdminApiError> {
        Ok(GetHealthzResponse::Status200_ProcessIsAlive(
            models::HealthzResponse {
                status: "ok".to_string(),
            },
        ))
    }

    /// Readiness probe (spec §20.5): 200 when every [`AppState`]-reported
    /// dependency check passes, 503 (same body shape) otherwise.
    async fn get_readyz(
        &self,
        _method: &Method,
        _host: &Host,
        _cookies: &CookieJar,
    ) -> Result<GetReadyzResponse, AdminApiError> {
        let checks = self.state.readiness_checks();
        let ready = checks.iter().all(|check| check.ok);
        let body = models::ReadyzResponse {
            status: if ready { "ready" } else { "not_ready" }.to_string(),
            checks: checks.into_iter().map(to_wire_check).collect(),
        };
        Ok(if ready {
            GetReadyzResponse::Status200_ServiceIsReadyToServe(body)
        } else {
            GetReadyzResponse::Status503_ServiceIsNotReady(body)
        })
    }
}

#[async_trait]
impl Status<AdminApiError> for HealthApi {
    /// Minimal service status (spec §17.3): identity/build info always;
    /// `lease`/`checkpoint` stay `None` (see module docs).
    async fn get_status(
        &self,
        _method: &Method,
        _host: &Host,
        _cookies: &CookieJar,
    ) -> Result<GetStatusResponse, AdminApiError> {
        let identity = self.state.identity();
        let service = models::ServiceInfo {
            name: identity.name,
            version: identity.version,
            region: identity.region,
            started_at: Some(to_wire_time(self.state.started_at())),
        };
        Ok(GetStatusResponse::Status200_CurrentServiceStatus(
            models::StatusResponse {
                service,
                lease: None,
                checkpoint: None,
            },
        ))
    }
}

#[async_trait]
impl ErrorHandler<AdminApiError> for HealthApi {
    /// Renders `AdminApiError` via its `IntoResponse` impl (the shared JSON
    /// error model) instead of the generated default's bare, unlabeled 500.
    async fn handle_error(
        &self,
        _method: &http::Method,
        _host: &headers::Host,
        _cookies: &axum_extra::extract::CookieJar,
        error: AdminApiError,
    ) -> Result<axum::response::Response, http::StatusCode> {
        Ok(axum::response::IntoResponse::into_response(error))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use clock::FakeClock;
    use pretty_assertions::assert_eq;

    use crate::state::{CaStateProvider, DependencyCheck, InMemoryCaState, ServiceIdentity};

    use super::*;

    fn api(ca: InMemoryCaState) -> HealthApi {
        let ca: Arc<dyn CaStateProvider> = Arc::new(ca);
        let clock = Arc::new(FakeClock::new(SystemTime::UNIX_EPOCH));
        HealthApi::new(AppState::new(ca, clock))
    }

    fn method_host_cookies() -> (Method, Host, CookieJar) {
        (
            Method::GET,
            Host::from(http::uri::Authority::from_static("localhost")),
            CookieJar::new(),
        )
    }

    #[tokio::test]
    async fn healthz_is_always_ok() {
        let (method, host, cookies) = method_host_cookies();
        let response = api(InMemoryCaState::default())
            .get_healthz(&method, &host, &cookies)
            .await
            .expect("infallible");
        let GetHealthzResponse::Status200_ProcessIsAlive(body) = response;
        assert_eq!(body.status, "ok");
    }

    #[tokio::test]
    async fn readyz_is_ready_with_no_checks() {
        let (method, host, cookies) = method_host_cookies();
        let response = api(InMemoryCaState::default())
            .get_readyz(&method, &host, &cookies)
            .await
            .expect("infallible");
        match response {
            GetReadyzResponse::Status200_ServiceIsReadyToServe(body) => {
                assert_eq!(body.status, "ready");
                assert!(body.checks.is_empty());
            }
            GetReadyzResponse::Status503_ServiceIsNotReady(_) => panic!("expected ready"),
        }
    }

    #[tokio::test]
    async fn readyz_is_not_ready_when_a_check_fails() {
        let ca = InMemoryCaState::default()
            .with_check(DependencyCheck::ok("storage"))
            .with_check(DependencyCheck::failing("hsm", "unreachable"));
        let (method, host, cookies) = method_host_cookies();
        let response = api(ca)
            .get_readyz(&method, &host, &cookies)
            .await
            .expect("infallible");
        match response {
            GetReadyzResponse::Status200_ServiceIsReadyToServe(_) => panic!("expected not_ready"),
            GetReadyzResponse::Status503_ServiceIsNotReady(body) => {
                assert_eq!(body.status, "not_ready");
                assert_eq!(body.checks.len(), 2);
                assert!(body.checks[0].ok);
                assert!(!body.checks[1].ok);
                assert_eq!(body.checks[1].detail.as_deref(), Some("unreachable"));
            }
        }
    }

    #[tokio::test]
    async fn status_reports_identity_and_omits_lease_and_checkpoint() {
        let identity = ServiceIdentity {
            name: "mtc-ca".to_string(),
            version: "9.9.9".to_string(),
            region: "us-east-1".to_string(),
        };
        let (method, host, cookies) = method_host_cookies();
        let response = api(InMemoryCaState::new(identity))
            .get_status(&method, &host, &cookies)
            .await
            .expect("infallible");
        let GetStatusResponse::Status200_CurrentServiceStatus(body) = response else {
            panic!("expected current service status");
        };
        assert_eq!(body.service.name, "mtc-ca");
        assert_eq!(body.service.version, "9.9.9");
        assert_eq!(body.service.region, "us-east-1");
        assert_eq!(
            body.service.started_at,
            Some(to_wire_time(SystemTime::UNIX_EPOCH))
        );
        assert_eq!(body.lease, None);
        assert_eq!(body.checkpoint, None);
    }
}
