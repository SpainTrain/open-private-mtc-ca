//! Metric registry and typed instrument handles.
//!
//! [`MetricsRegistry`] is the single registration point backing both
//! exporters (Prometheus text exposition and `CloudWatch` EMF). The typed
//! handles ([`Counter`], [`Gauge`], [`Histogram`], [`LabeledCounter`]) keep
//! the backing implementation crate out of the public API, and
//! [`CaMetrics::register`] constructs the full §20.1 metric set with the
//! exact spec names and instrument kinds.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::error::MetricsError;
use crate::names;

/// Default histogram buckets for the §20.1 latency metrics, in seconds.
///
/// Spans sub-10ms HSM signings up to the 10s issuance-latency SLO (§20.1)
/// with headroom for degraded operation.
pub const DEFAULT_LATENCY_BUCKETS: [f64; 13] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

/// Maximum raw observations buffered per histogram between EMF flushes.
///
/// Matches the EMF limit of 100 values per metric in one log event.
/// Observations beyond the cap are dropped from EMF output only — the
/// Prometheus histogram still records every observation.
const MAX_RAW_OBSERVATIONS: usize = 100;

/// Raw histogram observations retained for the EMF exporter, which emits
/// value arrays (the published EMF format has no bucketed representation).
#[derive(Debug, Default)]
pub struct RawObservations {
    values: Mutex<Vec<f64>>,
}

impl RawObservations {
    fn record(&self, value: f64) {
        let mut guard = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.len() < MAX_RAW_OBSERVATIONS {
            guard.push(value);
        }
    }

    /// Takes and clears the buffered observations.
    pub(crate) fn drain(&self) -> Vec<f64> {
        std::mem::take(&mut *self.values.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Shared metric registry backing both the Prometheus and EMF exporters.
///
/// Cheap to clone: clones share the same underlying registry state.
#[derive(Clone, Default)]
pub struct MetricsRegistry {
    inner: prometheus::Registry,
    raw_histograms: Arc<Mutex<BTreeMap<&'static str, Arc<RawObservations>>>>,
}

fn registration_error(error: &prometheus::Error) -> MetricsError {
    MetricsError::Registration(error.to_string())
}

impl MetricsRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a monotonic counter.
    ///
    /// # Errors
    /// Returns `MetricsError` if a metric with this name is already
    /// registered with a different kind or help text.
    pub fn counter(&self, name: &'static str, help: &'static str) -> Result<Counter, MetricsError> {
        let inner = prometheus::IntCounter::new(name, help).map_err(|e| registration_error(&e))?;
        self.inner
            .register(Box::new(inner.clone()))
            .map_err(|e| registration_error(&e))?;
        Ok(Counter { inner })
    }

    /// Registers a labeled monotonic counter with a single label dimension.
    ///
    /// # Errors
    /// Returns `MetricsError` if a metric with this name is already
    /// registered with a different kind, labels, or help text.
    pub fn labeled_counter(
        &self,
        name: &'static str,
        help: &'static str,
        label: &'static str,
    ) -> Result<LabeledCounter, MetricsError> {
        let inner = prometheus::IntCounterVec::new(prometheus::Opts::new(name, help), &[label])
            .map_err(|e| registration_error(&e))?;
        self.inner
            .register(Box::new(inner.clone()))
            .map_err(|e| registration_error(&e))?;
        Ok(LabeledCounter { inner })
    }

    /// Registers a gauge.
    ///
    /// # Errors
    /// Returns `MetricsError` if a metric with this name is already
    /// registered with a different kind or help text.
    pub fn gauge(&self, name: &'static str, help: &'static str) -> Result<Gauge, MetricsError> {
        let inner = prometheus::Gauge::new(name, help).map_err(|e| registration_error(&e))?;
        self.inner
            .register(Box::new(inner.clone()))
            .map_err(|e| registration_error(&e))?;
        Ok(Gauge { inner })
    }

    /// Registers a histogram with the given bucket upper bounds.
    ///
    /// # Errors
    /// Returns `MetricsError` if a metric with this name is already
    /// registered with a different kind, buckets, or help text.
    pub fn histogram(
        &self,
        name: &'static str,
        help: &'static str,
        buckets: &[f64],
    ) -> Result<Histogram, MetricsError> {
        let opts = prometheus::HistogramOpts::new(name, help).buckets(buckets.to_vec());
        let inner = prometheus::Histogram::with_opts(opts).map_err(|e| registration_error(&e))?;
        self.inner
            .register(Box::new(inner.clone()))
            .map_err(|e| registration_error(&e))?;
        let raw = Arc::new(RawObservations::default());
        self.raw_histograms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name, Arc::clone(&raw));
        Ok(Histogram { inner, raw })
    }

    /// Encodes the current state of every registered metric in the
    /// Prometheus text exposition format (version 0.0.4).
    ///
    /// # Errors
    /// Returns `MetricsError` if the underlying encoder fails or the
    /// output is not valid UTF-8.
    pub fn encode_prometheus_text(&self) -> Result<String, MetricsError> {
        use prometheus::Encoder as _;

        let families = self.inner.gather();
        let mut buffer = Vec::new();
        prometheus::TextEncoder::new()
            .encode(&families, &mut buffer)
            .map_err(|e| MetricsError::Encode(e.to_string()))?;
        String::from_utf8(buffer).map_err(|e| MetricsError::Encode(e.to_string()))
    }

    /// Snapshot of all registered metric families (for the EMF exporter).
    pub(crate) fn families(&self) -> Vec<prometheus::proto::MetricFamily> {
        self.inner.gather()
    }

    /// Handles to the raw-observation buffers of every registered histogram.
    pub(crate) fn raw_histogram_handles(&self) -> Vec<(&'static str, Arc<RawObservations>)> {
        self.raw_histograms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(name, raw)| (*name, Arc::clone(raw)))
            .collect()
    }
}

/// Monotonic counter handle.
#[derive(Clone)]
pub struct Counter {
    inner: prometheus::IntCounter,
}

impl Counter {
    /// Increments the counter by one.
    pub fn inc(&self) {
        self.inner.inc();
    }

    /// Increments the counter by `n`.
    pub fn inc_by(&self, n: u64) {
        self.inner.inc_by(n);
    }

    /// Current counter value.
    #[must_use]
    pub fn value(&self) -> u64 {
        self.inner.get()
    }
}

/// Labeled monotonic counter handle (one label dimension).
///
/// Standard Prometheus vec semantics: the family appears in exposition once
/// at least one label value has been recorded.
#[derive(Clone)]
pub struct LabeledCounter {
    inner: prometheus::IntCounterVec,
}

impl LabeledCounter {
    /// Increments the counter for `label_value` by one.
    pub fn inc(&self, label_value: &str) {
        self.inner.with_label_values(&[label_value]).inc();
    }

    /// Increments the counter for `label_value` by `n`.
    pub fn inc_by(&self, label_value: &str, n: u64) {
        self.inner.with_label_values(&[label_value]).inc_by(n);
    }

    /// Current counter value for `label_value`.
    #[must_use]
    pub fn value(&self, label_value: &str) -> u64 {
        self.inner.with_label_values(&[label_value]).get()
    }
}

/// Gauge handle (point-in-time value).
#[derive(Clone)]
pub struct Gauge {
    inner: prometheus::Gauge,
}

impl Gauge {
    /// Sets the gauge to `value`.
    pub fn set(&self, value: f64) {
        self.inner.set(value);
    }

    /// Current gauge value.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.inner.get()
    }
}

/// Histogram handle (latency distribution).
#[derive(Clone)]
pub struct Histogram {
    inner: prometheus::Histogram,
    raw: Arc<RawObservations>,
}

impl Histogram {
    /// Records one observation (in the metric's base unit — seconds for the
    /// §20.1 latency metrics).
    pub fn observe(&self, value: f64) {
        self.inner.observe(value);
        self.raw.record(value);
    }
}

/// The full §20.1 metric set with typed, correctly-kinded handles.
///
/// Constructed by [`CaMetrics::register`]; the metric names come verbatim
/// from [`crate::names`], so call sites cannot misname or mis-kind a metric.
/// Cheap to clone — handles share state with the registry.
#[derive(Clone)]
pub struct CaMetrics {
    /// `issuance_latency_seconds` — histogram (§20.1).
    pub issuance_latency_seconds: Histogram,
    /// `batch_commit_latency_seconds` — histogram (§20.1).
    pub batch_commit_latency_seconds: Histogram,
    /// `hsm_signing_latency_seconds` — histogram (§20.1).
    pub hsm_signing_latency_seconds: Histogram,
    /// `lease_renewals_total` — counter (§20.1).
    pub lease_renewals_total: Counter,
    /// `lease_renewals_failed_total` — counter (§20.1).
    pub lease_renewals_failed_total: Counter,
    /// `epoch_advances_total` — counter (§20.1).
    pub epoch_advances_total: Counter,
    /// `batches_committed_total` — counter (§20.1).
    pub batches_committed_total: Counter,
    /// `batches_abandoned_total` — counter (§20.1).
    pub batches_abandoned_total: Counter,
    /// `entries_by_source_total{source_type}` — labeled counter (§20.1, §10).
    pub entries_by_source_total: LabeledCounter,
    /// `tile_cache_hits_total` — counter (§20.1).
    pub tile_cache_hits_total: Counter,
    /// `tile_cache_misses_total` — counter (§20.1).
    pub tile_cache_misses_total: Counter,
    /// `crr_replication_lag_seconds` — gauge (§20.1).
    pub crr_replication_lag_seconds: Gauge,
    /// `ddb_replication_lag_seconds` — gauge (§20.1).
    pub ddb_replication_lag_seconds: Gauge,
}

fn help_for(name: &'static str) -> &'static str {
    names::spec_for(name).map_or(name, |spec| spec.help)
}

impl CaMetrics {
    /// Registers all 13 §20.1 metrics on `registry` and returns the typed set.
    ///
    /// # Errors
    ///
    /// Returns [`MetricsError::Registration`] if any metric is already
    /// registered (the set can only be registered once per registry).
    pub fn register(registry: &MetricsRegistry) -> Result<Self, MetricsError> {
        Ok(Self {
            issuance_latency_seconds: registry.histogram(
                names::ISSUANCE_LATENCY_SECONDS,
                help_for(names::ISSUANCE_LATENCY_SECONDS),
                &DEFAULT_LATENCY_BUCKETS,
            )?,
            batch_commit_latency_seconds: registry.histogram(
                names::BATCH_COMMIT_LATENCY_SECONDS,
                help_for(names::BATCH_COMMIT_LATENCY_SECONDS),
                &DEFAULT_LATENCY_BUCKETS,
            )?,
            hsm_signing_latency_seconds: registry.histogram(
                names::HSM_SIGNING_LATENCY_SECONDS,
                help_for(names::HSM_SIGNING_LATENCY_SECONDS),
                &DEFAULT_LATENCY_BUCKETS,
            )?,
            lease_renewals_total: registry.counter(
                names::LEASE_RENEWALS_TOTAL,
                help_for(names::LEASE_RENEWALS_TOTAL),
            )?,
            lease_renewals_failed_total: registry.counter(
                names::LEASE_RENEWALS_FAILED_TOTAL,
                help_for(names::LEASE_RENEWALS_FAILED_TOTAL),
            )?,
            epoch_advances_total: registry.counter(
                names::EPOCH_ADVANCES_TOTAL,
                help_for(names::EPOCH_ADVANCES_TOTAL),
            )?,
            batches_committed_total: registry.counter(
                names::BATCHES_COMMITTED_TOTAL,
                help_for(names::BATCHES_COMMITTED_TOTAL),
            )?,
            batches_abandoned_total: registry.counter(
                names::BATCHES_ABANDONED_TOTAL,
                help_for(names::BATCHES_ABANDONED_TOTAL),
            )?,
            entries_by_source_total: registry.labeled_counter(
                names::ENTRIES_BY_SOURCE_TOTAL,
                help_for(names::ENTRIES_BY_SOURCE_TOTAL),
                names::LABEL_SOURCE_TYPE,
            )?,
            tile_cache_hits_total: registry.counter(
                names::TILE_CACHE_HITS_TOTAL,
                help_for(names::TILE_CACHE_HITS_TOTAL),
            )?,
            tile_cache_misses_total: registry.counter(
                names::TILE_CACHE_MISSES_TOTAL,
                help_for(names::TILE_CACHE_MISSES_TOTAL),
            )?,
            crr_replication_lag_seconds: registry.gauge(
                names::CRR_REPLICATION_LAG_SECONDS,
                help_for(names::CRR_REPLICATION_LAG_SECONDS),
            )?,
            ddb_replication_lag_seconds: registry.gauge(
                names::DDB_REPLICATION_LAG_SECONDS,
                help_for(names::DDB_REPLICATION_LAG_SECONDS),
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::names::{InstrumentKind, ALL_METRIC_NAMES, SPEC_METRICS};

    fn registered() -> (MetricsRegistry, CaMetrics) {
        let registry = MetricsRegistry::new();
        let metrics = CaMetrics::register(&registry).expect("registration succeeds");
        (registry, metrics)
    }

    #[test]
    fn registers_all_thirteen_families_with_exact_names() {
        let (registry, metrics) = registered();
        // Vec-metric families surface once a label value exists (standard
        // Prometheus semantics), so touch the labeled counter.
        metrics.entries_by_source_total.inc("acme");
        let mut family_names: Vec<String> = registry
            .families()
            .iter()
            .map(|family| family.name().to_string())
            .collect();
        family_names.sort();
        let mut expected: Vec<String> = ALL_METRIC_NAMES.iter().map(ToString::to_string).collect();
        expected.sort();
        assert_eq!(family_names, expected);
    }

    #[test]
    fn exposition_declares_correct_instrument_kind_per_metric() {
        let (registry, metrics) = registered();
        metrics.entries_by_source_total.inc("acme");
        let text = registry.encode_prometheus_text().expect("encodes");
        for spec in &SPEC_METRICS {
            let kind = match spec.kind {
                InstrumentKind::Histogram => "histogram",
                InstrumentKind::Counter => "counter",
                InstrumentKind::Gauge => "gauge",
            };
            let type_line = format!("# TYPE {} {kind}", spec.name);
            assert!(
                text.contains(&type_line),
                "missing `{type_line}` in exposition:\n{text}"
            );
        }
    }

    #[test]
    fn counter_register_record_export_round_trip() {
        let (registry, metrics) = registered();
        metrics.lease_renewals_total.inc();
        metrics.lease_renewals_total.inc_by(2);
        assert_eq!(metrics.lease_renewals_total.value(), 3);
        let text = registry.encode_prometheus_text().expect("encodes");
        assert!(text.contains("lease_renewals_total 3"), "{text}");
    }

    #[test]
    fn gauge_register_record_export_round_trip() {
        let (registry, metrics) = registered();
        metrics.crr_replication_lag_seconds.set(1.5);
        assert!((metrics.crr_replication_lag_seconds.value() - 1.5).abs() < f64::EPSILON);
        let text = registry.encode_prometheus_text().expect("encodes");
        assert!(text.contains("crr_replication_lag_seconds 1.5"), "{text}");
    }

    #[test]
    fn histogram_register_record_export_round_trip() {
        let (registry, metrics) = registered();
        metrics.issuance_latency_seconds.observe(0.05);
        metrics.issuance_latency_seconds.observe(3.0);
        let text = registry.encode_prometheus_text().expect("encodes");
        assert!(
            text.contains("issuance_latency_seconds_bucket{le=\"0.05\"} 1"),
            "{text}"
        );
        assert!(text.contains("issuance_latency_seconds_count 2"), "{text}");
        assert!(text.contains("issuance_latency_seconds_sum 3.05"), "{text}");
    }

    #[test]
    fn labeled_counter_register_record_export_round_trip() {
        let (registry, metrics) = registered();
        metrics.entries_by_source_total.inc("acme");
        metrics.entries_by_source_total.inc("acme");
        metrics.entries_by_source_total.inc_by("api", 5);
        assert_eq!(metrics.entries_by_source_total.value("acme"), 2);
        assert_eq!(metrics.entries_by_source_total.value("api"), 5);
        let text = registry.encode_prometheus_text().expect("encodes");
        assert!(
            text.contains("entries_by_source_total{source_type=\"acme\"} 2"),
            "{text}"
        );
        assert!(
            text.contains("entries_by_source_total{source_type=\"api\"} 5"),
            "{text}"
        );
    }

    #[test]
    fn duplicate_registration_is_an_error() {
        let (registry, _metrics) = registered();
        let second = CaMetrics::register(&registry);
        assert!(matches!(second, Err(MetricsError::Registration(_))));
    }

    #[test]
    fn raw_observations_cap_at_emf_limit() {
        let raw = RawObservations::default();
        for i in 0..250 {
            raw.record(f64::from(i));
        }
        let drained = raw.drain();
        assert_eq!(drained.len(), MAX_RAW_OBSERVATIONS);
        // Drained buffers start empty again.
        raw.record(1.0);
        assert_eq!(raw.drain(), vec![1.0]);
    }
}
