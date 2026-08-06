//! Application-state seam (spec §17.2 acceptance criteria: "`AppState` seam
//! injects CA-service state handles as trait objects so handlers run
//! against the memory backend in tests").
//!
//! [`CaStateProvider`] is the port every admin-API handler reads CA-service
//! data through, injected into [`AppState`] as `Arc<dyn CaStateProvider>`
//! (`prefer-generics-on-hot-paths`: this is an injected, runtime-swappable
//! architectural seam, not a hot path, so dynamic dispatch is the
//! deliberate choice). [`InMemoryCaState`] is the in-memory implementation
//! backing both the test suite and the standalone dev binary
//! (`src/main.rs`) until the real CA service exists.
//!
//! Deliberately framework-agnostic: nothing here depends on
//! `mtc-admin-api-server`'s generated wire types or on axum. The adapter in
//! [`crate::handlers::health`] translates between this domain seam and the
//! `OpenAPI` wire models -- the same "translation stays at the boundary"
//! discipline `no-sdk-types-in-domain` applies to vendor SDK types applies
//! here to generated wire types.

use std::sync::Arc;
use std::time::SystemTime;

use clock::Clock;

/// Domain-level identity/build information for the running instance (spec
/// §17.3 `ServiceInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceIdentity {
    /// Service name (e.g. `mtc-ca`).
    pub name: String,
    /// Semantic version of the running build.
    pub version: String,
    /// Cloud region (or simulated region) of this instance.
    pub region: String,
}

/// One dependency check contributing to readiness (spec §20.5 `/readyz`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyCheck {
    /// Check identifier (e.g. `storage`, `hsm`).
    pub name: String,
    /// Whether this check passed.
    pub ok: bool,
    /// Human-readable failure detail; expected to be `None` when `ok`.
    pub detail: Option<String>,
}

impl DependencyCheck {
    /// Creates a passing check.
    #[must_use]
    pub fn ok(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: true,
            detail: None,
        }
    }

    /// Creates a failing check with a human-readable detail.
    #[must_use]
    pub fn failing(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: false,
            detail: Some(detail.into()),
        }
    }
}

/// The seam the admin API's handlers read CA-service data through.
///
/// A real implementation is injected by the CA service binary once it
/// exists (see the `// TODO(bead)` in `lib.rs`); [`InMemoryCaState`] backs
/// tests and the standalone dev binary in the meantime.
pub trait CaStateProvider: Send + Sync {
    /// Identity/build info for `/status`.
    fn identity(&self) -> ServiceIdentity;

    /// Dependency checks backing `/readyz`. An empty list (no failing
    /// entries) means "ready".
    fn readiness_checks(&self) -> Vec<DependencyCheck>;
}

/// Shared application state handed to every handler via axum's `State`
/// extractor (spec §17.4 `lib.rs`).
#[derive(Clone)]
pub struct AppState {
    ca: Arc<dyn CaStateProvider>,
    clock: Arc<dyn Clock>,
    started_at: SystemTime,
}

impl AppState {
    /// Builds state over an injected CA-service handle and clock.
    ///
    /// `started_at` is read once, here, through `clock` (rule
    /// `no-systemtime-now-in-prod`) -- handlers never read wall-clock time
    /// directly.
    #[must_use]
    pub fn new(ca: Arc<dyn CaStateProvider>, clock: Arc<dyn Clock>) -> Self {
        let started_at = clock.now();
        Self {
            ca,
            clock,
            started_at,
        }
    }

    /// Identity/build info for `/status`, from the injected [`CaStateProvider`].
    #[must_use]
    pub fn identity(&self) -> ServiceIdentity {
        self.ca.identity()
    }

    /// Dependency checks for `/readyz`, from the injected [`CaStateProvider`].
    #[must_use]
    pub fn readiness_checks(&self) -> Vec<DependencyCheck> {
        self.ca.readiness_checks()
    }

    /// The instant this state (and so, in practice, the process) started.
    #[must_use]
    pub const fn started_at(&self) -> SystemTime {
        self.started_at
    }

    /// The injected clock, for callers that need it directly.
    #[must_use]
    pub const fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }
}

/// In-memory [`CaStateProvider`] for tests and the standalone dev binary.
///
/// Real CA-service wiring is future work -- see the `// TODO(bead)` in
/// `lib.rs`.
#[derive(Debug, Clone)]
pub struct InMemoryCaState {
    identity: ServiceIdentity,
    checks: Vec<DependencyCheck>,
}

impl InMemoryCaState {
    /// Creates state reporting the given identity and no readiness checks
    /// (so `/readyz` is unconditionally ready).
    #[must_use]
    pub const fn new(identity: ServiceIdentity) -> Self {
        Self {
            identity,
            checks: Vec::new(),
        }
    }

    /// Adds a dependency check, builder-style (tests exercising `/readyz`).
    #[must_use]
    pub fn with_check(mut self, check: DependencyCheck) -> Self {
        self.checks.push(check);
        self
    }
}

impl Default for InMemoryCaState {
    /// A `mtc-ca` identity in the simulated `local` region, at this crate's
    /// own build version.
    fn default() -> Self {
        Self::new(ServiceIdentity {
            name: "mtc-ca".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            region: "local".to_string(),
        })
    }
}

impl CaStateProvider for InMemoryCaState {
    fn identity(&self) -> ServiceIdentity {
        self.identity.clone()
    }

    fn readiness_checks(&self) -> Vec<DependencyCheck> {
        self.checks.clone()
    }
}

#[cfg(test)]
mod tests {
    use clock::FakeClock;
    use pretty_assertions::assert_eq;

    use super::*;

    fn identity(region: &str) -> ServiceIdentity {
        ServiceIdentity {
            name: "mtc-ca".to_string(),
            version: "0.1.0".to_string(),
            region: region.to_string(),
        }
    }

    #[test]
    fn default_in_memory_state_is_ready_with_no_checks() {
        let state = InMemoryCaState::default();
        assert_eq!(state.identity().name, "mtc-ca");
        assert!(state.readiness_checks().is_empty());
    }

    #[test]
    fn with_check_accumulates_in_order() {
        let state = InMemoryCaState::new(identity("us-east-1"))
            .with_check(DependencyCheck::ok("storage"))
            .with_check(DependencyCheck::failing("hsm", "unreachable"));
        let checks = state.readiness_checks();
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0], DependencyCheck::ok("storage"));
        assert_eq!(checks[1], DependencyCheck::failing("hsm", "unreachable"));
    }

    #[test]
    fn app_state_reads_started_at_from_injected_clock_once() {
        let clock = Arc::new(FakeClock::new(SystemTime::UNIX_EPOCH));
        let ca: Arc<dyn CaStateProvider> = Arc::new(InMemoryCaState::new(identity("eu-west-1")));
        let state = AppState::new(ca, clock.clone());
        assert_eq!(state.started_at(), SystemTime::UNIX_EPOCH);

        // Advancing the clock after construction must not move the
        // already-captured `started_at` snapshot.
        clock.advance(std::time::Duration::from_mins(1));
        assert_eq!(state.started_at(), SystemTime::UNIX_EPOCH);
        assert_eq!(state.identity().region, "eu-west-1");
    }

    #[test]
    fn app_state_is_send_sync_and_cheaply_cloneable() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AppState>();

        let clock = Arc::new(FakeClock::default());
        let ca: Arc<dyn CaStateProvider> = Arc::new(InMemoryCaState::default());
        let state = AppState::new(ca, clock);
        let cloned = state.clone();
        // The clone is an independently usable handle reporting the same
        // underlying state, not just a type-checks-as-Clone formality.
        assert_eq!(state.identity(), cloned.identity());
        assert_eq!(state.started_at(), cloned.started_at());
    }
}
