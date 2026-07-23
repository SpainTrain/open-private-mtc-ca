//! Validates emitted EMF events against the published EMF JSON schema.
//!
//! `emf-schema.json` is transcribed from the AWS `CloudWatch` documentation
//! page "Specification: Embedded metric format" (the published EMF JSON
//! schema). On top of schema validation, this suite checks the cross-member
//! rules the specification states in prose: every dimension reference and
//! every declared metric name must exist as a root-level member of the event,
//! with the right JSON type.

// Integration-test helpers sit outside #[test] fns, so the
// allow-expect-in-tests exemption does not reach them (documented
// scoped-allow pattern, docs/lint-policy.md deviation 1).
#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use mtc_metrics::{names, CaMetrics, EmfConfig, EmfEmitter, MetricsRegistry, TimestampMillis};
use serde_json::Value;

const TIMESTAMP: TimestampMillis = TimestampMillis(1_700_000_000_000);

/// Registers the full §20.1 set, records through every instrument kind
/// (including two label values for the labeled counter), and flushes.
fn sample_events() -> Vec<Value> {
    let registry = MetricsRegistry::new();
    let metrics = CaMetrics::register(&registry).expect("registration succeeds");

    metrics.issuance_latency_seconds.observe(1.25);
    metrics.batch_commit_latency_seconds.observe(0.5);
    metrics.hsm_signing_latency_seconds.observe(0.032);
    metrics.lease_renewals_total.inc();
    metrics.lease_renewals_failed_total.inc_by(2);
    metrics.epoch_advances_total.inc();
    metrics.batches_committed_total.inc_by(3);
    metrics.batches_abandoned_total.inc();
    metrics.entries_by_source_total.inc("acme");
    metrics.entries_by_source_total.inc_by("api", 7);
    metrics.tile_cache_hits_total.inc_by(10);
    metrics.tile_cache_misses_total.inc();
    metrics.crr_replication_lag_seconds.set(0.8);
    metrics.ddb_replication_lag_seconds.set(0.05);

    let config = EmfConfig {
        enabled: true,
        namespace: "MtcCa".to_string(),
        default_dimensions: BTreeMap::from([("service".to_string(), "mtc-ca".to_string())]),
    };
    let mut emitter = EmfEmitter::from_config(&registry, &config).expect("enabled config");
    emitter.flush(TIMESTAMP)
}

#[test]
fn every_event_validates_against_the_published_emf_schema() {
    let schema: Value =
        serde_json::from_str(include_str!("emf-schema.json")).expect("schema parses");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");

    let events = sample_events();
    assert!(!events.is_empty(), "flush must produce events");
    for event in &events {
        let errors: Vec<String> = validator
            .iter_errors(event)
            .map(|error| format!("{error} at {}", error.instance_path()))
            .collect();
        assert!(
            errors.is_empty(),
            "EMF event failed schema validation: {errors:#?}\nevent: {event:#}"
        );
    }
}

#[test]
fn dimension_references_and_metric_names_resolve_to_root_members() {
    for event in sample_events() {
        let root = event.as_object().expect("event is an object");
        let directives = event["_aws"]["CloudWatchMetrics"]
            .as_array()
            .expect("CloudWatchMetrics is an array");
        for directive in directives {
            // Prose rule: every dimension reference must name a root-level
            // string member.
            for set in directive["Dimensions"].as_array().expect("Dimensions") {
                let refs = set.as_array().expect("DimensionSet is an array");
                assert!(refs.len() <= 30, "at most 30 dimension references");
                for reference in refs {
                    let key = reference.as_str().expect("dimension ref is a string");
                    assert!(
                        root.get(key).is_some_and(Value::is_string),
                        "dimension `{key}` must be a root string member: {event:#}"
                    );
                }
            }
            // Prose rule: every declared metric must have a root-level member
            // that is a number or an array of at most 100 numbers.
            let metrics = directive["Metrics"].as_array().expect("Metrics");
            assert!(metrics.len() <= 100, "at most 100 metrics per event");
            for definition in metrics {
                let name = definition["Name"].as_str().expect("Name is a string");
                let value = root
                    .get(name)
                    .unwrap_or_else(|| panic!("metric `{name}` missing at root: {event:#}"));
                match value {
                    Value::Number(_) => {}
                    Value::Array(values) => {
                        assert!(
                            values.len() <= 100,
                            "metric `{name}` exceeds 100 values per event"
                        );
                        assert!(
                            values.iter().all(Value::is_number),
                            "metric `{name}` array must be all numbers"
                        );
                    }
                    other => panic!("metric `{name}` has non-numeric value {other:#}"),
                }
            }
        }
    }
}

#[test]
fn all_thirteen_spec_metrics_are_emitted() {
    let events = sample_events();
    let mut emitted: BTreeSet<&str> = BTreeSet::new();
    for event in &events {
        for directive in event["_aws"]["CloudWatchMetrics"]
            .as_array()
            .expect("CloudWatchMetrics")
        {
            for definition in directive["Metrics"].as_array().expect("Metrics") {
                if let Some(name) = definition["Name"].as_str() {
                    if let Some(spec) = names::ALL_METRIC_NAMES
                        .iter()
                        .find(|spec_name| **spec_name == name)
                    {
                        emitted.insert(spec);
                    }
                }
            }
        }
    }
    let expected: BTreeSet<&str> = names::ALL_METRIC_NAMES.into_iter().collect();
    assert_eq!(
        emitted, expected,
        "every §20.1 metric must appear in EMF output"
    );
}

#[test]
fn labeled_metric_gets_its_own_dimensioned_events() {
    let events = sample_events();
    let labeled: Vec<&Value> = events
        .iter()
        .filter(|event| event.get(names::LABEL_SOURCE_TYPE).is_some())
        .collect();
    assert_eq!(labeled.len(), 2, "one event per source_type value");
    let mut sources: Vec<&str> = labeled
        .iter()
        .filter_map(|event| event[names::LABEL_SOURCE_TYPE].as_str())
        .collect();
    sources.sort_unstable();
    assert_eq!(sources, ["acme", "api"]);
}
