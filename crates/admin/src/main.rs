//! Standalone dev server for the admin API core (spec §17.2, §18.1 "the
//! 60-second demo").
//!
//! `crates/ca-service` does not exist yet as of ticket mtc-gja (built
//! concurrently by another agent in a separate worktree) -- this binary lets
//! `crates/admin` run and be curled on its own during development:
//!
//! ```text
//! cargo run -p mtc-admin
//! curl localhost:8080/healthz
//! ```
//!
//! See the `// TODO(bead)` in `lib.rs`: once `crates/ca-service` exists,
//! [`mtc_admin::router`] mounts into its Fargate-task binary instead (spec
//! §17.4); this standalone entry point becomes dev-only fallback (or is
//! retired) at that point.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use clock::SystemClock;
use mtc_admin::{AppState, InMemoryCaState};

/// Local dev port (spec §18.1: "Admin UI at `localhost:8080`").
const DEV_PORT: u16 = 8080;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let state = AppState::new(Arc::new(InMemoryCaState::default()), Arc::new(SystemClock));
    let app = mtc_admin::router(state);

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, DEV_PORT));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "mtc-admin dev server listening (standalone -- not yet mounted into ca-service, see TODO(bead) in lib.rs)");
    axum::serve(listener, app).await?;
    Ok(())
}
