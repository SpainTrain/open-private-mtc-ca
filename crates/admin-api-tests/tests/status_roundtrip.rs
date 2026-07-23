//! Round-trips a `/status` response between the generated client and server
//! types (ticket openapi-codegen-pipeline; spec §17.2, §17.3).
//!
//! Both crates are generated from `api/admin.openapi.yaml`; this test pins the
//! contract that they agree on the wire format.

use pretty_assertions::assert_eq;
use serde_json::json;

/// A representative `/status` response as it appears on the wire.
fn status_fixture() -> serde_json::Value {
    json!({
        "service": {
            "name": "mtc-ca",
            "version": "0.1.0",
            "region": "us-east-1",
            "started_at": "2026-07-21T12:00:00Z"
        },
        "lease": {
            "holder": "us-east-1",
            "epoch": 42,
            "expires_at": "2026-07-21T12:05:00Z"
        },
        "checkpoint": {
            "tree_size": 1_048_576,
            "root_hash": "9f2c1d8a3b4e5f60718293a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4",
            "timestamp": "2026-07-21T12:00:00Z"
        }
    })
}

#[test]
fn status_response_round_trips_between_client_and_server_types() {
    let wire = status_fixture();

    // Wire JSON -> client type (what `mtcctl` deserializes).
    let client_typed: mtc_admin_api_client::models::StatusResponse =
        serde_json::from_value(wire.clone()).expect("client type accepts the wire format");

    // Client type -> JSON -> server type (what the CA service produces).
    let via_client = serde_json::to_value(&client_typed).expect("client type serializes");
    let server_typed: mtc_admin_api_server::models::StatusResponse =
        serde_json::from_value(via_client).expect("server type accepts client-serialized JSON");

    // Server type -> JSON must reproduce the original wire body exactly.
    let via_server = serde_json::to_value(&server_typed).expect("server type serializes");
    assert_eq!(wire, via_server);

    // Typed spot-checks across the two generated type families.
    let client_lease = client_typed.lease.expect("fixture lease (client)");
    let server_lease = server_typed.lease.expect("fixture lease (server)");
    assert_eq!(client_lease.epoch, server_lease.epoch);
    assert_eq!(server_lease.epoch, 42_i64);
    assert_eq!(client_typed.service.name, server_typed.service.name);
}

#[test]
fn status_response_omits_absent_optional_sections() {
    // A minimal body (no lease, no checkpoint) must round-trip without the
    // optional keys reappearing as nulls.
    let wire = json!({
        "service": { "name": "mtc-ca", "version": "0.1.0", "region": "us-east-1" }
    });

    let client_typed: mtc_admin_api_client::models::StatusResponse =
        serde_json::from_value(wire.clone()).expect("client type accepts minimal body");
    let server_typed: mtc_admin_api_server::models::StatusResponse =
        serde_json::from_value(wire.clone()).expect("server type accepts minimal body");

    assert_eq!(
        wire,
        serde_json::to_value(&client_typed).expect("client type serializes")
    );
    assert_eq!(
        wire,
        serde_json::to_value(&server_typed).expect("server type serializes")
    );
}
