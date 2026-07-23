//! Demo ACME server binary.
//!
//! Serves the RFC 8555 core surface (directory, new-nonce, new-account) on
//! `127.0.0.1` for the ticket demo:
//!
//! ```console
//! $ cargo run -p acme-core
//! $ curl http://127.0.0.1:4402/acme/directory
//! ```
//!
//! Port is overridable via `ACME_PORT` (0 picks an ephemeral port). The
//! scripted client demo lives in `examples/demo_client.rs`.

use std::sync::Arc;

use acme_core::{router, AcmeState, BaseUrl};
use clock::SystemClock;
use eyre::WrapErr;

const DEFAULT_PORT: u16 = 4402;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let port = match std::env::var("ACME_PORT") {
        Ok(raw) => raw
            .parse::<u16>()
            .wrap_err_with(|| format!("ACME_PORT must be a port number, got {raw:?}"))?,
        Err(_) => DEFAULT_PORT,
    };

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .wrap_err_with(|| format!("failed to bind 127.0.0.1:{port}"))?;
    let addr = listener.local_addr().wrap_err("no local address")?;

    let state = AcmeState::new(
        BaseUrl::new(format!("http://{addr}")),
        Arc::new(SystemClock),
    );

    println!("ACME directory: http://{addr}/acme/directory");
    axum::serve(listener, router(state))
        .await
        .wrap_err("server error")
}
