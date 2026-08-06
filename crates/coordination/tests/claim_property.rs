//! Property test (spec §19.2): N concurrent `claim_lease` attempts on one
//! expired lease yield **exactly one** winner per round, and the winning epochs
//! across rounds are unique and strictly monotonic — the core safety property
//! of the epoch-fencing takeover (spec §8.3: "every takeover atomically
//! increments epoch").
//!
//! File-level unwrap/expect allow: this is an integration test, and proptest
//! runs the body inside a closure clippy does not recognize as `#[test]`
//! context, so the per-helper scoped allow (docs/lint-policy.md) is applied
//! once here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use clock::{Clock, FakeClock};
use cloud_memory::MemoryReplicatedKv;
use cloud_types::ReplicatedKv;
use coordination::{
    HolderId, LeaseCoordinator, LeaseError, LogId, Region, LEASE_TTL, TAKEOVER_SAFETY_MARGIN,
};
use proptest::prelude::*;

/// Number of successive takeover rounds per case; each must produce one winner
/// whose epoch is exactly one greater than the previous round's.
const ROUNDS: u64 = 4;

fn make_coord(
    kv: &Arc<dyn ReplicatedKv>,
    clock: &Arc<FakeClock>,
    log_id: &LogId,
    holder: &str,
    region: &str,
) -> LeaseCoordinator {
    let dyn_clock: Arc<dyn Clock> = clock.clone();
    LeaseCoordinator::new(
        Arc::clone(kv),
        dyn_clock,
        log_id,
        HolderId::new(holder),
        Region::new(region),
    )
}

proptest! {
    // Each case spawns up to ROUNDS * claimants tasks; keep the case count
    // modest so the suite stays fast.
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn n_concurrent_claims_yield_one_monotonic_winner(claimants in 2usize..=10) {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap();

        let outcome: Result<(), TestCaseError> = rt.block_on(async {
            let kv: Arc<dyn ReplicatedKv> = Arc::new(MemoryReplicatedKv::new());
            let clock = Arc::new(FakeClock::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000)));
            let log_id = LogId::new("prop-log").unwrap();

            // Seed the lease so there is always an expired incumbent to take over.
            let seed = make_coord(&kv, &clock, &log_id, "seed", "us-east-1");
            let first = seed.acquire().await.unwrap();
            let mut prev_epoch = first.epoch.0;

            for round in 0..ROUNDS {
                // Make the current lease takeover-eligible for this round.
                clock.advance(LEASE_TTL + TAKEOVER_SAFETY_MARGIN + Duration::from_secs(1));

                // Fire N claimants at the same expired lease concurrently.
                let mut joins = Vec::with_capacity(claimants);
                for i in 0..claimants {
                    let holder = format!("r{round}-c{i}");
                    let coord = make_coord(&kv, &clock, &log_id, &holder, "us-west-2");
                    joins.push(tokio::spawn(async move { coord.claim_lease().await }));
                }

                let mut winning_epochs = Vec::new();
                for join in joins {
                    match join.await.unwrap() {
                        Ok(lease) => winning_epochs.push(lease.epoch.0),
                        // A loser either lost the epoch CAS (EpochAdvanced) or
                        // read the already-taken-over fresh lease (LeaseHeld).
                        Err(LeaseError::EpochAdvanced | LeaseError::LeaseHeld) => {}
                        Err(other) => {
                            prop_assert!(false, "round {}: unexpected error {:?}", round, other);
                        }
                    }
                }

                prop_assert_eq!(
                    winning_epochs.len(),
                    1,
                    "round {}: exactly one claimant must win",
                    round
                );
                let won = winning_epochs[0];
                prop_assert_eq!(
                    won,
                    prev_epoch + 1,
                    "round {}: winning epoch must be prev+1 (unique + monotonic)",
                    round
                );
                prev_epoch = won;
            }

            Ok(())
        });

        outcome?;
    }
}
