//! Admin API core (spec §17.2, §17.4): mounts routes generated from
//! `api/admin.openapi.yaml` (`crates/admin-api-server`) onto an axum
//! [`Router`].
//!
//! Ticket mtc-gja stands up this crate and wires the health/version surface
//! (`/healthz`, `/readyz`, `/status`) end-to-end -- spec, generated stub,
//! handler -- plus the [`AppState`]/[`AdminApiError`] seam later
//! business-operation tickets (spec §17.5) build on. UI routes/templates
//! (§17.4's `handlers/dashboard.rs` etc., `templates.rs`) and business
//! operations beyond health/version are out of this ticket's scope.
//!
//! ```
//! use std::sync::Arc;
//!
//! use clock::SystemClock;
//! use mtc_admin::{AppState, InMemoryCaState};
//!
//! let state = AppState::new(Arc::new(InMemoryCaState::default()), Arc::new(SystemClock));
//! let _app = mtc_admin::router(state);
//! ```

// TODO(bead): mount this router into the `ca-service` binary. Spec §17.4
// says the admin surface runs "from the same Fargate task as the CA
// service", but `crates/ca-service` does not exist yet as of this ticket
// (built concurrently by another agent in a separate worktree). `src/main.rs`
// in this crate is a standalone stand-in dev server so `mtc-admin` is
// independently runnable (`cargo run -p mtc-admin`) and testable until that
// mount point exists -- file a bead for the ca-service mount once that crate
// lands.

pub mod error;
pub mod handlers;
pub mod state;

use axum::Router;

pub use error::AdminApiError;
pub use state::{AppState, CaStateProvider, DependencyCheck, InMemoryCaState, ServiceIdentity};

use handlers::health::HealthApi;

/// Builds the mounted admin API router (spec §17.4 `lib.rs`: "mounts UI +
/// API routes (axum)") over the given [`AppState`].
///
/// Currently mounts the generated health/status surface (`/healthz`,
/// `/readyz`, `/status`) via `mtc_admin_api_server::server::new`; UI routes
/// and further API operations layer on in later tickets without changing
/// this seam.
pub fn router(state: AppState) -> Router {
    mtc_admin_api_server::server::new(std::sync::Arc::new(HealthApi::new(state)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use clock::FakeClock;

    use super::*;

    #[test]
    fn router_builds_over_in_memory_state() {
        let state = AppState::new(
            Arc::new(InMemoryCaState::default()),
            Arc::new(FakeClock::default()),
        );
        let _app = router(state);
    }
}
