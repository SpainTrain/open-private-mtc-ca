//! Metric-name constants for the spec §20.1 metric set.
//!
//! This is the **single module** that owns the metric names: dashboards and
//! alarms (CDK, ticket obs-cw-dashboards-slo-alarms) assert their widget and
//! alarm metric names against these constants, and the exporters derive EMF
//! units from [`SPEC_METRICS`]. Names are copied verbatim from the §20.1
//! metrics table in `docs/mtc-architecture-spec.md` — do not edit them without
//! a spec change.

/// The instrument kind of a §20.1 metric, exactly as listed in the spec table.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum InstrumentKind {
    /// Latency distribution (`prometheus` histogram).
    Histogram,
    /// Monotonic count (`prometheus` counter).
    Counter,
    /// Point-in-time value (`prometheus` gauge).
    Gauge,
}

/// Static description of one §20.1 metric: name, kind, labels, EMF unit, help.
#[derive(Copy, Clone, Debug)]
pub struct MetricSpec {
    /// Exact metric name from the §20.1 table.
    pub name: &'static str,
    /// Instrument kind from the §20.1 table.
    pub kind: InstrumentKind,
    /// Prometheus label names (empty for unlabeled metrics).
    pub labels: &'static [&'static str],
    /// CloudWatch unit emitted by the EMF exporter (a valid EMF `Unit` value).
    pub unit: &'static str,
    /// Prometheus `# HELP` text.
    pub help: &'static str,
}

/// End-to-end certificate issuance latency (histogram, seconds).
pub const ISSUANCE_LATENCY_SECONDS: &str = "issuance_latency_seconds";
/// Batch commit latency (histogram, seconds).
pub const BATCH_COMMIT_LATENCY_SECONDS: &str = "batch_commit_latency_seconds";
/// HSM signing latency (histogram, seconds).
pub const HSM_SIGNING_LATENCY_SECONDS: &str = "hsm_signing_latency_seconds";
/// Lease renewals (counter).
pub const LEASE_RENEWALS_TOTAL: &str = "lease_renewals_total";
/// Failed lease renewals (counter).
pub const LEASE_RENEWALS_FAILED_TOTAL: &str = "lease_renewals_failed_total";
/// Epoch advances (counter).
pub const EPOCH_ADVANCES_TOTAL: &str = "epoch_advances_total";
/// Batches committed (counter).
pub const BATCHES_COMMITTED_TOTAL: &str = "batches_committed_total";
/// Batches abandoned (counter).
pub const BATCHES_ABANDONED_TOTAL: &str = "batches_abandoned_total";
/// Entries ingested, labeled by source type (counter, `source_type` label).
pub const ENTRIES_BY_SOURCE_TOTAL: &str = "entries_by_source_total";
/// Tile cache hits (counter).
pub const TILE_CACHE_HITS_TOTAL: &str = "tile_cache_hits_total";
/// Tile cache misses (counter).
pub const TILE_CACHE_MISSES_TOTAL: &str = "tile_cache_misses_total";
/// S3 cross-region replication lag (gauge, seconds).
pub const CRR_REPLICATION_LAG_SECONDS: &str = "crr_replication_lag_seconds";
/// DynamoDB global-table replication lag (gauge, seconds).
pub const DDB_REPLICATION_LAG_SECONDS: &str = "ddb_replication_lag_seconds";

/// Label on [`ENTRIES_BY_SOURCE_TOTAL`] identifying the intake source (§10).
pub const LABEL_SOURCE_TYPE: &str = "source_type";

/// The full §20.1 metric set, in spec-table order.
pub const SPEC_METRICS: [MetricSpec; 13] = [
    MetricSpec {
        name: ISSUANCE_LATENCY_SECONDS,
        kind: InstrumentKind::Histogram,
        labels: &[],
        unit: "Seconds",
        help: "End-to-end certificate issuance latency in seconds.",
    },
    MetricSpec {
        name: BATCH_COMMIT_LATENCY_SECONDS,
        kind: InstrumentKind::Histogram,
        labels: &[],
        unit: "Seconds",
        help: "Batch commit latency in seconds.",
    },
    MetricSpec {
        name: HSM_SIGNING_LATENCY_SECONDS,
        kind: InstrumentKind::Histogram,
        labels: &[],
        unit: "Seconds",
        help: "HSM signing latency in seconds.",
    },
    MetricSpec {
        name: LEASE_RENEWALS_TOTAL,
        kind: InstrumentKind::Counter,
        labels: &[],
        unit: "Count",
        help: "Total lease renewals.",
    },
    MetricSpec {
        name: LEASE_RENEWALS_FAILED_TOTAL,
        kind: InstrumentKind::Counter,
        labels: &[],
        unit: "Count",
        help: "Total failed lease renewals.",
    },
    MetricSpec {
        name: EPOCH_ADVANCES_TOTAL,
        kind: InstrumentKind::Counter,
        labels: &[],
        unit: "Count",
        help: "Total epoch advances.",
    },
    MetricSpec {
        name: BATCHES_COMMITTED_TOTAL,
        kind: InstrumentKind::Counter,
        labels: &[],
        unit: "Count",
        help: "Total batches committed to the log.",
    },
    MetricSpec {
        name: BATCHES_ABANDONED_TOTAL,
        kind: InstrumentKind::Counter,
        labels: &[],
        unit: "Count",
        help: "Total batches abandoned before commit.",
    },
    MetricSpec {
        name: ENTRIES_BY_SOURCE_TOTAL,
        kind: InstrumentKind::Counter,
        labels: &[LABEL_SOURCE_TYPE],
        unit: "Count",
        help: "Total log entries ingested, labeled by source type.",
    },
    MetricSpec {
        name: TILE_CACHE_HITS_TOTAL,
        kind: InstrumentKind::Counter,
        labels: &[],
        unit: "Count",
        help: "Total tile cache hits.",
    },
    MetricSpec {
        name: TILE_CACHE_MISSES_TOTAL,
        kind: InstrumentKind::Counter,
        labels: &[],
        unit: "Count",
        help: "Total tile cache misses.",
    },
    MetricSpec {
        name: CRR_REPLICATION_LAG_SECONDS,
        kind: InstrumentKind::Gauge,
        labels: &[],
        unit: "Seconds",
        help: "S3 cross-region replication lag in seconds.",
    },
    MetricSpec {
        name: DDB_REPLICATION_LAG_SECONDS,
        kind: InstrumentKind::Gauge,
        labels: &[],
        unit: "Seconds",
        help: "DynamoDB global-table replication lag in seconds.",
    },
];

/// All 13 §20.1 metric names, in spec-table order.
pub const ALL_METRIC_NAMES: [&str; 13] = [
    ISSUANCE_LATENCY_SECONDS,
    BATCH_COMMIT_LATENCY_SECONDS,
    HSM_SIGNING_LATENCY_SECONDS,
    LEASE_RENEWALS_TOTAL,
    LEASE_RENEWALS_FAILED_TOTAL,
    EPOCH_ADVANCES_TOTAL,
    BATCHES_COMMITTED_TOTAL,
    BATCHES_ABANDONED_TOTAL,
    ENTRIES_BY_SOURCE_TOTAL,
    TILE_CACHE_HITS_TOTAL,
    TILE_CACHE_MISSES_TOTAL,
    CRR_REPLICATION_LAG_SECONDS,
    DDB_REPLICATION_LAG_SECONDS,
];

/// Looks up the [`MetricSpec`] for a §20.1 metric name.
#[must_use]
pub fn spec_for(name: &str) -> Option<&'static MetricSpec> {
    SPEC_METRICS.iter().find(|spec| spec.name == name)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// The names and kinds below are transcribed independently from the
    /// §20.1 metrics table; this test fails if the exported constants ever
    /// drift from the spec.
    #[test]
    fn names_and_kinds_match_spec_table_exactly() {
        let expected: [(&str, InstrumentKind); 13] = [
            ("issuance_latency_seconds", InstrumentKind::Histogram),
            ("batch_commit_latency_seconds", InstrumentKind::Histogram),
            ("hsm_signing_latency_seconds", InstrumentKind::Histogram),
            ("lease_renewals_total", InstrumentKind::Counter),
            ("lease_renewals_failed_total", InstrumentKind::Counter),
            ("epoch_advances_total", InstrumentKind::Counter),
            ("batches_committed_total", InstrumentKind::Counter),
            ("batches_abandoned_total", InstrumentKind::Counter),
            ("entries_by_source_total", InstrumentKind::Counter),
            ("tile_cache_hits_total", InstrumentKind::Counter),
            ("tile_cache_misses_total", InstrumentKind::Counter),
            ("crr_replication_lag_seconds", InstrumentKind::Gauge),
            ("ddb_replication_lag_seconds", InstrumentKind::Gauge),
        ];
        assert_eq!(SPEC_METRICS.len(), expected.len());
        for (spec, (name, kind)) in SPEC_METRICS.iter().zip(expected) {
            assert_eq!(spec.name, name);
            assert_eq!(spec.kind, kind);
        }
        assert_eq!(ALL_METRIC_NAMES, expected.map(|(name, _)| name));
    }

    #[test]
    fn entries_by_source_is_the_only_labeled_metric() {
        for spec in &SPEC_METRICS {
            if spec.name == ENTRIES_BY_SOURCE_TOTAL {
                assert_eq!(spec.labels, &[LABEL_SOURCE_TYPE]);
            } else {
                assert!(spec.labels.is_empty(), "{} must be unlabeled", spec.name);
            }
        }
    }

    #[test]
    fn spec_for_finds_every_name() {
        for name in ALL_METRIC_NAMES {
            assert!(spec_for(name).is_some());
        }
        assert!(spec_for("no_such_metric").is_none());
    }
}
