//! Orchestrates one replication link.
//!
//! Owns whichever pollers are configured (S3, `DynamoDB`, or both), the
//! shared runtime controls the control endpoint writes to, and the poll loop
//! that drives discovery/apply cycles.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::control::{ControlHandle, LinkStatus, ResourceStatus, SharedStatus};
use crate::ddb::DdbPoller;
use crate::lag::LagPolicy;
use crate::s3::S3Poller;

/// One directed replication link. Build with [`Link::new`], hand its
/// [`ControlHandle`] to the control HTTP router (`control::router`), then
/// [`Link::run`] it until a shutdown signal arrives.
pub struct Link {
    name: String,
    poll_interval: Duration,
    control: ControlHandle,
    lag_rx: watch::Receiver<LagPolicy>,
    status: SharedStatus,
    s3: Option<S3Poller>,
    ddb: Option<DdbPoller>,
}

impl Link {
    /// Builds a link from its already-constructed pollers (each poller
    /// carries its own injected `Arc<dyn Clock>` — see
    /// `bin/dev-replicator.rs`).
    #[must_use]
    pub fn new(
        name: String,
        poll_interval: Duration,
        initial_lag: LagPolicy,
        s3: Option<S3Poller>,
        ddb: Option<DdbPoller>,
    ) -> (Self, SharedStatus) {
        let (control, lag_rx) = ControlHandle::new(initial_lag);
        let status = Arc::new(tokio::sync::RwLock::new(LinkStatus::initial(
            name.clone(),
            initial_lag,
            s3.is_some(),
            ddb.is_some(),
        )));
        (
            Self {
                name,
                poll_interval,
                control,
                lag_rx,
                status: Arc::clone(&status),
                s3,
                ddb,
            },
            status,
        )
    }

    /// A cloneable handle to this link's runtime controls, for wiring into
    /// [`crate::control::router`].
    #[must_use]
    pub fn control_handle(&self) -> ControlHandle {
        self.control.clone()
    }

    /// The shared status snapshot, for wiring into [`crate::control::router`].
    #[must_use]
    pub fn shared_status(&self) -> SharedStatus {
        Arc::clone(&self.status)
    }

    /// Runs discovery/apply cycles every `poll_interval` until `shutdown`
    /// carries `true`.
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        tracing::info!(link = %self.name, "link shutting down");
                        break;
                    }
                }
                () = tokio::time::sleep(self.poll_interval) => {
                    self.run_one_cycle().await;
                }
            }
        }
    }

    /// Runs exactly one discover+apply cycle (both resources, if
    /// configured), honoring the current pause/lag state. `pub` — besides
    /// backing [`Self::run`]'s loop, it is the seam integration tests use to
    /// drive a link deterministically without waiting on `poll_interval`.
    pub async fn run_one_cycle(&mut self) {
        // Pick up any runtime lag change since the last cycle (mr-replication-sim
        // AC: lag configurable per link at runtime).
        if self.lag_rx.has_changed().unwrap_or(false) {
            let policy = *self.lag_rx.borrow_and_update();
            if let Some(s3) = &mut self.s3 {
                s3.set_lag_policy(policy);
            }
            if let Some(ddb) = &mut self.ddb {
                ddb.set_lag_policy(policy);
            }
        }

        let paused = self.control.is_paused();
        if !paused {
            self.run_s3_cycle().await;
            self.run_ddb_cycle().await;
        }
        self.publish_status(paused).await;
    }

    async fn run_s3_cycle(&mut self) {
        let Some(s3) = &mut self.s3 else { return };
        if let Err(err) = s3.discover().await {
            tracing::error!(link = %self.name, error = %err, "s3 discovery failed");
        }
        let summary = s3.apply_due().await;
        if summary.attempted() > 0 {
            tracing::info!(
                link = %self.name,
                resource = "s3",
                applied = summary.applied,
                failed = summary.failed,
                lag_policy = ?s3.lag_policy(),
                "s3 replication cycle"
            );
        }
    }

    async fn run_ddb_cycle(&mut self) {
        let Some(ddb) = &mut self.ddb else { return };
        if let Err(err) = ddb.discover().await {
            tracing::error!(link = %self.name, error = %err, "ddb discovery failed");
        }
        let summary = ddb.apply_due().await;
        if summary.attempted() > 0 {
            tracing::info!(
                link = %self.name,
                resource = "ddb",
                applied = summary.applied,
                stale = summary.stale,
                failed = summary.failed,
                lag_policy = ?ddb.lag_policy(),
                "ddb replication cycle"
            );
        }
    }

    async fn publish_status(&self, paused: bool) {
        let active_lag = self
            .s3
            .as_ref()
            .map(S3Poller::lag_policy)
            .or_else(|| self.ddb.as_ref().map(DdbPoller::lag_policy))
            .unwrap_or_else(LagPolicy::immediate);

        let mut status = self.status.write().await;
        status.paused = paused;
        status.lag = active_lag.into();
        if let Some(s3) = &self.s3 {
            status.s3 = Some(ResourceStatus {
                pending: s3.pending_len(),
                applied: s3.applied_len(),
                oldest_pending_age_secs: s3.oldest_pending_age().map(|d| d.as_secs_f64()),
            });
        }
        if let Some(ddb) = &self.ddb {
            status.ddb = Some(ResourceStatus {
                pending: ddb.pending_len(),
                applied: ddb.applied_len(),
                oldest_pending_age_secs: ddb.oldest_pending_age().map(|d| d.as_secs_f64()),
            });
        }
    }
}
