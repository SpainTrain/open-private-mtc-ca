//! Scripted ACME client demo (ticket `ca-acme-core`).
//!
//! Boots the ACME server on an ephemeral 127.0.0.1 port, then drives it over
//! real TCP with a minimal hand-rolled HTTP/1.1 client:
//!
//! 1. fetch the directory,
//! 2. obtain a nonce (HEAD new-nonce),
//! 3. register an account (ES256 JWS) — expects `201` + `Location`,
//! 4. re-register the same key — expects `200`, same account,
//! 5. replay the consumed nonce — expects `400` `badNonce`.
//!
//! ```console
//! $ cargo run -p acme-core --example demo_client
//! ```
//!
//! Exits non-zero if any expectation fails.

use std::fmt::Write as _;
use std::net::SocketAddr;
use std::sync::Arc;

use acme_core::client::{signed_request_body, ClientBinding};
use acme_core::{router, AcmeState, BaseUrl};
use clock::SystemClock;
use eyre::{ensure, eyre, WrapErr};
use p256::ecdsa::SigningKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// One-connection HTTP/1.1 exchange (`Connection: close`, read to EOF).
async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> eyre::Result<HttpResponse> {
    let mut stream = TcpStream::connect(addr).await.wrap_err("connect")?;
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(body) = body {
        request.push_str("Content-Type: application/jose+json\r\n");
        let _ = write!(request, "Content-Length: {}\r\n", body.len());
    }
    request.push_str("\r\n");
    if let Some(body) = body {
        request.push_str(body);
    }
    stream
        .write_all(request.as_bytes())
        .await
        .wrap_err("write")?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.wrap_err("read")?;
    let raw = String::from_utf8(raw).wrap_err("non-utf8 response")?;
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| eyre!("malformed HTTP response"))?;
    let mut lines = head.lines();
    let status_line = lines.next().ok_or_else(|| eyre!("empty response"))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| eyre!("bad status line {status_line:?}"))?
        .parse()
        .wrap_err("bad status code")?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
        .collect();
    Ok(HttpResponse {
        status,
        headers,
        body: body.to_owned(),
    })
}

fn nonce_of(response: &HttpResponse) -> eyre::Result<String> {
    Ok(response
        .header("replay-nonce")
        .ok_or_else(|| eyre!("missing Replay-Nonce header"))?
        .to_owned())
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    // Boot the server on an ephemeral port.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .wrap_err("bind")?;
    let addr = listener.local_addr().wrap_err("local addr")?;
    let base = format!("http://{addr}");
    let state = AcmeState::new(BaseUrl::new(&*base), Arc::new(SystemClock));
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, router(state)).await {
            eprintln!("server error: {err}");
        }
    });
    println!("[demo] ACME server listening at {base}");

    // 1. Directory.
    let directory = http(addr, "GET", "/acme/directory", None).await?;
    ensure!(directory.status == 200, "directory: {}", directory.status);
    println!(
        "[demo] GET /acme/directory -> 200\n       {}",
        directory.body
    );
    let dir: serde_json::Value = serde_json::from_str(&directory.body)?;
    let new_account_url = dir["newAccount"]
        .as_str()
        .ok_or_else(|| eyre!("directory missing newAccount"))?
        .to_owned();

    // 2. Nonce via HEAD new-nonce.
    let nonce_resp = http(addr, "HEAD", "/acme/new-nonce", None).await?;
    ensure!(nonce_resp.status == 200, "new-nonce: {}", nonce_resp.status);
    let nonce = nonce_of(&nonce_resp)?;
    println!("[demo] HEAD /acme/new-nonce -> 200, Replay-Nonce: {nonce}");

    // 3. Register an account.
    let key = SigningKey::from_slice(&[42; 32]).map_err(|e| eyre!("test key: {e}"))?;
    let body = signed_request_body(
        &key,
        &ClientBinding::Jwk,
        &nonce,
        &new_account_url,
        &serde_json::json!({
            "termsOfServiceAgreed": true,
            "contact": ["mailto:demo@example.com"],
        }),
    )
    .map_err(|e| eyre!("sign: {e}"))?;
    let created = http(addr, "POST", "/acme/new-account", Some(&body)).await?;
    ensure!(
        created.status == 201,
        "expected 201, got {}",
        created.status
    );
    let location = created
        .header("location")
        .ok_or_else(|| eyre!("201 without Location"))?
        .to_owned();
    println!("[demo] POST /acme/new-account -> 201 Created, account: {location}");
    let fresh_nonce = nonce_of(&created)?;

    // 4. Re-register the same key: existing account, 200, same Location.
    let body = signed_request_body(
        &key,
        &ClientBinding::Jwk,
        &fresh_nonce,
        &new_account_url,
        &serde_json::json!({"termsOfServiceAgreed": true}),
    )
    .map_err(|e| eyre!("sign: {e}"))?;
    let existing = http(addr, "POST", "/acme/new-account", Some(&body)).await?;
    ensure!(
        existing.status == 200,
        "expected 200, got {}",
        existing.status
    );
    ensure!(
        existing.header("location") == Some(location.as_str()),
        "re-registration must return the same account"
    );
    println!("[demo] POST /acme/new-account (same key) -> 200 OK, same account");

    // 5. Replay the original (already consumed) nonce: badNonce.
    let body = signed_request_body(
        &key,
        &ClientBinding::Jwk,
        &nonce, // spent in step 3
        &new_account_url,
        &serde_json::json!({"termsOfServiceAgreed": true}),
    )
    .map_err(|e| eyre!("sign: {e}"))?;
    let rejected = http(addr, "POST", "/acme/new-account", Some(&body)).await?;
    ensure!(
        rejected.status == 400,
        "expected 400, got {}",
        rejected.status
    );
    let problem: serde_json::Value = serde_json::from_str(&rejected.body)?;
    ensure!(
        problem["type"] == "urn:ietf:params:acme:error:badNonce",
        "expected badNonce, got {}",
        rejected.body
    );
    println!(
        "[demo] POST /acme/new-account (replayed nonce) -> 400\n       {}",
        rejected.body
    );

    println!("[demo] all expectations met");
    Ok(())
}
