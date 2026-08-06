//! Integration test: an axum/tower test client driving the mounted router
//! (spec §17.2 acceptance criterion "Integration: axum/tower test client
//! over the mounted router; check that regenerated code compiles in CI").
//!
//! Exercises the generated-stub-to-handler wiring over real HTTP semantics
//! (status codes, JSON bodies, routing) -- complementing the handler-level
//! unit tests in `src/handlers/health.rs` that call the trait methods
//! directly against `AppState`.

// Integration-test helpers (`get`, below) sit outside #[test] fns, so the
// allow-expect-in-tests exemption does not reach them (documented
// scoped-allow pattern, docs/lint-policy.md deviation 1).
#![allow(clippy::expect_used)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use clock::FakeClock;
use pretty_assertions::assert_eq;
use serde_json::Value;
use tower::ServiceExt;

use mtc_admin::{AppState, DependencyCheck, InMemoryCaState, ServiceIdentity};

fn identity() -> ServiceIdentity {
    ServiceIdentity {
        name: "mtc-ca".to_string(),
        version: "0.1.0".to_string(),
        region: "us-east-1".to_string(),
    }
}

fn router_over(ca: InMemoryCaState) -> Router {
    let state = AppState::new(Arc::new(ca), Arc::new(FakeClock::default()));
    mtc_admin::router(state)
}

async fn get(app: Router, path: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(path)
        // The generated handlers extract `TypedHeader<Host>`; unlike a real
        // TCP client, `oneshot` sends exactly the request built here, so the
        // `Host` header must be set explicitly or extraction rejects the
        // request before any handler code runs.
        .header(axum::http::header::HOST, "localhost")
        .body(Body::empty())
        .expect("request builds");
    let response = app.oneshot(request).await.expect("router is infallible");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body reads");
    let body: Value = serde_json::from_slice(&bytes).expect("json body");
    (status, body)
}

#[tokio::test]
async fn healthz_returns_200_ok_status_json() {
    let (status, body) = get(router_over(InMemoryCaState::new(identity())), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!({"status": "ok"}));
}

#[tokio::test]
async fn readyz_returns_200_ready_when_no_checks_fail() {
    let (status, body) = get(router_over(InMemoryCaState::new(identity())), "/readyz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!({"status": "ready", "checks": []}));
}

#[tokio::test]
async fn readyz_returns_503_not_ready_when_a_check_fails() {
    let ca = InMemoryCaState::new(identity())
        .with_check(DependencyCheck::ok("storage"))
        .with_check(DependencyCheck::failing("hsm", "unreachable"));
    let (status, body) = get(router_over(ca), "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body,
        serde_json::json!({
            "status": "not_ready",
            "checks": [
                {"name": "storage", "ok": true},
                {"name": "hsm", "ok": false, "detail": "unreachable"},
            ],
        })
    );
}

#[tokio::test]
async fn status_returns_200_with_identity_and_no_lease_or_checkpoint() {
    let (status, body) = get(router_over(InMemoryCaState::new(identity())), "/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["service"]["name"], "mtc-ca");
    assert_eq!(body["service"]["version"], "0.1.0");
    assert_eq!(body["service"]["region"], "us-east-1");
    assert!(body["service"]["started_at"].is_string());
    // Optional, business-state fields are genuinely absent (not `null`) --
    // out of scope for this ticket (bd mtc-gja).
    assert!(body.get("lease").is_none());
    assert!(body.get("checkpoint").is_none());
}

#[tokio::test]
async fn unknown_path_is_404() {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/does-not-exist")
        .header(axum::http::header::HOST, "localhost")
        .body(Body::empty())
        .expect("request builds");
    let response = router_over(InMemoryCaState::new(identity()))
        .oneshot(request)
        .await
        .expect("router is infallible");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
