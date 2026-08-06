//! Property test: `parse(serialize(x)) == x` for [`SourceType`], [`SourceId`],
//! and [`LogEntry`] (spec §19.2's round-trip discipline, applied to the
//! §10.2 intake envelope rather than the wire-format codec).
//!
//! `serde_json` is used as a concrete, ubiquitous stand-in serde format for
//! this test only; every type here is format-agnostic (any `serde` data
//! format embeds them) — nothing in this crate is pinned to JSON.

// Test-only file: the production unwrap/expect ban does not apply here
// (docs/lint-policy.md: non-#[test] helpers in integration-test files still
// need this scoped allow; every function below is a #[test] or a proptest!
// body, but the strategy closures are not literally `#[test]` fns).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, UNIX_EPOCH};

use mtc_ca_service::{LogEntry, SourceId, SourceType};
use proptest::prelude::*;

fn arb_source_type() -> impl Strategy<Value = SourceType> {
    prop_oneof![
        Just(SourceType::NativeAcme),
        any::<String>().prop_map(SourceType::Adapter),
    ]
}

fn arb_source_id() -> impl Strategy<Value = SourceId> {
    any::<String>().prop_map(SourceId)
}

fn arb_submitted_at() -> impl Strategy<Value = std::time::SystemTime> {
    // UNIX_EPOCH + (0..=u32::MAX) seconds covers roughly 1970-2106, always
    // >= UNIX_EPOCH — serde's `SystemTime` impl errors on times before it.
    (any::<u32>(), 0..1_000_000_000u32)
        .prop_map(|(secs, nanos)| UNIX_EPOCH + Duration::new(u64::from(secs), nanos))
}

fn arb_log_entry() -> impl Strategy<Value = LogEntry> {
    (
        proptest::collection::vec(any::<u8>(), 0..256),
        arb_source_type(),
        arb_source_id(),
        arb_submitted_at(),
    )
        .prop_map(|(tbs_cert, source_type, source_id, submitted_at)| {
            LogEntry::new(tbs_cert, source_type, source_id, submitted_at)
        })
}

proptest! {
    /// Spec §19.2 round-trip discipline: any `SourceType` survives a
    /// serialize/deserialize cycle unchanged, whether it is the v1
    /// `NativeAcme` variant or an arbitrary future adapter tag.
    #[test]
    fn source_type_round_trips(value in arb_source_type()) {
        let json = serde_json::to_string(&value).expect("SourceType always encodes");
        let parsed: SourceType = serde_json::from_str(&json).expect("valid JSON parses back");
        prop_assert_eq!(parsed, value);
    }

    /// `SourceId` wraps an arbitrary adapter-supplied string (spec §10.2);
    /// any string content, including empty and non-ASCII, round-trips.
    #[test]
    fn source_id_round_trips(value in arb_source_id()) {
        let json = serde_json::to_string(&value).expect("SourceId always encodes");
        let parsed: SourceId = serde_json::from_str(&json).expect("valid JSON parses back");
        prop_assert_eq!(parsed, value);
    }

    /// The full `LogEntry` envelope (spec §10.2: `tbs_cert`, `source_type`,
    /// `source_id`, `submitted_at`) round-trips as a whole — the shape
    /// `EntryIntake::submit_entry` callers construct and the shape batch
    /// state will eventually persist (spec §8.2).
    #[test]
    fn log_entry_round_trips(entry in arb_log_entry()) {
        let json = serde_json::to_string(&entry).expect("LogEntry always encodes");
        let parsed: LogEntry = serde_json::from_str(&json).expect("valid JSON parses back");
        prop_assert_eq!(parsed, entry);
    }
}
