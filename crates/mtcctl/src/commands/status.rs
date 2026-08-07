//! `mtcctl status` (spec §17.3: "Show service status, lease, checkpoint").
//!
//! Ticket mtc-no9's one wired-up operation ("`mtcctl status` returns live
//! data from the health/version endpoint as first working command").

use mtc_admin_api_client::apis::status_api;
use mtc_admin_api_client::models::StatusResponse;

use crate::cli::GlobalArgs;
use crate::error::CliError;
use crate::{client, output};

/// Fetches `/status` and emits it in the requested output format.
///
/// # Errors
///
/// Returns [`CliError::Api`] / [`CliError::Connection`] if the admin API
/// call fails, or [`CliError::Render`] if formatting the response fails.
pub async fn run(global: &GlobalArgs) -> Result<(), CliError> {
    let config = client::configuration(global);
    let status = status_api::get_status(&config)
        .await
        .map_err(|err| client::classify(&global.endpoint, err))?;
    output::emit(&status, global.output, render_human)
}

/// Human-readable rendering: a `field | value` table of the service
/// identity, plus `lease.*` / `checkpoint.*` rows when the server reports
/// them. Both sections are optional in the wire schema -- the CA service
/// doesn't populate `lease`/`checkpoint` yet (ticket mtc-gja's scope), so
/// their absence is a valid response, not a rendering gap.
fn render_human(status: &StatusResponse) -> String {
    let mut table = output::key_value_table();
    table.add_row(["name".to_string(), status.service.name.clone()]);
    table.add_row(["version".to_string(), status.service.version.clone()]);
    table.add_row(["region".to_string(), status.service.region.clone()]);
    if let Some(started_at) = &status.service.started_at {
        table.add_row(["started_at".to_string(), started_at.to_rfc3339()]);
    }
    if let Some(lease) = &status.lease {
        table.add_row(["lease.holder".to_string(), lease.holder.clone()]);
        table.add_row(["lease.epoch".to_string(), lease.epoch.to_string()]);
        table.add_row([
            "lease.expires_at".to_string(),
            lease.expires_at.to_rfc3339(),
        ]);
    }
    if let Some(checkpoint) = &status.checkpoint {
        table.add_row([
            "checkpoint.tree_size".to_string(),
            checkpoint.tree_size.to_string(),
        ]);
        table.add_row([
            "checkpoint.root_hash".to_string(),
            checkpoint.root_hash.clone(),
        ]);
        table.add_row([
            "checkpoint.timestamp".to_string(),
            checkpoint.timestamp.to_rfc3339(),
        ]);
    }
    table.to_string()
}

#[cfg(test)]
mod tests {
    use mtc_admin_api_client::models::{CheckpointStatus, LeaseStatus, ServiceInfo};

    use super::{render_human, StatusResponse};

    fn dt(s: &str) -> chrono::DateTime<chrono::FixedOffset> {
        s.parse().expect("fixture timestamp parses")
    }

    fn sample() -> StatusResponse {
        StatusResponse {
            service: ServiceInfo {
                name: "mtc-ca".to_string(),
                version: "0.1.0".to_string(),
                region: "us-east-1".to_string(),
                started_at: Some(dt("2026-07-21T12:00:00Z")),
            },
            lease: Some(LeaseStatus {
                holder: "us-east-1".to_string(),
                epoch: 42,
                expires_at: dt("2026-07-21T12:05:00Z"),
            }),
            checkpoint: Some(CheckpointStatus {
                tree_size: 1_048_576,
                root_hash: "9f2c1d8a3b4e".to_string(),
                timestamp: dt("2026-07-21T12:00:00Z"),
            }),
        }
    }

    #[test]
    fn human_render_includes_service_identity() {
        let rendered = render_human(&sample());
        assert!(rendered.contains("mtc-ca"));
        assert!(rendered.contains("us-east-1"));
        assert!(rendered.contains("0.1.0"));
    }

    #[test]
    fn human_render_includes_lease_and_checkpoint_when_present() {
        let rendered = render_human(&sample());
        assert!(rendered.contains("lease.holder"));
        assert!(rendered.contains("42"));
        assert!(rendered.contains("checkpoint.tree_size"));
        assert!(rendered.contains("1048576"));
    }

    #[test]
    fn human_render_omits_absent_lease_and_checkpoint() {
        let mut status = sample();
        status.lease = None;
        status.checkpoint = None;
        let rendered = render_human(&status);
        assert!(!rendered.contains("lease."));
        assert!(!rendered.contains("checkpoint."));
    }
}
