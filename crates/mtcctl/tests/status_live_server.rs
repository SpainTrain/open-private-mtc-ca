//! Runs the compiled `mtcctl` binary against a real, in-process admin API
//! server and asserts its JSON output is stable (ticket mtc-no9 Testing:
//! "Integration: run binary against an in-process axum test server; assert
//! JSON output is stable").
//!
//! Uses the real `mtc-admin` router + `InMemoryCaState` -- the same server
//! `make demo` runs (spec §18.1) -- bound to an ephemeral port instead of
//! `:8080`, rather than a hand-rolled fake, so this test exercises the
//! actual wire contract between `mtcctl` and the admin API.
//!
//! All `.expect()` calls below sit directly inside this file's single
//! `#[tokio::test]` function body, not a helper outside it, so the
//! documented `allow-expect-in-tests` exemption applies without a scoped
//! `#[allow(...)]` (docs/lint-policy.md deviation 1).

use std::net::{Ipv4Addr, SocketAddr};
use std::process::Command;
use std::sync::Arc;
use std::time::SystemTime;

use clock::FakeClock;
use mtc_admin::{AppState, InMemoryCaState};

// `flavor = "multi_thread"`: the default single-threaded test runtime would
// starve here -- `std::process::Command::output()` below blocks its OS
// thread for the whole subprocess run, and with only one runtime thread
// that's the same thread the spawned `axum::serve` task needs polled on to
// ever accept the connection `mtcctl` is about to make.
#[tokio::test(flavor = "multi_thread")]
async fn status_json_output_is_stable_against_a_live_server() {
    let state = AppState::new(
        Arc::new(InMemoryCaState::default()),
        Arc::new(FakeClock::new(SystemTime::UNIX_EPOCH)),
    );
    let app = mtc_admin::router(state);

    let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .expect("bind an ephemeral port");
    let addr = listener.local_addr().expect("listener has a local address");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("in-process test server");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_mtcctl"))
        .args([
            "--endpoint",
            &format!("http://{addr}"),
            "--output",
            "json",
            "status",
        ])
        .output()
        .expect("mtcctl runs to completion");

    assert!(
        output.status.success(),
        "mtcctl exited non-zero (code {:?}); stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is valid JSON: {e}\nstdout was: {stdout:?}"));

    // The in-memory server's default identity (crates/admin/src/state.rs);
    // stable regardless of when/where this test runs.
    assert_eq!(parsed["service"]["name"], "mtc-ca");
    assert_eq!(parsed["service"]["region"], "local");
    assert!(parsed["service"]["version"].is_string());
    assert!(parsed["service"]["started_at"].is_string());

    // lease/checkpoint are not yet populated by the real server (ticket
    // mtc-gja's scope) -- both are optional in the schema, so simply
    // absent rather than null.
    assert!(parsed.get("lease").is_none());
    assert!(parsed.get("checkpoint").is_none());
}

// See the comment on the previous test re: why this needs a multi-thread
// runtime.
#[tokio::test(flavor = "multi_thread")]
async fn status_human_output_is_non_empty_against_a_live_server() {
    let state = AppState::new(
        Arc::new(InMemoryCaState::default()),
        Arc::new(FakeClock::new(SystemTime::UNIX_EPOCH)),
    );
    let app = mtc_admin::router(state);

    let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .expect("bind an ephemeral port");
    let addr = listener.local_addr().expect("listener has a local address");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("in-process test server");
    });

    // Default `--output` (human) with no explicit flag, per the AC's
    // "human default".
    let output = Command::new(env!("CARGO_BIN_EXE_mtcctl"))
        .args(["--endpoint", &format!("http://{addr}"), "status"])
        .output()
        .expect("mtcctl runs to completion");

    assert!(
        output.status.success(),
        "mtcctl exited non-zero (code {:?}); stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.contains("mtc-ca"));
    assert!(stdout.contains("local"));
}
