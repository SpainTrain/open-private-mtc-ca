//! Property test (spec §19.2): concurrent conditional writes on
//! `MemoryReplicatedKv` admit exactly one winner per round — the invariant
//! the lease/epoch protocol is built on (spec §9.5 "of N concurrent
//! conditional puts racing on one key, at most one may succeed per state
//! transition").

use std::sync::Arc;

use cloud_memory::MemoryReplicatedKv;
use cloud_types::{CloudError, Condition, Key, ReplicatedKv, Value};

const TASKS: u64 = 100;
const ROUNDS: u64 = 20;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_conditional_puts_yield_exactly_one_winner_per_round() {
    let kv = Arc::new(MemoryReplicatedKv::new());

    for round in 0..ROUNDS {
        // A fresh key per round: each round is an independent CAS race
        // starting from "key absent", the scenario Condition::NotExists
        // guards.
        let key = Key::new(format!("race/{round}"));

        let mut joins = Vec::with_capacity(usize::try_from(TASKS).unwrap_or(usize::MAX));
        for task in 0..TASKS {
            let kv = Arc::clone(&kv);
            let key = key.clone();
            joins.push(tokio::spawn(async move {
                kv.put(&key, Value::U64(task), &[Condition::NotExists])
                    .await
            }));
        }

        let mut wins = 0u32;
        let mut losses = 0u32;
        for join in joins {
            match join.await.expect("task must not panic") {
                Ok(()) => wins += 1,
                Err(CloudError::ConditionFailed { .. }) => losses += 1,
                Err(other) => panic!("round {round}: unexpected error {other:?}"),
            }
        }

        assert_eq!(wins, 1, "round {round}: exactly one task must win the CAS");
        assert_eq!(
            losses,
            u32::try_from(TASKS).unwrap_or(u32::MAX) - 1,
            "round {round}: everyone else must lose the CAS"
        );

        // The stored value is whichever task won — read-back is consistent,
        // never a torn write from two "winners".
        let stored = kv.get(&key).await.expect("winner's write is visible");
        let Value::U64(winner_task) = stored.value else {
            panic!("round {round}: expected a U64 value");
        };
        assert!(winner_task < TASKS);
    }
}

/// Same invariant, but racing `atomic_update` (epoch-checked increment)
/// instead of `put` — the primitive the lease/epoch protocol's counter
/// allocation actually uses (spec §8.3).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_epoch_checked_increments_yield_exactly_one_winner_per_round() {
    use std::collections::BTreeMap;

    use cloud_types::{Item, UpdateExpression};

    let kv = Arc::new(MemoryReplicatedKv::new());
    let key = Key::new("coord/counter");
    kv.put(
        &key,
        Value::Map(BTreeMap::from([
            ("count".to_string(), Value::U64(0)),
            ("epoch".to_string(), Value::U64(0)),
        ])),
        &[],
    )
    .await
    .expect("seed counter");

    for round in 0..ROUNDS {
        let mut joins = Vec::with_capacity(usize::try_from(TASKS).unwrap_or(usize::MAX));
        for _ in 0..TASKS {
            let kv = Arc::clone(&kv);
            let key = key.clone();
            joins.push(tokio::spawn(async move {
                kv.atomic_update(
                    &key,
                    UpdateExpression::new()
                        .increment("count", 1)
                        .set("epoch", Value::U64(round + 1)),
                    &[Condition::AttributeEquals {
                        attribute: "epoch".to_string(),
                        expected: Value::U64(round),
                    }],
                )
                .await
            }));
        }

        let mut wins = 0u32;
        let mut losses = 0u32;
        for join in joins {
            match join.await.expect("task must not panic") {
                Ok(Item { .. }) => wins += 1,
                Err(CloudError::ConditionFailed { .. }) => losses += 1,
                Err(other) => panic!("round {round}: unexpected error {other:?}"),
            }
        }

        assert_eq!(
            wins, 1,
            "round {round}: exactly one task must win the epoch CAS"
        );
        assert_eq!(losses, u32::try_from(TASKS).unwrap_or(u32::MAX) - 1);
    }

    let final_item = kv.get(&key).await.expect("get");
    let Value::Map(map) = final_item.value else {
        panic!("expected a map");
    };
    // Exactly one increment landed per round — never zero, never more.
    assert_eq!(map.get("count"), Some(&Value::U64(ROUNDS)));
}
