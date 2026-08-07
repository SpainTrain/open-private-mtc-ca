//! Shared `proptest` strategy helpers and the `Arbitrary` convention (spec
//! §19.2, §19.3 Layer 1).
//!
//! # Why these helpers live here, not `impl Arbitrary` on the types
//!
//! `proptest::arbitrary::Arbitrary` impls for `mtc`'s spec types are intended
//! to live *with each type*, inside `crates/mtc`, gated behind a `testing`
//! Cargo feature (a separate ticket owns adding them — this crate must not
//! reach back into `mtc`, which it depends on read-only). The eventual shape
//! looks like:
//!
//! ```ignore
//! // crates/mtc/src/types.rs, once the `testing` feature lands.
//! #[cfg(any(test, feature = "testing"))]
//! impl proptest::arbitrary::Arbitrary for HashOutput {
//!     type Parameters = ();
//!     type Strategy = proptest::strategy::BoxedStrategy<Self>;
//!     fn arbitrary_with((): ()) -> Self::Strategy {
//!         // Built from this crate's helper, not reimplemented:
//!         mtc_testutil::strategy::arb_hash_output().boxed()
//!     }
//! }
//! ```
//!
//! That placement keeps the impl co-located with the type it generates —
//! spec §19.3 Layer 1's own example (`fn checkpoint_roundtrip(cp: Checkpoint)`)
//! only compiles once `Checkpoint: Arbitrary` — while this crate provides the
//! actual generation logic as plain functions returning
//! `impl Strategy<Value = T>`. Callers use `arb_hash_output()` etc. directly
//! today; a future `Arbitrary` impl wraps one of these functions rather than
//! duplicating it, so there is exactly one place each type's generation logic
//! is expressed.
//!
//! # Extended iteration counts in CI (spec §19.3 Layer 1: "10,000+ cases for
//! spec types")
//!
//! None of the strategies here — and no `proptest! { .. }` body in this
//! crate — overrides `cases` in a `#![proptest_config(..)]` block, and
//! neither should a caller's. `proptest` reads the `PROPTEST_CASES`
//! environment variable into `ProptestConfig::default()` before any explicit
//! `cases:` override would apply, so hardcoding `cases:` in code silences the
//! env var. Leaving `cases` unset — the plain `proptest! { #[test] fn foo(x
//! in ..) {..} }` form every property test in `crates/mtc` already uses — is
//! therefore the whole mechanism:
//!
//! - local runs default to proptest's own built-in 256 cases (fast, well
//!   within the spec §19.1 <30s unit-test budget);
//! - `PROPTEST_CASES=10000 cargo test` (or higher) runs the extended count
//!   spec §19.3 Layer 1 requires for spec types, with no code change.
//!
//! Wiring a CI job to set `PROPTEST_CASES=10000` (or higher) as a step/job
//! env var — so this happens automatically instead of by hand — is follow-up
//! work outside this crate (see the ticket report).
//!
//! # `proptest-regressions/`
//!
//! `proptest`'s default `FileFailurePersistence` writes a failing case's seed
//! to `<crate>/proptest-regressions/<test-file>.txt` the first time a
//! property test in that file fails, and replays it on every subsequent run
//! before trying new cases. These files are fixtures, not build output:
//! **commit them** — the workspace `.gitignore` does not exclude
//! `proptest-regressions/` (verified for this ticket). Once a failure is
//! fixed, its seed file becomes a permanent regression test (spec §19.3,
//! "Corpus management"). This crate has none checked in yet because none of
//! its property tests have ever failed; the directory will appear the first
//! time one does, in this crate or any component crate that adopts the same
//! convention.

use std::net::{Ipv4Addr, Ipv6Addr};

use proptest::prelude::*;

use mtc::{
    BatchId, Claim, DnsName, Epoch, HashOutput, Index, LogEntry, LogId, SubjectInfoHash,
    SubjectType, TbsCertificateLogEntry, TreeSize,
};

/// An arbitrary [`HashOutput`] (uniform over all 32-byte values).
pub fn arb_hash_output() -> impl Strategy<Value = HashOutput> {
    any::<[u8; 32]>().prop_map(HashOutput)
}

/// An arbitrary [`Index`] (uniform over the full `u64` range).
pub fn arb_index() -> impl Strategy<Value = Index> {
    any::<u64>().prop_map(Index)
}

/// An arbitrary [`TreeSize`] (uniform over the full `u64` range).
pub fn arb_tree_size() -> impl Strategy<Value = TreeSize> {
    any::<u64>().prop_map(TreeSize)
}

/// An arbitrary [`Epoch`] (uniform over the full `u64` range).
pub fn arb_epoch() -> impl Strategy<Value = Epoch> {
    any::<u64>().prop_map(Epoch)
}

/// An arbitrary non-empty printable-ASCII [`LogId`] (1..=64 bytes).
pub fn arb_log_id() -> impl Strategy<Value = LogId> {
    "[ -~]{1,64}".prop_filter_map("non-empty LogId", |s| LogId::new(s).ok())
}

/// An arbitrary non-empty printable-ASCII [`BatchId`] (1..=64 bytes).
pub fn arb_batch_id() -> impl Strategy<Value = BatchId> {
    "[ -~]{1,64}".prop_filter_map("non-empty BatchId", |s| BatchId::new(s).ok())
}

/// An arbitrary [`DnsName`]: 1..=32 arbitrary bytes (within the wire's
/// `1..=255` bound, with headroom left for composed claim-list strategies).
pub fn arb_dns_name() -> impl Strategy<Value = DnsName> {
    proptest::collection::vec(any::<u8>(), 1..=32)
        .prop_filter_map("1..=32 bytes is within DNSName<1..255>", |b| {
            DnsName::new(b).ok()
        })
}

/// An arbitrary [`Claim`]: one of the four v1 claim types with a non-empty
/// value list (draft-ietf-plants-merkle-tree-certs-03 §4.1, §4.2).
pub fn arb_claim() -> impl Strategy<Value = Claim> {
    prop_oneof![
        proptest::collection::vec(arb_dns_name(), 1..4)
            .prop_filter_map("non-empty dns claim", |n| Claim::dns(n).ok()),
        proptest::collection::vec(arb_dns_name(), 1..4)
            .prop_filter_map("non-empty dns_wildcard claim", |n| Claim::dns_wildcard(n)
                .ok()),
        proptest::collection::vec(any::<[u8; 4]>(), 1..4).prop_filter_map(
            "non-empty ipv4 claim",
            |a| Claim::ipv4(a.into_iter().map(Ipv4Addr::from).collect::<Vec<_>>()).ok()
        ),
        proptest::collection::vec(any::<[u8; 16]>(), 1..4).prop_filter_map(
            "non-empty ipv6 claim",
            |a| Claim::ipv6(a.into_iter().map(Ipv6Addr::from).collect::<Vec<_>>()).ok()
        ),
    ]
}

/// An arbitrary [`SubjectInfoHash`] (wraps an arbitrary [`HashOutput`]).
pub fn arb_subject_info_hash() -> impl Strategy<Value = SubjectInfoHash> {
    arb_hash_output().prop_map(SubjectInfoHash::from_hash)
}

/// An arbitrary [`TbsCertificateLogEntry`]: `subject_type` is always
/// [`SubjectType::Tls`] (v1's only variant), with 0..4 arbitrary claims.
pub fn arb_certificate_entry() -> impl Strategy<Value = TbsCertificateLogEntry> {
    (
        arb_subject_info_hash(),
        proptest::collection::vec(arb_claim(), 0..4),
    )
        .prop_map(|(hash, claims)| {
            TbsCertificateLogEntry::builder()
                .subject_type(SubjectType::Tls)
                .subject_info_hash(hash)
                .claims(claims)
                .build()
        })
}

/// An arbitrary [`LogEntry`]: either [`LogEntry::Null`] (the `null_entry`
/// gap placeholder) or a [`LogEntry::Certificate`] wrapping
/// [`arb_certificate_entry`].
pub fn arb_log_entry() -> impl Strategy<Value = LogEntry> {
    prop_oneof![
        Just(LogEntry::null()),
        arb_certificate_entry().prop_map(LogEntry::certificate),
    ]
}

#[cfg(test)]
mod tests {
    use mtc::{Claim, LogEntry, SubjectInfoHash, SubjectType, TlsParse, TlsSerialize};
    use proptest::prelude::*;

    use super::{
        arb_batch_id, arb_certificate_entry, arb_claim, arb_dns_name, arb_epoch, arb_hash_output,
        arb_index, arb_log_entry, arb_log_id, arb_subject_info_hash, arb_tree_size,
    };

    proptest! {
        #[test]
        fn arb_hash_output_is_always_32_bytes(h in arb_hash_output()) {
            prop_assert_eq!(h.as_bytes().len(), 32);
        }

        #[test]
        fn arb_index_tree_size_epoch_debug_format_embeds_the_value(
            i in arb_index(), t in arb_tree_size(), e in arb_epoch(),
        ) {
            // Cross-checks the generated value against each newtype's own
            // Debug impl (mirrors crates/mtc's own `integer_newtypes_debug_
            // format_names_the_type` unit test), rather than a tautology.
            prop_assert_eq!(format!("{i:?}"), format!("Index({})", i.0));
            prop_assert_eq!(format!("{t:?}"), format!("TreeSize({})", t.0));
            prop_assert_eq!(format!("{e:?}"), format!("Epoch({})", e.0));
        }

        #[test]
        fn arb_log_id_and_batch_id_are_never_empty(log in arb_log_id(), batch in arb_batch_id()) {
            prop_assert!(!log.as_str().is_empty());
            prop_assert!(!batch.as_str().is_empty());
        }

        #[test]
        fn arb_dns_name_respects_the_wire_bound(name in arb_dns_name()) {
            prop_assert!(!name.as_bytes().is_empty());
            prop_assert!(name.as_bytes().len() <= mtc::DnsName::MAX_LEN);
        }

        #[test]
        fn arb_claim_value_lists_are_never_empty(claim in arb_claim()) {
            let len = match &claim {
                Claim::Dns(n) | Claim::DnsWildcard(n) => n.len(),
                Claim::Ipv4(a) => a.len(),
                Claim::Ipv6(a) => a.len(),
                // `Claim` is #[non_exhaustive] outside its defining crate;
                // this arm only guards a future variant `arb_claim` cannot
                // yet produce.
                _ => unreachable!("arb_claim only generates the four known variants"),
            };
            prop_assert!(len >= 1);
        }

        #[test]
        fn arb_subject_info_hash_round_trips_its_hash(sih in arb_subject_info_hash()) {
            // The newtype is a lossless wrapper: re-wrapping the exposed hash
            // must reproduce the same value.
            prop_assert_eq!(SubjectInfoHash::from_hash(*sih.as_hash()), sih);
        }

        #[test]
        fn arb_certificate_entry_always_has_the_v1_tls_subject(entry in arb_certificate_entry()) {
            prop_assert_eq!(entry.subject_type(), SubjectType::Tls);
        }

        #[test]
        fn arb_log_entry_is_null_or_certificate(entry in arb_log_entry()) {
            prop_assert!(matches!(entry, LogEntry::Null | LogEntry::Certificate(_)));
        }

        // Exemplar property test (ticket AC): round-trip a core spec type
        // through the wire codec using ONLY this crate's shared strategy
        // helper — the pattern every component epic's property tests reuse
        // (spec §19.2 "Serialization round-trips", §19.3 Layer 1).
        #[test]
        fn exemplar_log_entry_round_trips_through_the_wire_codec(entry in arb_log_entry()) {
            let bytes = entry.tls_serialize_to_vec().expect("fixture entry encodes");
            let parsed = LogEntry::tls_parse_exact(&bytes);
            prop_assert_eq!(parsed.as_ref(), Ok(&entry));
        }
    }
}
