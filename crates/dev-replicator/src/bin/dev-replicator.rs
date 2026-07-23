//! `dev-replicator` binary: one directed replication link, configured via
//! `REPL_*` environment variables (see [`dev_replicator::config::LinkConfig`]).
//!
//! ```console
//! $ REPL_LINK_NAME=us-east-1-to-us-west-2 \
//!   REPL_SOURCE_ENDPOINT_URL=http://127.0.0.1:4566 \
//!   REPL_TARGET_ENDPOINT_URL=http://127.0.0.1:4567 \
//!   REPL_S3_BUCKET=mtc-log-local \
//!   REPL_DDB_TABLE=mtc-log-coordination \
//!   REPL_LAG_MS=5000 \
//!   cargo run -p dev-replicator
//! ```
//!
//! Runtime control (mr-replication-sim AC): `curl -X POST 127.0.0.1:9300/lag
//! -d '{"kind":"stalled"}'`, `curl 127.0.0.1:9300/status`. See
//! `crate::control` for the full route table.

use std::sync::Arc;

use clock::SystemClock;
use dev_replicator::config::{EndpointConfig, LinkConfig};
use dev_replicator::control;
use dev_replicator::ddb::DdbPoller;
use dev_replicator::link::Link;
use dev_replicator::s3::S3Poller;
use eyre::WrapErr;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dev_replicator=info".into()),
        )
        .init();

    let cfg = LinkConfig::from_env().wrap_err(
        "invalid dev-replicator configuration — see REPL_* env vars in dev_replicator::config docs",
    )?;
    tracing::info!(
        link = %cfg.link_name,
        source = %cfg.source.endpoint_url,
        target = %cfg.target.endpoint_url,
        s3_bucket = ?cfg.s3_bucket,
        ddb_table = ?cfg.ddb_table,
        lag = ?cfg.initial_lag,
        poll_interval = ?cfg.poll_interval,
        "starting replication link"
    );

    let clock: Arc<dyn clock::Clock> = Arc::new(SystemClock);

    let s3 = if let Some(bucket) = &cfg.s3_bucket {
        let source = build_s3_client(&cfg.source);
        let target = build_s3_client(&cfg.target);
        Some(S3Poller::new(
            source,
            target,
            bucket.clone(),
            Arc::clone(&clock),
            cfg.initial_lag,
        ))
    } else {
        None
    };
    let ddb = if let Some(table) = &cfg.ddb_table {
        let source = build_ddb_client(&cfg.source);
        let target = build_ddb_client(&cfg.target);
        Some(DdbPoller::new(
            source,
            target,
            table.clone(),
            Arc::clone(&clock),
            cfg.initial_lag,
        ))
    } else {
        None
    };

    let (link, status) = Link::new(
        cfg.link_name.clone(),
        cfg.poll_interval,
        cfg.initial_lag,
        s3,
        ddb,
    );
    let control_handle = link.control_handle();
    let control_app = control::router(control_handle, status);

    let listener = tokio::net::TcpListener::bind(cfg.control_addr)
        .await
        .wrap_err_with(|| format!("failed to bind control endpoint on {}", cfg.control_addr))?;
    tracing::info!(addr = %cfg.control_addr, "control endpoint listening");

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(async move { axum::serve(listener, control_app).await });
    let link_task = tokio::spawn(link.run(shutdown_rx));

    tokio::signal::ctrl_c()
        .await
        .wrap_err("failed to listen for ctrl-c")?;
    tracing::info!("shutdown signal received");
    // Ignore the result: the receiver can only have been dropped if the
    // link task already exited, in which case there is nothing left to stop.
    let _ = shutdown_tx.send(true);
    // Ignore the join result: a panicked link task has nothing further for
    // us to act on here — the process is exiting either way.
    let _ = link_task.await;
    server.abort();

    Ok(())
}

/// Builds an S3 client bound to one explicit endpoint/region with static
/// `LocalStack` test credentials.
///
/// Deliberately **not** the `aws-config` ambient-environment default chain:
/// one link talks to *two different* endpoints, so each client's config must
/// be built explicitly per side — see the crate-level docs.
fn build_s3_client(endpoint: &EndpointConfig) -> aws_sdk_s3::Client {
    let credentials =
        aws_sdk_s3::config::Credentials::new("test", "test", None, None, "dev-replicator-static");
    let config = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .region(aws_sdk_s3::config::Region::new(endpoint.region.clone()))
        .endpoint_url(&endpoint.endpoint_url)
        .credentials_provider(credentials)
        // LocalStack needs path-style addressing (bucket.localhost doesn't resolve).
        .force_path_style(true)
        .build();
    aws_sdk_s3::Client::from_conf(config)
}

/// Builds a `DynamoDB` client bound to one explicit endpoint/region — see
/// [`build_s3_client`].
fn build_ddb_client(endpoint: &EndpointConfig) -> aws_sdk_dynamodb::Client {
    let credentials = aws_sdk_dynamodb::config::Credentials::new(
        "test",
        "test",
        None,
        None,
        "dev-replicator-static",
    );
    let config = aws_sdk_dynamodb::config::Builder::new()
        .behavior_version(aws_sdk_dynamodb::config::BehaviorVersion::latest())
        .region(aws_sdk_dynamodb::config::Region::new(
            endpoint.region.clone(),
        ))
        .endpoint_url(&endpoint.endpoint_url)
        .credentials_provider(credentials)
        .build();
    aws_sdk_dynamodb::Client::from_conf(config)
}
