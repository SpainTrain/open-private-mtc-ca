//! Typed metrics facade for the MTC CA — the spec §20.1 metric set with
//! Prometheus-compatible exposition and `CloudWatch` EMF emission, usable from
//! Fargate services and Lambdas (ticket obs-metrics-core).
//!
//! One registry, two exporters:
//!
//! - **Prometheus**: [`MetricsRegistry::encode_prometheus_text`] renders the
//!   text exposition format; [`serve_admin`] serves it at `GET /metrics` on
//!   an admin port ([`DEFAULT_ADMIN_PORT`]).
//! - **`CloudWatch` EMF**: [`EmfEmitter`] (gated behind [`EmfConfig`]) turns
//!   the same registry into Embedded-Metric-Format JSON log lines — plain
//!   structured logging, ingestible by `LocalStack` `CloudWatch` Logs in dev.
//!   No AWS SDK types appear anywhere in this crate (§22.8).
//!
//! [`CaMetrics::register`] constructs all 13 §20.1 metrics with their exact
//! spec names and instrument kinds; the names themselves live in [`names`],
//! the single module dashboards/alarms (CDK) assert against.
//!
//! # Design notes
//!
//! - **No wall-clock reads** (§22.11): EMF timestamps are passed in as
//!   [`TimestampMillis`], obtained by the caller from its injected `Clock`.
//! - **EMF histograms** are arrays of raw observations (capped at 100 per
//!   flush) because the published EMF specification represents distributions
//!   as value arrays; **EMF counters** are per-flush deltas so `CloudWatch`
//!   `Sum` aggregation is correct.
//! - **Wiring** metrics into CA service components belongs to ticket
//!   obs-service-instrumentation; dashboards/alarms to
//!   obs-cw-dashboards-slo-alarms.
//!
//! # Example
//!
//! ```
//! use mtc_metrics::{CaMetrics, MetricsRegistry};
//!
//! # fn main() -> Result<(), mtc_metrics::MetricsError> {
//! let registry = MetricsRegistry::new();
//! let metrics = CaMetrics::register(&registry)?;
//!
//! metrics.batches_committed_total.inc();
//! metrics.issuance_latency_seconds.observe(0.42);
//! metrics.entries_by_source_total.inc("acme");
//!
//! let text = registry.encode_prometheus_text()?;
//! assert!(text.contains("batches_committed_total 1"));
//! assert!(text.contains("entries_by_source_total{source_type=\"acme\"} 1"));
//! # Ok(())
//! # }
//! ```

pub mod admin;
pub mod emf;
mod error;
pub mod names;
mod registry;

pub use admin::{serve_admin, AdminServer, DEFAULT_ADMIN_PORT, PROMETHEUS_CONTENT_TYPE};
pub use emf::{EmfConfig, EmfEmitter, TimestampMillis, DEFAULT_EMF_NAMESPACE};
pub use error::MetricsError;
pub use registry::{
    CaMetrics, Counter, Gauge, Histogram, LabeledCounter, MetricsRegistry, DEFAULT_LATENCY_BUCKETS,
};
