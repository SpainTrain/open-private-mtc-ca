//! CloudWatch Embedded Metric Format (EMF) emission.
//!
//! EMF is **structured JSON logging**: each flush produces self-describing
//! JSON log events whose `_aws` metadata tells CloudWatch which root members
//! to extract as metrics. The events are plain lines written to whatever log
//! sink the service uses (stdout under Fargate/Lambda log drivers); the log
//! pipeline — LocalStack CloudWatch Logs in dev (§1: no real AWS spend) —
//! ingests them. No AWS SDK types are involved anywhere (§22.8): this module
//! only builds `serde_json` values.
//!
//! Semantics per instrument kind:
//! - **Counters** are emitted as *per-flush deltas*, so CloudWatch `Sum`
//!   aggregation over a period equals the true count.
//! - **Gauges** are emitted as their current value.
//! - **Histograms** are emitted as arrays of the raw observations recorded
//!   since the previous flush (the published EMF format represents
//!   distributions as value arrays, capped at 100 values per event).
//!
//! Labeled metrics (e.g. `entries_by_source_total{source_type}`) become
//! separate events whose label is an extra CloudWatch dimension.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;

use serde_json::{json, Map, Value};

use crate::error::MetricsError;
use crate::names;
use crate::registry::MetricsRegistry;

/// Milliseconds since the Unix epoch (the EMF `Timestamp` member).
///
/// A newtype (§22.1). This crate deliberately never reads wall-clock time:
/// callers obtain the timestamp from their injected `Clock` (§22.11) and pass
/// it in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimestampMillis(pub u64);

/// Namespace used when an enabled [`EmfConfig`] leaves `namespace` empty.
pub const DEFAULT_EMF_NAMESPACE: &str = "MtcCa";

/// Configuration gate for the EMF exporter.
///
/// EMF emission is off unless a config with `enabled: true` is provided
/// (Prometheus exposition is unaffected either way).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EmfConfig {
    /// Master switch; when `false`, [`EmfEmitter::from_config`] yields `None`.
    pub enabled: bool,
    /// CloudWatch metrics namespace (falls back to [`DEFAULT_EMF_NAMESPACE`]).
    pub namespace: String,
    /// Dimensions stamped on every event, e.g. `service` and `region`
    /// (§20.1 standard fields).
    #[serde(default)]
    pub default_dimensions: BTreeMap<String, String>,
}

impl EmfConfig {
    /// A disabled configuration (the default).
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            namespace: String::new(),
            default_dimensions: BTreeMap::new(),
        }
    }
}

impl Default for EmfConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

/// One metric sample destined for an EMF event.
struct Sample {
    name: String,
    unit: Option<&'static str>,
    value: Value,
}

/// CloudWatch EMF exporter over a [`MetricsRegistry`].
///
/// Stateful: tracks the previously seen counter values (for delta emission),
/// so a service should hold exactly one emitter per registry.
pub struct EmfEmitter {
    registry: MetricsRegistry,
    namespace: String,
    default_dimensions: BTreeMap<String, String>,
    last_counters: HashMap<String, f64>,
}

impl EmfEmitter {
    /// Builds an emitter from `config`, or `None` when EMF is disabled.
    #[must_use]
    pub fn from_config(registry: &MetricsRegistry, config: &EmfConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        let namespace = if config.namespace.is_empty() {
            DEFAULT_EMF_NAMESPACE.to_string()
        } else {
            config.namespace.clone()
        };
        Some(Self {
            registry: registry.clone(),
            namespace,
            default_dimensions: config.default_dimensions.clone(),
            last_counters: HashMap::new(),
        })
    }

    /// Produces the EMF log events for everything recorded since the last
    /// flush. `timestamp` comes from the caller's injected clock (§22.11).
    pub fn flush(&mut self, timestamp: TimestampMillis) -> Vec<Value> {
        use prometheus::proto::MetricType;

        // Group samples by their (sorted) label sets: the unlabeled base
        // event plus one event per distinct label-value combination.
        let mut groups: BTreeMap<Vec<(String, String)>, Vec<Sample>> = BTreeMap::new();

        for family in self.registry.families() {
            let name = family.name().to_string();
            let unit = unit_for(&name);
            match family.get_field_type() {
                MetricType::COUNTER => {
                    for metric in family.get_metric() {
                        let mut labels: Vec<(String, String)> = metric
                            .get_label()
                            .iter()
                            .map(|pair| (pair.name().to_string(), pair.value().to_string()))
                            .collect();
                        labels.sort();
                        let current = metric.get_counter().get_value();
                        let key = sample_key(&name, &labels);
                        let previous = self.last_counters.insert(key, current).unwrap_or(0.0);
                        let delta = (current - previous).max(0.0);
                        groups.entry(labels).or_default().push(Sample {
                            name: name.clone(),
                            unit,
                            value: json_number(delta),
                        });
                    }
                }
                MetricType::GAUGE => {
                    for metric in family.get_metric() {
                        let mut labels: Vec<(String, String)> = metric
                            .get_label()
                            .iter()
                            .map(|pair| (pair.name().to_string(), pair.value().to_string()))
                            .collect();
                        labels.sort();
                        groups.entry(labels).or_default().push(Sample {
                            name: name.clone(),
                            unit,
                            value: json_number(metric.get_gauge().get_value()),
                        });
                    }
                }
                // Histograms are emitted from the raw-observation buffers
                // below; other kinds are not part of the §20.1 set.
                _ => {}
            }
        }

        for (name, raw) in self.registry.raw_histogram_handles() {
            let values = raw.drain();
            if values.is_empty() {
                continue;
            }
            let array: Vec<Value> = values.into_iter().map(json_number).collect();
            groups.entry(Vec::new()).or_default().push(Sample {
                name: name.to_string(),
                unit: unit_for(name),
                value: Value::Array(array),
            });
        }

        let mut events = Vec::new();
        for (labels, samples) in groups {
            if samples.is_empty() {
                continue;
            }
            events.push(self.build_event(timestamp, &labels, &samples));
        }
        events
    }

    /// Flushes as newline-delimited JSON to `writer` (one event per line),
    /// the framing CloudWatch Logs agents and LocalStack ingest directly.
    pub fn flush_to<W: Write>(
        &mut self,
        writer: &mut W,
        timestamp: TimestampMillis,
    ) -> Result<(), MetricsError> {
        for event in self.flush(timestamp) {
            serde_json::to_writer(&mut *writer, &event)
                .map_err(|e| MetricsError::Emf(e.to_string()))?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    }

    fn build_event(
        &self,
        timestamp: TimestampMillis,
        labels: &[(String, String)],
        samples: &[Sample],
    ) -> Value {
        let mut dimension_set: BTreeSet<String> = self.default_dimensions.keys().cloned().collect();
        dimension_set.extend(labels.iter().map(|(key, _)| key.clone()));
        let dimension_refs: Vec<String> = dimension_set.into_iter().collect();

        let metric_definitions: Vec<Value> = samples
            .iter()
            .map(|sample| {
                let mut definition = Map::new();
                definition.insert("Name".to_string(), Value::String(sample.name.clone()));
                if let Some(unit) = sample.unit {
                    definition.insert("Unit".to_string(), Value::String(unit.to_string()));
                }
                Value::Object(definition)
            })
            .collect();

        let mut root = Map::new();
        root.insert(
            "_aws".to_string(),
            json!({
                "Timestamp": timestamp.0,
                "CloudWatchMetrics": [{
                    "Namespace": self.namespace,
                    "Dimensions": [dimension_refs],
                    "Metrics": metric_definitions,
                }],
            }),
        );
        for (key, value) in &self.default_dimensions {
            root.insert(key.clone(), Value::String(value.clone()));
        }
        for (key, value) in labels {
            root.insert(key.clone(), Value::String(value.clone()));
        }
        for sample in samples {
            root.insert(sample.name.clone(), sample.value.clone());
        }
        Value::Object(root)
    }
}

/// CloudWatch unit for a §20.1 metric name (`None` for unknown names).
fn unit_for(name: &str) -> Option<&'static str> {
    names::spec_for(name).map(|spec| spec.unit)
}

/// Stable identity for a counter time series across flushes.
fn sample_key(name: &str, labels: &[(String, String)]) -> String {
    let mut key = String::from(name);
    for (label, value) in labels {
        key.push('\u{1f}');
        key.push_str(label);
        key.push('=');
        key.push_str(value);
    }
    key
}

/// Renders `value` as a JSON number, preferring integer representation for
/// whole values (counters). Non-finite values collapse to 0 — CloudWatch
/// rejects NaN/Inf, and no §20.1 instrument can legitimately produce them.
fn json_number(value: f64) -> Value {
    #[allow(clippy::cast_possible_truncation)]
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        Value::from(value as i64)
    } else {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or_else(|| Value::from(0))
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::registry::CaMetrics;

    fn config() -> EmfConfig {
        EmfConfig {
            enabled: true,
            namespace: "MtcCaTest".to_string(),
            default_dimensions: BTreeMap::from([("service".to_string(), "mtc-ca".to_string())]),
        }
    }

    fn setup() -> (MetricsRegistry, CaMetrics, EmfEmitter) {
        let registry = MetricsRegistry::new();
        let metrics = CaMetrics::register(&registry).expect("registration succeeds");
        let emitter = EmfEmitter::from_config(&registry, &config()).expect("enabled");
        (registry, metrics, emitter)
    }

    /// Finds the event that carries root member `key`.
    fn event_with<'e>(events: &'e [Value], key: &str) -> &'e Value {
        events
            .iter()
            .find(|event| event.get(key).is_some())
            .unwrap_or_else(|| panic!("no event with root member {key}"))
    }

    #[test]
    fn disabled_config_yields_no_emitter() {
        let registry = MetricsRegistry::new();
        assert!(EmfEmitter::from_config(&registry, &EmfConfig::disabled()).is_none());
        assert!(EmfEmitter::from_config(&registry, &EmfConfig::default()).is_none());
    }

    #[test]
    fn empty_namespace_falls_back_to_default() {
        let registry = MetricsRegistry::new();
        let _metrics = CaMetrics::register(&registry).expect("registers");
        let mut emitter = EmfEmitter::from_config(
            &registry,
            &EmfConfig {
                enabled: true,
                namespace: String::new(),
                default_dimensions: BTreeMap::new(),
            },
        )
        .expect("enabled");
        let events = emitter.flush(TimestampMillis(1));
        let event = event_with(&events, "batches_committed_total");
        assert_eq!(
            event["_aws"]["CloudWatchMetrics"][0]["Namespace"],
            json!(DEFAULT_EMF_NAMESPACE)
        );
    }

    #[test]
    fn base_event_has_emf_metadata_and_values() {
        let (_registry, metrics, mut emitter) = setup();
        metrics.batches_committed_total.inc_by(3);
        metrics.crr_replication_lag_seconds.set(0.75);
        let events = emitter.flush(TimestampMillis(1_700_000_000_000));
        let event = event_with(&events, "batches_committed_total");

        assert_eq!(event["_aws"]["Timestamp"], json!(1_700_000_000_000_u64));
        let directive = &event["_aws"]["CloudWatchMetrics"][0];
        assert_eq!(directive["Namespace"], json!("MtcCaTest"));
        assert_eq!(directive["Dimensions"], json!([["service"]]));
        assert_eq!(event["service"], json!("mtc-ca"));
        assert_eq!(event["batches_committed_total"], json!(3));
        assert_eq!(event["crr_replication_lag_seconds"], json!(0.75));

        let declared: Vec<&str> = directive["Metrics"]
            .as_array()
            .expect("Metrics array")
            .iter()
            .map(|m| m["Name"].as_str().expect("Name string"))
            .collect();
        assert!(declared.contains(&"batches_committed_total"));
        assert!(declared.contains(&"crr_replication_lag_seconds"));
    }

    #[test]
    fn counters_emit_per_flush_deltas() {
        let (_registry, metrics, mut emitter) = setup();
        metrics.lease_renewals_total.inc_by(5);
        let first = emitter.flush(TimestampMillis(1));
        assert_eq!(
            event_with(&first, "lease_renewals_total")["lease_renewals_total"],
            json!(5)
        );

        metrics.lease_renewals_total.inc_by(2);
        let second = emitter.flush(TimestampMillis(2));
        assert_eq!(
            event_with(&second, "lease_renewals_total")["lease_renewals_total"],
            json!(2)
        );

        let third = emitter.flush(TimestampMillis(3));
        assert_eq!(
            event_with(&third, "lease_renewals_total")["lease_renewals_total"],
            json!(0)
        );
    }

    #[test]
    fn histograms_emit_raw_value_arrays_and_drain() {
        let (_registry, metrics, mut emitter) = setup();
        metrics.issuance_latency_seconds.observe(0.25);
        metrics.issuance_latency_seconds.observe(1.5);
        metrics.issuance_latency_seconds.observe(4.0);

        let events = emitter.flush(TimestampMillis(1));
        let event = event_with(&events, "issuance_latency_seconds");
        assert_eq!(event["issuance_latency_seconds"], json!([0.25, 1.5, 4]));
        let directive = &event["_aws"]["CloudWatchMetrics"][0];
        let issuance = directive["Metrics"]
            .as_array()
            .expect("Metrics array")
            .iter()
            .find(|m| m["Name"] == json!("issuance_latency_seconds"))
            .expect("issuance metric declared");
        assert_eq!(issuance["Unit"], json!("Seconds"));

        // Drained: the next flush carries no histogram member.
        let next = emitter.flush(TimestampMillis(2));
        for event in &next {
            assert!(event.get("issuance_latency_seconds").is_none());
        }
    }

    #[test]
    fn labeled_counters_become_dimensioned_events() {
        let (_registry, metrics, mut emitter) = setup();
        metrics.entries_by_source_total.inc_by("acme", 4);
        let events = emitter.flush(TimestampMillis(1));
        let event = event_with(&events, "source_type");

        assert_eq!(event["source_type"], json!("acme"));
        assert_eq!(event["entries_by_source_total"], json!(4));
        let directive = &event["_aws"]["CloudWatchMetrics"][0];
        assert_eq!(directive["Dimensions"], json!([["service", "source_type"]]));
        assert_eq!(event["service"], json!("mtc-ca"));
    }

    #[test]
    fn counter_units_are_count_and_gauge_units_are_seconds() {
        let (_registry, metrics, mut emitter) = setup();
        metrics.tile_cache_hits_total.inc();
        metrics.ddb_replication_lag_seconds.set(0.1);
        let events = emitter.flush(TimestampMillis(1));
        let directive =
            &event_with(&events, "tile_cache_hits_total")["_aws"]["CloudWatchMetrics"][0];
        let unit_of = |name: &str| {
            directive["Metrics"]
                .as_array()
                .expect("Metrics array")
                .iter()
                .find(|m| m["Name"] == json!(name))
                .map(|m| m["Unit"].clone())
                .expect("metric declared")
        };
        assert_eq!(unit_of("tile_cache_hits_total"), json!("Count"));
        assert_eq!(unit_of("ddb_replication_lag_seconds"), json!("Seconds"));
    }

    #[test]
    fn flush_to_writes_one_json_event_per_line() {
        let (_registry, metrics, mut emitter) = setup();
        metrics.batches_committed_total.inc();
        metrics.entries_by_source_total.inc("acme");
        let mut sink = Vec::new();
        emitter
            .flush_to(&mut sink, TimestampMillis(7))
            .expect("writes");
        let text = String::from_utf8(sink).expect("utf8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "base event + one labeled event: {text}");
        for line in lines {
            let event: Value = serde_json::from_str(line).expect("valid JSON line");
            assert_eq!(event["_aws"]["Timestamp"], json!(7));
        }
    }

    #[test]
    fn json_number_prefers_integers_and_rejects_non_finite() {
        assert_eq!(json_number(3.0), json!(3));
        assert_eq!(json_number(0.25), json!(0.25));
        assert_eq!(json_number(f64::NAN), json!(0));
        assert_eq!(json_number(f64::INFINITY), json!(0));
    }
}
