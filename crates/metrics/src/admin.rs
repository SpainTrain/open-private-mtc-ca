//! Minimal admin HTTP endpoint exposing Prometheus text format at `/metrics`.
//!
//! Hand-rolled HTTP/1.1 responder over tokio — deliberately: the admin
//! surface serves exactly one read-only path on a loopback/admin port, and a
//! full HTTP framework is not warranted at this layer. Service crates that
//! already run an HTTP stack can instead mount
//! [`MetricsRegistry::encode_prometheus_text`] on their own router.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use crate::error::MetricsError;
use crate::registry::MetricsRegistry;

/// Conventional default admin port for Prometheus exposition (the IANA/CNCF
/// allocation for OpenTelemetry-style Prometheus exporters).
pub const DEFAULT_ADMIN_PORT: u16 = 9464;

/// Content type of the Prometheus text exposition format (version 0.0.4).
pub const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";
const MAX_REQUEST_BYTES: usize = 8 * 1024;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Handle to a running admin endpoint.
///
/// The listener task runs until [`AdminServer::shutdown`] is awaited (or the
/// process exits).
pub struct AdminServer {
    local_addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl AdminServer {
    /// The bound address (useful with port 0 for tests).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stops accepting connections and waits for the listener task to exit.
    pub async fn shutdown(self) {
        let AdminServer { shutdown, task, .. } = self;
        // Both results are best-effort: the task may already be gone.
        let _ = shutdown.send(());
        let _ = task.await;
    }
}

/// Serves `GET /metrics` (Prometheus text exposition, §20.1) on `addr`.
///
/// Bind to `(Ipv4Addr::LOCALHOST, DEFAULT_ADMIN_PORT)` for the conventional
/// admin port, or port 0 to let the OS choose (tests).
///
/// # Errors
///
/// Returns [`MetricsError::Io`] if the listener cannot bind.
pub async fn serve_admin(
    registry: MetricsRegistry,
    addr: SocketAddr,
) -> Result<AdminServer, MetricsError> {
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _peer)) => {
                        let registry = registry.clone();
                        tokio::spawn(async move {
                            if let Err(error) = handle_connection(stream, &registry).await {
                                tracing::debug!(%error, "admin connection error");
                            }
                        });
                    }
                    Err(error) => tracing::warn!(%error, "admin accept failed"),
                }
            }
        }
    });
    Ok(AdminServer {
        local_addr,
        shutdown: shutdown_tx,
        task,
    })
}

async fn handle_connection(
    mut stream: TcpStream,
    registry: &MetricsRegistry,
) -> std::io::Result<()> {
    let head =
        match tokio::time::timeout(REQUEST_READ_TIMEOUT, read_request_head(&mut stream)).await {
            Ok(head) => head?,
            Err(_elapsed) => return Ok(()), // slow client: drop the connection
        };
    let Some(request_line) = head.lines().next() else {
        return Ok(());
    };
    let (status, content_type, body) = route(request_line, registry);
    write_response(&mut stream, status, content_type, &body).await
}

/// Reads until the end of the request head (blank line), EOF, or the size cap.
async fn read_request_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    while !header_complete(&buf) && buf.len() < MAX_REQUEST_BYTES {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn header_complete(buf: &[u8]) -> bool {
    buf.windows(4).any(|window| window == b"\r\n\r\n")
}

fn route(request_line: &str, registry: &MetricsRegistry) -> (u16, &'static str, String) {
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method != "GET" {
        return (405, TEXT_CONTENT_TYPE, "method not allowed\n".to_string());
    }
    match target.split('?').next().unwrap_or(target) {
        "/metrics" => match registry.encode_prometheus_text() {
            Ok(body) => (200, PROMETHEUS_CONTENT_TYPE, body),
            Err(error) => (
                500,
                TEXT_CONTENT_TYPE,
                format!("failed to encode metrics: {error}\n"),
            ),
        },
        _ => (404, TEXT_CONTENT_TYPE, "not found\n".to_string()),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    }
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len(),
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::CaMetrics;

    async fn start() -> (AdminServer, CaMetrics) {
        let registry = MetricsRegistry::new();
        let metrics = CaMetrics::register(&registry).expect("registration succeeds");
        let addr: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");
        let server = serve_admin(registry, addr).await.expect("binds");
        (server, metrics)
    }

    async fn request(addr: SocketAddr, raw: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.expect("connects");
        stream.write_all(raw.as_bytes()).await.expect("writes");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("reads response");
        response
    }

    #[tokio::test]
    async fn serves_prometheus_text_at_metrics_path() {
        let (server, metrics) = start().await;
        metrics.batches_committed_total.inc();
        metrics.issuance_latency_seconds.observe(0.42);
        let response = request(
            server.local_addr(),
            "GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(
            response.contains("Content-Type: text/plain; version=0.0.4"),
            "{response}"
        );
        assert!(response.contains("batches_committed_total 1"), "{response}");
        assert!(
            response.contains("# TYPE issuance_latency_seconds histogram"),
            "{response}"
        );
        server.shutdown().await;
    }

    #[tokio::test]
    async fn unknown_path_is_404() {
        let (server, _metrics) = start().await;
        let response = request(
            server.local_addr(),
            "GET /nope HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 404 Not Found\r\n"),
            "{response}"
        );
        server.shutdown().await;
    }

    #[tokio::test]
    async fn non_get_method_is_405() {
        let (server, _metrics) = start().await;
        let response = request(
            server.local_addr(),
            "POST /metrics HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
            "{response}"
        );
        server.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_stops_accepting() {
        let (server, _metrics) = start().await;
        let addr = server.local_addr();
        server.shutdown().await;
        assert!(TcpStream::connect(addr).await.is_err());
    }
}
