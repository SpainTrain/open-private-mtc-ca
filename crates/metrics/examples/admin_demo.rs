//! Demo for ticket obs-metrics-core: registers the §20.1 metric set, records
//! sample values, prints one EMF flush to stdout, and serves Prometheus text
//! exposition on the default admin port.
//!
//! Run:
//!
//! ```console
//! cargo run -p mtc-metrics --example admin_demo
//! # in another shell:
//! curl -s localhost:9464/metrics
//! ```

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};

use mtc_metrics::{
    serve_admin, CaMetrics, EmfConfig, EmfEmitter, MetricsRegistry, TimestampMillis,
    DEFAULT_ADMIN_PORT,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> eyre::Result<()> {
    let registry = MetricsRegistry::new();
    let metrics = CaMetrics::register(&registry)?;

    // Sample §20.1 activity so every metric family has data.
    metrics.issuance_latency_seconds.observe(1.8);
    metrics.issuance_latency_seconds.observe(0.9);
    metrics.batch_commit_latency_seconds.observe(0.31);
    metrics.hsm_signing_latency_seconds.observe(0.021);
    metrics.lease_renewals_total.inc_by(12);
    metrics.lease_renewals_failed_total.inc();
    metrics.epoch_advances_total.inc();
    metrics.batches_committed_total.inc_by(4);
    metrics.batches_abandoned_total.inc();
    metrics.entries_by_source_total.inc_by("acme", 128);
    metrics.entries_by_source_total.inc_by("api", 32);
    metrics.tile_cache_hits_total.inc_by(950);
    metrics.tile_cache_misses_total.inc_by(50);
    metrics.crr_replication_lag_seconds.set(0.8);
    metrics.ddb_replication_lag_seconds.set(0.12);

    // EMF flush to stdout (structured JSON log lines — what a Fargate/Lambda
    // log driver would ship to CloudWatch Logs / LocalStack).
    let config = EmfConfig {
        enabled: true,
        namespace: "MtcCa".to_string(),
        default_dimensions: BTreeMap::from([("service".to_string(), "mtc-ca-demo".to_string())]),
    };
    if let Some(mut emitter) = EmfEmitter::from_config(&registry, &config) {
        // Fixed timestamp: wall-clock reads are forbidden outside tests
        // (§22.11) — production services obtain time from their injected
        // `Clock` and pass `TimestampMillis` in.
        let demo_timestamp = TimestampMillis(1_700_000_000_000);
        println!("--- EMF events (one JSON log line each) ---");
        emitter.flush_to(&mut std::io::stdout().lock(), demo_timestamp)?;
        println!("-------------------------------------------");
    }

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_ADMIN_PORT));
    let server = serve_admin(registry, addr).await?;
    println!(
        "admin endpoint: http://{}/metrics  (ctrl-c to stop)",
        server.local_addr()
    );
    tokio::signal::ctrl_c().await?;
    server.shutdown().await;
    Ok(())
}
