//! Output-format tests for a representative `/status` payload (ticket
//! mtc-no9 Testing: "output-format snapshot tests"), exercising
//! [`mtcctl::output::render`] against the real generated
//! `mtc_admin_api_client::models::StatusResponse` -- the same type
//! `mtcctl status` renders (AC: "human tables, `--output json` and
//! `--output yaml` via serde on the same response types").

use chrono::{DateTime, FixedOffset};
use mtc_admin_api_client::models::{CheckpointStatus, LeaseStatus, ServiceInfo, StatusResponse};
use mtcctl::cli::OutputFormat;
use mtcctl::output::render;
use pretty_assertions::assert_eq;

// Helper sits outside any #[test] fn, so the allow-expect-in-tests
// exemption doesn't reach it (docs/lint-policy.md deviation 1).
#[allow(clippy::expect_used)]
fn dt(s: &str) -> DateTime<FixedOffset> {
    s.parse().expect("fixture timestamp parses")
}

/// The representative payload: identity, lease, and checkpoint all present
/// (the fullest shape `/status` can return -- spec §17.3).
fn full_status_fixture() -> StatusResponse {
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
            root_hash: "9f2c1d8a3b4e5f60718293a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4"
                .to_string(),
            timestamp: dt("2026-07-21T12:00:00Z"),
        }),
    }
}

#[test]
fn json_output_round_trips_the_same_response_type() {
    let status = full_status_fixture();
    let rendered =
        render(&status, OutputFormat::Json, |_| String::new()).expect("json render succeeds");

    let parsed: StatusResponse = serde_json::from_str(&rendered).expect("valid JSON");
    assert_eq!(parsed, status);

    // Stable field names on the wire (agents/scripts parse these).
    assert!(rendered.contains("\"tree_size\""));
    assert!(rendered.contains("\"root_hash\""));
}

#[test]
fn yaml_output_round_trips_the_same_response_type() {
    let status = full_status_fixture();
    let rendered =
        render(&status, OutputFormat::Yaml, |_| String::new()).expect("yaml render succeeds");

    let parsed: StatusResponse = serde_yaml::from_str(&rendered).expect("valid YAML");
    assert_eq!(parsed, status);
}

#[test]
fn json_and_yaml_agree_with_each_other_via_the_common_type() {
    // Both formats are serde over the identical `StatusResponse` value --
    // parsing either back must produce the same struct.
    let status = full_status_fixture();
    let json = render(&status, OutputFormat::Json, |_| String::new()).expect("json render");
    let yaml = render(&status, OutputFormat::Yaml, |_| String::new()).expect("yaml render");

    let from_json: StatusResponse = serde_json::from_str(&json).expect("valid JSON");
    let from_yaml: StatusResponse = serde_yaml::from_str(&yaml).expect("valid YAML");
    assert_eq!(from_json, from_yaml);
}

#[test]
fn human_output_defers_entirely_to_the_supplied_renderer() {
    let status = full_status_fixture();
    let rendered = render(&status, OutputFormat::Human, |s| {
        format!("service={}", s.service.name)
    })
    .expect("human render succeeds");
    assert_eq!(rendered, "service=mtc-ca");
}

#[test]
fn minimal_payload_without_lease_or_checkpoint_round_trips() {
    // /status's `lease`/`checkpoint` are optional (not yet populated by the
    // real CA service -- ticket mtc-gja's scope); the output layer must
    // handle their absence, not just the full fixture.
    let status = StatusResponse {
        service: ServiceInfo {
            name: "mtc-ca".to_string(),
            version: "0.1.0".to_string(),
            region: "local".to_string(),
            started_at: None,
        },
        lease: None,
        checkpoint: None,
    };

    let json = render(&status, OutputFormat::Json, |_| String::new()).expect("json render");
    let parsed: StatusResponse = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed, status);
    assert!(!json.contains("lease"));
    assert!(!json.contains("checkpoint"));
}
