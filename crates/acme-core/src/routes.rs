//! Axum routes: directory (RFC 8555 §7.1.1), new-nonce (§7.2), new-account
//! (§7.3).
//!
//! Only implemented resources appear in the directory (§7.1.1 requires URLs
//! for implemented resources only); orders/finalize land with the
//! `ca-acme-orders` / `ca-acme-issuance` tickets. Every response — success
//! or problem — carries a fresh `Replay-Nonce` header, added by a middleware
//! layer wrapping all routes.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::account::{Account, AccountStore, NewAccountRequest};
use crate::clock::Clock;
use crate::error::AcmeError;
use crate::jws::{AccountKeySource, Jws};
use crate::nonce::{Nonce, NonceStore, DEFAULT_NONCE_TTL_MILLIS};

/// Directory resource path.
pub const DIRECTORY_PATH: &str = "/acme/directory";
/// New-nonce resource path.
pub const NEW_NONCE_PATH: &str = "/acme/new-nonce";
/// New-account resource path.
pub const NEW_ACCOUNT_PATH: &str = "/acme/new-account";
/// Prefix for account URLs (the `kid` namespace).
pub const ACCOUNT_PATH_PREFIX: &str = "/acme/acct/";

/// Required media type for ACME POST bodies (RFC 8555 §6.2).
pub const JOSE_JSON: &str = "application/jose+json";

/// The `Replay-Nonce` header name (RFC 8555 §6.5.1).
#[must_use]
pub fn replay_nonce_header() -> HeaderName {
    HeaderName::from_static("replay-nonce")
}

/// The server's externally visible base URL (newtype per `use-newtypes`).
///
/// JWS `url` protected-header checks compare against
/// `base_url + route path`, so this must match what clients dial.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BaseUrl(String);

impl BaseUrl {
    /// Creates a base URL, stripping any trailing slashes.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        let mut url = url.into();
        while url.ends_with('/') {
            url.pop();
        }
        Self(url)
    }

    /// Joins an absolute path onto the base.
    #[must_use]
    pub fn join(&self, path: &str) -> String {
        format!("{}{path}", self.0)
    }

    /// The base URL string (no trailing slash).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

struct StateInner {
    base_url: BaseUrl,
    clock: Arc<dyn Clock>,
    nonces: Mutex<NonceStore>,
    accounts: Mutex<AccountStore>,
}

/// Shared ACME server state: nonce registry, account store, clock, base URL.
#[derive(Clone)]
pub struct AcmeState {
    inner: Arc<StateInner>,
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock means a handler panicked mid-update; both stores are
    // valid at every await/return point, so continuing with the inner value
    // is safe and avoids wedging the whole server on one panic.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AcmeState {
    /// Creates state with the default nonce TTL.
    #[must_use]
    pub fn new(base_url: BaseUrl, clock: Arc<dyn Clock>) -> Self {
        Self::with_nonce_ttl(base_url, clock, DEFAULT_NONCE_TTL_MILLIS)
    }

    /// Creates state with an explicit nonce TTL (tests).
    #[must_use]
    pub fn with_nonce_ttl(base_url: BaseUrl, clock: Arc<dyn Clock>, ttl_millis: u64) -> Self {
        Self {
            inner: Arc::new(StateInner {
                base_url,
                clock,
                nonces: Mutex::new(NonceStore::new(ttl_millis)),
                accounts: Mutex::new(AccountStore::new()),
            }),
        }
    }

    /// Absolute URL for a route path.
    #[must_use]
    pub fn url_for(&self, path: &str) -> String {
        self.inner.base_url.join(path)
    }

    fn fresh_nonce(&self) -> Result<Nonce, AcmeError> {
        lock_unpoisoned(&self.inner.nonces).issue(self.inner.clock.as_ref())
    }

    fn consume_nonce(&self, nonce: &Nonce) -> Result<(), AcmeError> {
        lock_unpoisoned(&self.inner.nonces).consume(nonce, self.inner.clock.as_ref())
    }

    fn with_accounts<R>(&self, f: impl FnOnce(&mut AccountStore) -> R) -> R {
        f(&mut lock_unpoisoned(&self.inner.accounts))
    }
}

/// Builds the ACME router. All routes are wrapped in the Replay-Nonce layer.
pub fn router(state: AcmeState) -> Router {
    Router::new()
        .route(DIRECTORY_PATH, get(directory))
        .route(NEW_NONCE_PATH, get(new_nonce))
        .route(NEW_ACCOUNT_PATH, post(new_account))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            attach_replay_nonce,
        ))
        .with_state(state)
}

/// Middleware: stamp a fresh Replay-Nonce onto every response (RFC 8555
/// §6.5: every successful POST response MUST carry one; error responses
/// SHOULD — we stamp all `/acme/*` responses).
async fn attach_replay_nonce(
    State(state): State<AcmeState>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    match state.fresh_nonce() {
        Ok(nonce) => {
            if let Ok(value) = HeaderValue::from_str(nonce.as_str()) {
                response.headers_mut().insert(replay_nonce_header(), value);
            }
            response
        }
        // RNG failure: the response would violate the §6.5 MUST; surface a
        // server error instead of a nonce-less success.
        Err(err) => err.into_response(),
    }
}

/// GET directory (RFC 8555 §7.1.1). Lists only implemented resources.
async fn directory(State(state): State<AcmeState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "newNonce": state.url_for(NEW_NONCE_PATH),
        "newAccount": state.url_for(NEW_ACCOUNT_PATH),
        "meta": {
            "website": "https://github.com/SpainTrain/open-private-mtc-ca",
        },
    }))
}

/// HEAD/GET new-nonce (RFC 8555 §7.2): HEAD -> 200, GET -> 204; the nonce
/// itself is attached by the middleware.
async fn new_nonce(method: Method) -> Response {
    let status = if method == Method::HEAD {
        StatusCode::OK
    } else {
        StatusCode::NO_CONTENT
    };
    (
        status,
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
    )
        .into_response()
}

/// Account object returned to clients (RFC 8555 §7.1.2 subset; `orders` is
/// out of scope until the orders ticket lands).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountResource {
    status: crate::account::AccountStatus,
    contact: Vec<String>,
    terms_of_service_agreed: bool,
}

impl From<&Account> for AccountResource {
    fn from(account: &Account) -> Self {
        Self {
            status: account.status,
            contact: account.contact.clone(),
            terms_of_service_agreed: account.terms_of_service_agreed,
        }
    }
}

/// POST new-account (RFC 8555 §7.3).
async fn new_account(
    State(state): State<AcmeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AcmeError> {
    require_jose_json(&headers)?;
    let jws = Jws::parse(&body)?;
    jws.check_alg()?;
    // Consume the nonce before any other semantic check so a replayed
    // request always reads as badNonce (and burns nothing else).
    let nonce = jws.nonce()?;
    state.consume_nonce(&nonce)?;
    jws.check_url(&state.url_for(NEW_ACCOUNT_PATH))?;

    // new-account requests must self-sign with the inline account key
    // (RFC 8555 §7.3.1: jwk, not kid).
    let jwk = match jws.account_key()? {
        AccountKeySource::Jwk(jwk) => jwk.clone(),
        AccountKeySource::Kid(_) => {
            return Err(AcmeError::Malformed(
                "new-account requires \"jwk\", not \"kid\"".into(),
            ))
        }
    };
    let verifying_key = jwk.verifying_key()?;
    jws.verify_signature(&verifying_key)?;

    let request: NewAccountRequest = serde_json::from_slice(jws.payload())
        .map_err(|e| AcmeError::Malformed(format!("invalid new-account payload: {e}")))?;

    let thumbprint = jwk.thumbprint();
    let (account, created) = state.with_accounts(|accounts| {
        if let Some(existing) = accounts.find_by_thumbprint(&thumbprint) {
            return Ok((existing.clone(), false));
        }
        if request.only_return_existing {
            return Err(AcmeError::AccountDoesNotExist);
        }
        Ok(accounts.get_or_create(jwk.clone(), &request))
    })?;

    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let location = state.url_for(&format!("{ACCOUNT_PATH_PREFIX}{}", account.id));
    let location = HeaderValue::from_str(&location)
        .map_err(|_| AcmeError::Internal("account URL is not a valid header value".into()))?;
    let mut response = (status, Json(AccountResource::from(&account))).into_response();
    response.headers_mut().insert(header::LOCATION, location);
    Ok(response)
}

fn require_jose_json(headers: &HeaderMap) -> Result<(), AcmeError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let media_type = content_type.split(';').next().unwrap_or("").trim();
    if media_type.eq_ignore_ascii_case(JOSE_JSON) {
        Ok(())
    } else {
        Err(AcmeError::Malformed(format!(
            "content-type must be {JOSE_JSON}, got {content_type:?}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use base64ct::Encoding as _;
    use http_body_util::BodyExt;
    use p256::ecdsa::SigningKey;
    use pretty_assertions::assert_eq;
    use tower::ServiceExt;

    use super::*;
    use crate::client::{signed_request_body, ClientBinding};
    use crate::clock::ManualClock;

    const BASE: &str = "http://localhost";

    fn test_state() -> AcmeState {
        AcmeState::new(BaseUrl::new(BASE), Arc::new(ManualClock::new()))
    }

    fn test_key(seed: u8) -> SigningKey {
        SigningKey::from_slice(&[seed; 32]).expect("valid scalar")
    }

    async fn send(
        app: &Router,
        request: axum::http::Request<axum::body::Body>,
    ) -> (StatusCode, HeaderMap, serde_json::Value) {
        let response = app.clone().oneshot(request).await.expect("infallible");
        let (parts, body) = response.into_parts();
        let bytes = body.collect().await.expect("body").to_bytes();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("json body")
        };
        (parts.status, parts.headers, json)
    }

    async fn fetch_nonce(app: &Router) -> String {
        let request = axum::http::Request::head(NEW_NONCE_PATH)
            .body(axum::body::Body::empty())
            .expect("request");
        let (status, headers, _) = send(app, request).await;
        assert_eq!(status, StatusCode::OK);
        header_str(&headers, &replay_nonce_header())
    }

    fn header_str(headers: &HeaderMap, name: &HeaderName) -> String {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .expect("header present")
            .to_owned()
    }

    fn post_new_account(body: String) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::post(NEW_ACCOUNT_PATH)
            .header(header::CONTENT_TYPE, JOSE_JSON)
            .body(axum::body::Body::from(body))
            .expect("request")
    }

    fn new_account_body(key: &SigningKey, nonce: &str) -> String {
        signed_request_body(
            key,
            &ClientBinding::Jwk,
            nonce,
            &format!("{BASE}{NEW_ACCOUNT_PATH}"),
            &serde_json::json!({"termsOfServiceAgreed": true, "contact": ["mailto:a@example.com"]}),
        )
        .expect("signable")
    }

    #[tokio::test]
    async fn directory_lists_only_implemented_resources() {
        let app = router(test_state());
        let request = axum::http::Request::get(DIRECTORY_PATH)
            .body(axum::body::Body::empty())
            .expect("request");
        let (status, headers, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["newNonce"], format!("{BASE}{NEW_NONCE_PATH}"));
        assert_eq!(body["newAccount"], format!("{BASE}{NEW_ACCOUNT_PATH}"));
        // Orders/finalize are separate tickets; the directory must not
        // advertise unimplemented resources (§7.1.1).
        assert!(body.get("newOrder").is_none());
        assert!(body.get("revokeCert").is_none());
        // Even the directory response carries a fresh nonce.
        assert!(headers.contains_key(replay_nonce_header()));
    }

    #[tokio::test]
    async fn new_nonce_head_200_get_204_no_store() {
        let app = router(test_state());
        let head = axum::http::Request::head(NEW_NONCE_PATH)
            .body(axum::body::Body::empty())
            .expect("request");
        let (status, headers, _) = send(&app, head).await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers.contains_key(replay_nonce_header()));
        assert_eq!(header_str(&headers, &header::CACHE_CONTROL), "no-store");

        let get = axum::http::Request::get(NEW_NONCE_PATH)
            .body(axum::body::Body::empty())
            .expect("request");
        let (status, headers, _) = send(&app, get).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(headers.contains_key(replay_nonce_header()));
    }

    #[tokio::test]
    async fn nonces_are_fresh_on_every_response() {
        let app = router(test_state());
        let a = fetch_nonce(&app).await;
        let b = fetch_nonce(&app).await;
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn register_then_reregister_returns_same_account() {
        let app = router(test_state());
        let key = test_key(1);

        let nonce = fetch_nonce(&app).await;
        let (status, headers, body) =
            send(&app, post_new_account(new_account_body(&key, &nonce))).await;
        assert_eq!(status, StatusCode::CREATED);
        let location = header_str(&headers, &header::LOCATION);
        assert!(location.starts_with(&format!("{BASE}{ACCOUNT_PATH_PREFIX}")));
        assert_eq!(body["status"], "valid");
        assert_eq!(body["termsOfServiceAgreed"], true);

        let nonce = fetch_nonce(&app).await;
        let (status, headers, _) =
            send(&app, post_new_account(new_account_body(&key, &nonce))).await;
        assert_eq!(status, StatusCode::OK); // existing account, not 201
        assert_eq!(header_str(&headers, &header::LOCATION), location);
    }

    #[tokio::test]
    async fn replayed_nonce_is_rejected_with_bad_nonce() {
        let app = router(test_state());
        let key = test_key(1);
        let nonce = fetch_nonce(&app).await;

        let (status, _, _) = send(&app, post_new_account(new_account_body(&key, &nonce))).await;
        assert_eq!(status, StatusCode::CREATED);

        // Same nonce again: single-use.
        let (status, headers, body) =
            send(&app, post_new_account(new_account_body(&key, &nonce))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["type"], "urn:ietf:params:acme:error:badNonce");
        // Error responses still hand the client a fresh nonce to retry with.
        assert!(headers.contains_key(replay_nonce_header()));
    }

    #[tokio::test]
    async fn forged_nonce_is_rejected() {
        let app = router(test_state());
        let key = test_key(1);
        let (status, _, body) = send(
            &app,
            post_new_account(new_account_body(&key, "AAAAAAAAAAAAAAAAAAAAAA")),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["type"], "urn:ietf:params:acme:error:badNonce");
    }

    #[tokio::test]
    async fn expired_nonce_is_rejected() {
        let clock = Arc::new(ManualClock::new());
        let state =
            AcmeState::with_nonce_ttl(BaseUrl::new(BASE), Arc::<ManualClock>::clone(&clock), 1_000);
        let app = router(state);
        let key = test_key(1);
        let nonce = fetch_nonce(&app).await;
        clock.advance(1_001);
        let (status, _, body) = send(&app, post_new_account(new_account_body(&key, &nonce))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["type"], "urn:ietf:params:acme:error:badNonce");
    }

    #[tokio::test]
    async fn wrong_alg_returns_bad_signature_algorithm() {
        let app = router(test_state());
        let nonce = fetch_nonce(&app).await;
        // Hand-build an RS256-flagged JWS (signature bytes irrelevant: alg is
        // checked first).
        let protected = base64ct::Base64UrlUnpadded::encode_string(
            serde_json::json!({
                "alg": "RS256",
                "nonce": nonce,
                "url": format!("{BASE}{NEW_ACCOUNT_PATH}"),
                "jwk": crate::jws::Jwk {
                    kty: "EC".into(), crv: "P-256".into(), x: "AA".into(), y: "AA".into(),
                },
            })
            .to_string()
            .as_bytes(),
        );
        let body = serde_json::json!({
            "protected": protected,
            "payload": "",
            "signature": "",
        })
        .to_string();
        let (status, _, body) = send(&app, post_new_account(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["type"],
            "urn:ietf:params:acme:error:badSignatureAlgorithm"
        );
        assert_eq!(body["algorithms"], serde_json::json!(["ES256"]));
    }

    #[tokio::test]
    async fn url_mismatch_returns_unauthorized() {
        let app = router(test_state());
        let key = test_key(1);
        let nonce = fetch_nonce(&app).await;
        let body = signed_request_body(
            &key,
            &ClientBinding::Jwk,
            &nonce,
            "http://evil.example/acme/new-account", // wrong URL
            &serde_json::json!({"termsOfServiceAgreed": true}),
        )
        .expect("signable");
        let (status, _, body) = send(&app, post_new_account(body)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["type"], "urn:ietf:params:acme:error:unauthorized");
    }

    #[tokio::test]
    async fn kid_on_new_account_is_malformed() {
        let app = router(test_state());
        let key = test_key(1);
        let nonce = fetch_nonce(&app).await;
        let body = signed_request_body(
            &key,
            &ClientBinding::Kid(format!("{BASE}{ACCOUNT_PATH_PREFIX}1")),
            &nonce,
            &format!("{BASE}{NEW_ACCOUNT_PATH}"),
            &serde_json::json!({"termsOfServiceAgreed": true}),
        )
        .expect("signable");
        let (status, _, body) = send(&app, post_new_account(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["type"], "urn:ietf:params:acme:error:malformed");
    }

    #[tokio::test]
    async fn tampered_signature_is_malformed() {
        let app = router(test_state());
        let key = test_key(1);
        let nonce = fetch_nonce(&app).await;
        let mut parsed: serde_json::Value =
            serde_json::from_str(&new_account_body(&key, &nonce)).expect("json");
        // Flip the signature to a valid-length but wrong value.
        parsed["signature"] =
            serde_json::Value::String(base64ct::Base64UrlUnpadded::encode_string(&[0u8; 64]));
        let (status, _, body) = send(&app, post_new_account(parsed.to_string())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["type"], "urn:ietf:params:acme:error:malformed");
    }

    #[tokio::test]
    async fn garbage_body_is_malformed() {
        let app = router(test_state());
        let (status, headers, body) =
            send(&app, post_new_account("this is not a jws".into())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["type"], "urn:ietf:params:acme:error:malformed");
        assert!(headers.contains_key(replay_nonce_header()));
    }

    #[tokio::test]
    async fn wrong_content_type_is_malformed() {
        let app = router(test_state());
        let key = test_key(1);
        let nonce = fetch_nonce(&app).await;
        let request = axum::http::Request::post(NEW_ACCOUNT_PATH)
            .header(header::CONTENT_TYPE, "application/json") // not jose+json
            .body(axum::body::Body::from(new_account_body(&key, &nonce)))
            .expect("request");
        let (status, _, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["type"], "urn:ietf:params:acme:error:malformed");
    }

    #[tokio::test]
    async fn only_return_existing_unknown_key_is_account_does_not_exist() {
        let app = router(test_state());
        let key = test_key(5); // never registered
        let nonce = fetch_nonce(&app).await;
        let body = signed_request_body(
            &key,
            &ClientBinding::Jwk,
            &nonce,
            &format!("{BASE}{NEW_ACCOUNT_PATH}"),
            &serde_json::json!({"onlyReturnExisting": true}),
        )
        .expect("signable");
        let (status, _, body) = send(&app, post_new_account(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["type"],
            "urn:ietf:params:acme:error:accountDoesNotExist"
        );
    }

    #[tokio::test]
    async fn base_url_strips_trailing_slash() {
        let base = BaseUrl::new("http://localhost:1234///");
        assert_eq!(
            base.join("/acme/directory"),
            "http://localhost:1234/acme/directory"
        );
    }
}
