//! Local HTTP control endpoint for one link.
//!
//! Ticket dev-crr-replication-sim AC "pause/resume via signal or local
//! control endpoint"; mr-replication-sim AC "Lag configurable per link at
//! runtime, including stall".
//!
//! Routes:
//!
//! | Method | Path        | Effect                                              |
//! |--------|-------------|------------------------------------------------------|
//! | GET    | `/status`   | Current lag policy, pause state, per-resource queue depth/lag (readiness observability) |
//! | GET    | `/healthz`  | Always `200 OK` once the server is up                |
//! | POST   | `/lag`      | `{"lag_ms": N}` or `{"stall": true}` — runtime lag change |
//! | POST   | `/pause`    | Halts discovery *and* apply (the partition hook)      |
//! | POST   | `/resume`   | Resumes a paused link                                 |
//!
//! # Pause vs. stall — two different faults, deliberately distinct
//!
//! - **Pause** simulates a severed network: the link stops discovering *and*
//!   applying entirely, as if the region were partitioned. On resume, the
//!   next discovery pass naturally catches up on everything missed.
//! - **Stall** ([`LagPolicy::Stalled`], set via `POST /lag {"stall": true}`)
//!   simulates infinite replication lag while the link stays "connected":
//!   discovery keeps running (so `/status` shows a growing queue), but
//!   nothing is ever applied until the policy changes — the shape of
//!   `chaos-crr-stall` (spec §19.9), where the region is up but replication
//!   itself is stuck.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, RwLock};

use crate::lag::LagPolicy;

/// JSON view of [`LagPolicy`] for the control API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LagPolicyView {
    /// Fixed lag in milliseconds.
    Fixed {
        /// Lag, in milliseconds.
        ms: u64,
    },
    /// Infinite lag — nothing is ever applied until the policy changes.
    Stalled,
}

impl From<LagPolicy> for LagPolicyView {
    fn from(policy: LagPolicy) -> Self {
        match policy {
            LagPolicy::Fixed(d) => Self::Fixed {
                ms: u64::try_from(d.as_millis()).unwrap_or(u64::MAX),
            },
            LagPolicy::Stalled => Self::Stalled,
        }
    }
}

impl From<LagPolicyView> for LagPolicy {
    fn from(view: LagPolicyView) -> Self {
        match view {
            LagPolicyView::Fixed { ms } => Self::Fixed(Duration::from_millis(ms)),
            LagPolicyView::Stalled => Self::Stalled,
        }
    }
}

/// Observability snapshot for one resource (S3 or DDB) on the link, updated
/// by [`crate::link::Link`]'s poll loop after every cycle.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ResourceStatus {
    /// Discoveries not yet applied to the target.
    pub pending: usize,
    /// Total changes applied to the target so far.
    pub applied: usize,
    /// Age of the oldest pending discovery, in seconds, if any.
    pub oldest_pending_age_secs: Option<f64>,
}

/// Full `/status` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkStatus {
    /// This link's configured name.
    pub link_name: String,
    /// Whether the link is currently paused (the partition hook).
    pub paused: bool,
    /// The link's current lag policy.
    pub lag: LagPolicyView,
    /// S3 resource status, if this link replicates a bucket.
    pub s3: Option<ResourceStatus>,
    /// `DynamoDB` resource status, if this link replicates a table.
    pub ddb: Option<ResourceStatus>,
}

/// Shared, poll-loop-updated status snapshot, read by the `/status` handler.
pub type SharedStatus = Arc<RwLock<LinkStatus>>;

impl LinkStatus {
    /// Starting snapshot before the first poll cycle completes.
    #[must_use]
    pub fn initial(link_name: String, lag: LagPolicy, has_s3: bool, has_ddb: bool) -> Self {
        Self {
            link_name,
            paused: false,
            lag: lag.into(),
            s3: has_s3.then(ResourceStatus::default),
            ddb: has_ddb.then(ResourceStatus::default),
        }
    }
}

/// Runtime controls the poll loop reads every cycle; the control endpoint
/// writes them. Cheap to clone (an `Arc`/`watch::Sender` pair).
#[derive(Clone)]
pub struct ControlHandle {
    /// Broadcasts lag-policy changes to the poll loop.
    pub lag_tx: watch::Sender<LagPolicy>,
    /// Whether the link is paused (the partition hook).
    pub paused: Arc<AtomicBool>,
}

impl ControlHandle {
    /// Creates a handle plus the [`watch::Receiver`] the poll loop consumes.
    #[must_use]
    pub fn new(initial_lag: LagPolicy) -> (Self, watch::Receiver<LagPolicy>) {
        let (lag_tx, lag_rx) = watch::channel(initial_lag);
        (
            Self {
                lag_tx,
                paused: Arc::new(AtomicBool::new(false)),
            },
            lag_rx,
        )
    }

    /// Whether the link is currently paused.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
struct AppState {
    control: ControlHandle,
    status: SharedStatus,
}

/// Body for `POST /lag`.
#[derive(Debug, Deserialize)]
struct SetLagRequest {
    #[serde(flatten)]
    view: LagPolicyView,
}

/// Builds the control router for one link.
pub fn router(control: ControlHandle, status: SharedStatus) -> Router {
    let state = AppState { control, status };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/status", get(get_status))
        .route("/lag", post(post_lag))
        .route("/pause", post(post_pause))
        .route("/resume", post(post_resume))
        .with_state(state)
}

async fn healthz() -> StatusCode {
    StatusCode::OK
}

async fn get_status(State(state): State<AppState>) -> Json<LinkStatus> {
    Json(state.status.read().await.clone())
}

async fn post_lag(
    State(state): State<AppState>,
    Json(req): Json<SetLagRequest>,
) -> Json<LagPolicyView> {
    let policy: LagPolicy = req.view.into();
    // Ignore the send error: it only fails if every receiver (the poll
    // loop's tasks) has already been dropped, i.e. the link is shutting
    // down — nothing useful to do with that here.
    let _ = state.control.lag_tx.send(policy);
    Json(policy.into())
}

async fn post_pause(State(state): State<AppState>) -> StatusCode {
    state.control.paused.store(true, Ordering::SeqCst);
    StatusCode::OK
}

async fn post_resume(State(state): State<AppState>) -> StatusCode {
    state.control.paused.store(false, Ordering::SeqCst);
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_status() -> SharedStatus {
        Arc::new(RwLock::new(LinkStatus::initial(
            "us-east-1-to-us-west-2".to_string(),
            LagPolicy::Fixed(Duration::from_secs(5)),
            true,
            true,
        )))
    }

    async fn body_json<T: serde::de::DeserializeOwned>(response: axum::response::Response) -> T {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn healthz_is_always_ok() {
        let (control, _rx) = ControlHandle::new(LagPolicy::immediate());
        let app = router(control, test_status());

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn status_reflects_the_shared_snapshot() {
        let (control, _rx) = ControlHandle::new(LagPolicy::Fixed(Duration::from_secs(5)));
        let status = test_status();
        let app = router(control, status);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: LinkStatus = body_json(resp).await;
        assert_eq!(body.link_name, "us-east-1-to-us-west-2");
        assert_eq!(body.lag, LagPolicyView::Fixed { ms: 5000 });
        assert!(body.s3.is_some());
        assert!(body.ddb.is_some());
    }

    #[tokio::test]
    async fn post_lag_updates_the_watch_channel() {
        let (control, mut rx) = ControlHandle::new(LagPolicy::immediate());
        let app = router(control, test_status());

        let req = Request::builder()
            .method("POST")
            .uri("/lag")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"kind":"fixed","ms":2500}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: LagPolicyView = body_json(resp).await;
        assert_eq!(body, LagPolicyView::Fixed { ms: 2500 });

        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), LagPolicy::Fixed(Duration::from_millis(2500)));
    }

    #[tokio::test]
    async fn post_lag_stall_sets_infinite_lag_at_runtime() {
        let (control, mut rx) = ControlHandle::new(LagPolicy::Fixed(Duration::from_secs(5)));
        let app = router(control, test_status());

        let req = Request::builder()
            .method("POST")
            .uri("/lag")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"kind":"stalled"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), LagPolicy::Stalled);
    }

    #[tokio::test]
    async fn pause_then_resume_round_trips() {
        let (control, _rx) = ControlHandle::new(LagPolicy::immediate());
        let paused = Arc::clone(&control.paused);
        let app = router(control, test_status());

        let pause_req = Request::builder()
            .method("POST")
            .uri("/pause")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(pause_req).await.unwrap().status(),
            StatusCode::OK
        );
        assert!(paused.load(Ordering::SeqCst));

        let resume_req = Request::builder()
            .method("POST")
            .uri("/resume")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.oneshot(resume_req).await.unwrap().status(),
            StatusCode::OK
        );
        assert!(!paused.load(Ordering::SeqCst));
    }
}
