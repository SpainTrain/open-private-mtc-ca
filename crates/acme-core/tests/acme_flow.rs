//! Integration test: an in-process ACME client walks the RFC 8555 core
//! flow — fetch the directory, obtain a nonce, register an account, and
//! demonstrate single-use nonce semantics.

// Integration-test helpers sit outside #[test] fns, so the
// allow-expect-in-tests exemption does not reach them (documented
// scoped-allow pattern, docs/lint-policy.md deviation 1).
#![allow(clippy::expect_used)]

use std::sync::Arc;

use acme_core::client::{signed_request_body, ClientBinding};
use acme_core::{
    router, AcmeState, BaseUrl, ManualClock, DIRECTORY_PATH, NEW_ACCOUNT_PATH, NEW_NONCE_PATH,
};
use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use p256::ecdsa::SigningKey;
use tower::ServiceExt;

const BASE: &str = "http://localhost";

struct ApiResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: serde_json::Value,
}

async fn send(app: &Router, request: Request<Body>) -> ApiResponse {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("router is infallible");
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.expect("body").to_bytes();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    };
    ApiResponse {
        status: parts.status,
        headers: parts.headers,
        body,
    }
}

fn replay_nonce(headers: &HeaderMap) -> String {
    headers
        .get("replay-nonce")
        .and_then(|v| v.to_str().ok())
        .expect("Replay-Nonce header present")
        .to_owned()
}

async fn get_nonce(app: &Router, path: &str) -> String {
    let response = send(
        app,
        Request::head(path).body(Body::empty()).expect("request"),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    replay_nonce(&response.headers)
}

fn post_jws(url_path: &str, body: String) -> Request<Body> {
    Request::post(url_path)
        .header(header::CONTENT_TYPE, "application/jose+json")
        .body(Body::from(body))
        .expect("request")
}

#[tokio::test]
async fn full_acme_core_flow() {
    let app = router(AcmeState::new(
        BaseUrl::new(BASE),
        Arc::new(ManualClock::new()),
    ));
    let account_key = SigningKey::from_slice(&[42; 32]).expect("valid scalar");

    // 1. Directory (§7.1.1): discover the endpoints; carries a nonce too.
    let directory = send(
        &app,
        Request::get(DIRECTORY_PATH)
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(directory.status, StatusCode::OK);
    let new_nonce_url = directory.body["newNonce"].as_str().expect("newNonce");
    let new_account_url = directory.body["newAccount"].as_str().expect("newAccount");
    assert_eq!(new_nonce_url, format!("{BASE}{NEW_NONCE_PATH}"));
    assert_eq!(new_account_url, format!("{BASE}{NEW_ACCOUNT_PATH}"));
    assert!(directory.headers.contains_key("replay-nonce"));

    // 2. Obtain a nonce (§7.2) from the URL the directory advertised.
    let nonce_path = new_nonce_url.strip_prefix(BASE).expect("same origin");
    let nonce = get_nonce(&app, nonce_path).await;

    // 3. Register an account (§7.3) with an ES256 JWS.
    let body = signed_request_body(
        &account_key,
        &ClientBinding::Jwk,
        &nonce,
        new_account_url,
        &serde_json::json!({
            "termsOfServiceAgreed": true,
            "contact": ["mailto:ops@example.com"],
        }),
    )
    .expect("signable");
    let account_path = new_account_url.strip_prefix(BASE).expect("same origin");
    let created = send(&app, post_jws(account_path, body)).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let location = created
        .headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("Location header")
        .to_owned();
    assert!(location.starts_with(&format!("{BASE}/acme/acct/")));
    assert_eq!(created.body["status"], "valid");
    let post_register_nonce = replay_nonce(&created.headers);
    assert_ne!(
        post_register_nonce, nonce,
        "every response gets a fresh nonce"
    );

    // 4. Replay the consumed nonce: rejected with badNonce (§6.5).
    let replayed = signed_request_body(
        &account_key,
        &ClientBinding::Jwk,
        &nonce, // already spent in step 3
        new_account_url,
        &serde_json::json!({"termsOfServiceAgreed": true}),
    )
    .expect("signable");
    let rejected = send(&app, post_jws(account_path, replayed)).await;
    assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
    assert_eq!(rejected.body["type"], "urn:ietf:params:acme:error:badNonce");
    assert!(
        rejected.headers.contains_key("replay-nonce"),
        "badNonce responses still supply a retry nonce"
    );

    // 5. Retry with the fresh nonce from the 201 response: the key is
    // already registered, so the existing account comes back (200, same
    // Location).
    let retry = signed_request_body(
        &account_key,
        &ClientBinding::Jwk,
        &post_register_nonce,
        new_account_url,
        &serde_json::json!({"termsOfServiceAgreed": true}),
    )
    .expect("signable");
    let existing = send(&app, post_jws(account_path, retry)).await;
    assert_eq!(existing.status, StatusCode::OK);
    assert_eq!(
        existing
            .headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some(location.as_str())
    );

    // 6. A different key with onlyReturnExisting finds nothing.
    let other_key = SigningKey::from_slice(&[43; 32]).expect("valid scalar");
    let nonce = get_nonce(&app, nonce_path).await;
    let probe = signed_request_body(
        &other_key,
        &ClientBinding::Jwk,
        &nonce,
        new_account_url,
        &serde_json::json!({"onlyReturnExisting": true}),
    )
    .expect("signable");
    let missing = send(&app, post_jws(account_path, probe)).await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        missing.body["type"],
        "urn:ietf:params:acme:error:accountDoesNotExist"
    );
}
