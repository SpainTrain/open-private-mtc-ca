//! Criterion signing-latency bench for the checkpoint signer vs the <100 ms p99
//! target (spec §14.3, §19.11).
//!
//! It times the full write-path step-7 pipeline — build → `signature_input` →
//! HSM sign (with the retry wrapper) → frame — over the in-memory ECDSA P-256
//! HSM, which runs everywhere `make bench` runs. Swap in
//! `cloud_softhsm::SoftHsm` behind the same `Arc<dyn Hsm>` seam to measure a
//! real token; cloud-softhsm's integration test already hard-asserts the
//! <100 ms p99 bar for the raw HSM sign, so this bench isolates the CA-side
//! per-checkpoint overhead around it.
//
// Benchmark harness, not production or `#[cfg(test)]` code: the unwrap/expect
// bans (spec §22.6) target production paths, so this measurement harness takes
// the documented scoped allow (docs/lint-policy.md). `missing_docs` is allowed
// because the `criterion_group!`/`criterion_main!` macros expand to public
// items a bench cannot document.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::sync::Arc;

use checkpoint_signer::{CheckpointSigner, RetryPolicy};
use clock::tokio::AsyncClock;
use clock::FakeClock;
use cloud_memory::MemoryHsm;
use cloud_types::{Hsm, KeySpec};
use criterion::{criterion_group, criterion_main, Criterion};
use mtc::{HashOutput, LogId, SignedAt, TreeSize};

fn bench_sign_checkpoint(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("current-thread runtime");

    let (signer, handle) = runtime.block_on(async {
        let hsm = Arc::new(MemoryHsm::new());
        let handle = hsm
            .generate_key(KeySpec::EcdsaP256)
            .await
            .expect("generate key");
        let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
        let signer = CheckpointSigner::new(hsm as Arc<dyn Hsm>, clock, RetryPolicy::default());
        (signer, handle)
    });

    let log_id = LogId::new("ca").expect("log id");
    let root_hash = HashOutput([0x11; 32]);

    c.bench_function(
        "sign_checkpoint (build+sign+frame, MemoryHsm; target <100ms p99, spec §14.3)",
        |b| {
            b.iter(|| {
                runtime.block_on(async {
                    signer
                        .sign_checkpoint(
                            &handle,
                            log_id.clone(),
                            TreeSize(2),
                            root_hash,
                            SignedAt(1_700_000_000),
                        )
                        .await
                        .expect("sign")
                })
            });
        },
    );
}

criterion_group!(benches, bench_sign_checkpoint);
criterion_main!(benches);
