//! Shared [`ReplicatedKv`] conformance suite (spec §9.7).
//!
//! Ported from `cloud-memory`'s original inline `#[cfg(test)]` module and
//! its separate `tests/replicated_kv_concurrency.rs` file (spec §9.6) -- the
//! assertions are unchanged, generalized behind the factory-closure pattern
//! so any backend can run them.
//!
//! # Two contract points this suite ratifies
//!
//! `cloud-types`' rustdoc on [`ReplicatedKv::atomic_update`] and
//! [`ReplicatedKv::transact`] already resolves both edge cases the
//! `memory-backend` ticket flagged as an implementation choice needing a
//! follow-up (`docs/journal.md`, 2026-07-23 memory-backend entry: "Both
//! choices are implementation details the cloud-test-suite-kv ticket should
//! validate/pin down"). This suite is where that resolution becomes an
//! enforced, cross-backend contract instead of a comment on one backend:
//!
//! - **`atomic_update` on a missing key** is [`CloudError::NotFound`] --
//!   `atomic_update` mutates an *existing* item, so a missing key is always
//!   "not found," independent of what conditions were requested (see the
//!   `# Errors` section on [`ReplicatedKv::atomic_update`]). Enforced by
//!   `test_atomic_update_missing_key_is_not_found` below.
//! - **`transact`'s `Update` op on a missing key** is
//!   [`CloudError::ConditionFailed`], not `NotFound` -- `transact`'s
//!   documented `# Errors` section has only `ConditionFailed`/`Transport`
//!   variants, so a missing item is folded into "the operation could not be
//!   applied" rather than widening `transact`'s error surface with a
//!   transaction-only `NotFound`. Enforced by
//!   `test_transact_update_on_missing_key_is_condition_failed` below.

use std::collections::BTreeMap;
use std::future::Future;
use std::ops::RangeInclusive;
use std::sync::Arc;

use cloud_types::{CloudError, Condition, Key, Operation, ReplicatedKv, UpdateExpression, Value};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

/// Task-count range sampled per round in the concurrency property cases
/// (spec §19.2): varying N, rather than a single fixed count, is what makes
/// these proptest-driven rather than a hand-picked constant.
const CONCURRENCY_TASK_RANGE: RangeInclusive<usize> = 4..=48;

/// Number of independent rounds each concurrency property case runs; every
/// round is a fresh CAS race (spec §9.5 "of N concurrent conditional puts
/// racing on one key, at most one may succeed per state transition").
const CONCURRENCY_ROUNDS: u32 = 6;

/// Runs the full [`ReplicatedKv`] conformance suite against instances built
/// by `factory`.
///
/// `factory` is called once per sub-test case (spec §9.7's factory-closure
/// pattern) so cases never share state.
///
/// # Panics
///
/// Panics (via `assert!`/`assert_eq!`) on the first behavior that diverges
/// from the contract documented on [`ReplicatedKv`].
pub async fn run_replicated_kv_suite<F, Fut, KV>(factory: F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv + 'static,
{
    test_put_then_get_round_trips(&factory).await;
    test_get_missing_is_not_found(&factory).await;
    test_conditional_put_succeeds_then_loses_cas(&factory).await;
    test_attribute_equals_condition_gates_put(&factory).await;
    test_atomic_update_increments_and_returns_post_state(&factory).await;
    test_atomic_update_honors_epoch_condition(&factory).await;
    test_atomic_update_missing_key_is_not_found(&factory).await;
    test_atomic_update_increment_on_absent_attribute_is_condition_failed(&factory).await;
    test_query_returns_matching_prefix_sorted_by_key(&factory).await;
    test_query_with_no_matches_is_empty_ok(&factory).await;
    test_transact_applies_all_ops_when_every_condition_holds(&factory).await;
    test_transact_with_one_failing_condition_applies_zero_ops(&factory).await;
    test_transact_failing_op_first_still_applies_zero_ops(&factory).await;
    test_transact_delete_and_put_are_all_or_nothing(&factory).await;
    test_transact_update_on_missing_key_is_condition_failed(&factory).await;
    test_concurrent_conditional_put_yields_exactly_one_winner(&factory).await;
    test_concurrent_atomic_update_yields_exactly_one_winner(&factory).await;
}

fn map_item(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

async fn test_put_then_get_round_trips<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let key = Key::new("cts/replicated-kv/round-trip");
    kv.put(&key, Value::U64(7), &[])
        .await
        .unwrap_or_else(|err| panic!("put should succeed: {err}"));
    let item = kv
        .get(&key)
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(item.value, Value::U64(7));
}

async fn test_get_missing_is_not_found<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let result = kv.get(&Key::new("cts/replicated-kv/missing")).await;
    let Err(err) = result else {
        panic!("get of a missing key must fail");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}

async fn test_conditional_put_succeeds_then_loses_cas<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let key = Key::new("cts/replicated-kv/cas-counter");
    kv.put(&key, Value::U64(1), &[Condition::NotExists])
        .await
        .unwrap_or_else(|err| panic!("first put should win the CAS: {err}"));
    let result = kv.put(&key, Value::U64(2), &[Condition::NotExists]).await;
    let Err(err) = result else {
        panic!("second put must lose the CAS");
    };
    assert!(err.is_precondition_failure(), "{err:?}");
    let item = kv
        .get(&key)
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(item.value, Value::U64(1));
}

async fn test_attribute_equals_condition_gates_put<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let key = Key::new("cts/replicated-kv/attribute-equals");
    kv.put(&key, Value::U64(3), &[])
        .await
        .unwrap_or_else(|err| panic!("seed should succeed: {err}"));
    let result = kv
        .put(
            &key,
            Value::U64(4),
            &[Condition::AttributeEquals {
                attribute: String::new(),
                expected: Value::U64(99),
            }],
        )
        .await;
    let Err(err) = result else {
        panic!("wrong expected value must fail");
    };
    assert!(matches!(err, CloudError::ConditionFailed { .. }), "{err:?}");
    kv.put(
        &key,
        Value::U64(4),
        &[Condition::AttributeEquals {
            attribute: String::new(),
            expected: Value::U64(3),
        }],
    )
    .await
    .unwrap_or_else(|err| panic!("correct expected value should succeed: {err}"));
    let item = kv
        .get(&key)
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(item.value, Value::U64(4));
}

async fn test_atomic_update_increments_and_returns_post_state<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let key = Key::new("cts/replicated-kv/atomic-increment");
    kv.put(&key, map_item([("next_index", Value::U64(0))]), &[])
        .await
        .unwrap_or_else(|err| panic!("seed should succeed: {err}"));
    let item = kv
        .atomic_update(
            &key,
            UpdateExpression::new().increment("next_index", 32),
            &[],
        )
        .await
        .unwrap_or_else(|err| panic!("atomic_update should succeed: {err}"));
    assert_eq!(item.value, map_item([("next_index", Value::U64(32))]));
}

async fn test_atomic_update_honors_epoch_condition<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let key = Key::new("cts/replicated-kv/atomic-epoch");
    kv.put(
        &key,
        map_item([("next_index", Value::U64(0)), ("epoch", Value::U64(1))]),
        &[],
    )
    .await
    .unwrap_or_else(|err| panic!("seed should succeed: {err}"));

    let stale = kv
        .atomic_update(
            &key,
            UpdateExpression::new().increment("next_index", 1),
            &[Condition::AttributeEquals {
                attribute: "epoch".to_string(),
                expected: Value::U64(2),
            }],
        )
        .await;
    let Err(stale_err) = stale else {
        panic!("stale epoch must lose the CAS");
    };
    assert!(stale_err.is_precondition_failure(), "{stale_err:?}");

    kv.atomic_update(
        &key,
        UpdateExpression::new().increment("next_index", 1),
        &[Condition::AttributeEquals {
            attribute: "epoch".to_string(),
            expected: Value::U64(1),
        }],
    )
    .await
    .unwrap_or_else(|err| panic!("current epoch should succeed: {err}"));
}

/// Ratified contract point 1 (see module docs): `atomic_update` on a missing
/// key is always [`CloudError::NotFound`], regardless of the conditions
/// requested.
async fn test_atomic_update_missing_key_is_not_found<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let result = kv
        .atomic_update(
            &Key::new("cts/replicated-kv/missing"),
            UpdateExpression::new(),
            &[],
        )
        .await;
    let Err(err) = result else {
        panic!("atomic_update on a missing key must fail");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}

async fn test_atomic_update_increment_on_absent_attribute_is_condition_failed<F, Fut, KV>(
    factory: &F,
) where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let key = Key::new("cts/replicated-kv/increment-absent-attribute");
    kv.put(&key, map_item([]), &[])
        .await
        .unwrap_or_else(|err| panic!("seed should succeed: {err}"));
    let result = kv
        .atomic_update(
            &key,
            UpdateExpression::new().increment("missing_attr", 1),
            &[],
        )
        .await;
    let Err(err) = result else {
        panic!("increment on an absent attribute must fail");
    };
    assert!(matches!(err, CloudError::ConditionFailed { .. }), "{err:?}");
}

async fn test_query_returns_matching_prefix_sorted_by_key<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    for k in [
        "cts/replicated-kv/query/coord/b",
        "cts/replicated-kv/query/coord/a",
        "cts/replicated-kv/query/other/x",
    ] {
        kv.put(&Key::new(k), Value::Bool(true), &[])
            .await
            .unwrap_or_else(|err| panic!("put should succeed: {err}"));
    }
    let items = kv
        .query("cts/replicated-kv/query/coord/")
        .await
        .unwrap_or_else(|err| panic!("query should succeed: {err}"));
    let keys: Vec<&str> = items.iter().map(|item| item.key.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "cts/replicated-kv/query/coord/a",
            "cts/replicated-kv/query/coord/b",
        ]
    );
}

async fn test_query_with_no_matches_is_empty_ok<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let items = kv
        .query("cts/replicated-kv/query/nothing/")
        .await
        .unwrap_or_else(|err| panic!("query should succeed: {err}"));
    assert_eq!(items, vec![]);
}

async fn test_transact_applies_all_ops_when_every_condition_holds<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let lease = Key::new("cts/replicated-kv/transact-ok/lease");
    let counter = Key::new("cts/replicated-kv/transact-ok/counter");
    kv.put(&lease, Value::U64(1), &[])
        .await
        .unwrap_or_else(|err| panic!("seed lease should succeed: {err}"));
    kv.put(&counter, map_item([("next_index", Value::U64(0))]), &[])
        .await
        .unwrap_or_else(|err| panic!("seed counter should succeed: {err}"));

    kv.transact(vec![
        Operation::ConditionCheck {
            key: lease.clone(),
            conditions: vec![Condition::AttributeEquals {
                attribute: String::new(),
                expected: Value::U64(1),
            }],
        },
        Operation::Update {
            key: counter.clone(),
            expr: UpdateExpression::new().increment("next_index", 32),
            conditions: vec![],
        },
    ])
    .await
    .unwrap_or_else(|err| panic!("transact should commit: {err}"));

    let item = kv
        .get(&counter)
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(item.value, map_item([("next_index", Value::U64(32))]));
}

async fn test_transact_with_one_failing_condition_applies_zero_ops<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let lease = Key::new("cts/replicated-kv/transact-fail/lease");
    let counter = Key::new("cts/replicated-kv/transact-fail/counter");
    kv.put(&lease, Value::U64(1), &[])
        .await
        .unwrap_or_else(|err| panic!("seed lease should succeed: {err}"));
    kv.put(&counter, map_item([("next_index", Value::U64(0))]), &[])
        .await
        .unwrap_or_else(|err| panic!("seed counter should succeed: {err}"));

    let result = kv
        .transact(vec![
            Operation::Update {
                key: counter.clone(),
                expr: UpdateExpression::new().increment("next_index", 32),
                conditions: vec![],
            },
            // This one fails: the lease's value is 1, not 99.
            Operation::ConditionCheck {
                key: lease.clone(),
                conditions: vec![Condition::AttributeEquals {
                    attribute: String::new(),
                    expected: Value::U64(99),
                }],
            },
        ])
        .await;
    let Err(err) = result else {
        panic!("transact must fail");
    };
    assert!(matches!(err, CloudError::ConditionFailed { .. }), "{err:?}");

    // Neither op applied -- not even the one before the failing op.
    let counter_item = kv
        .get(&counter)
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(
        counter_item.value,
        map_item([("next_index", Value::U64(0))])
    );
    let lease_item = kv
        .get(&lease)
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(lease_item.value, Value::U64(1));
}

async fn test_transact_failing_op_first_still_applies_zero_ops<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let lease = Key::new("cts/replicated-kv/transact-fail-first/lease");
    kv.put(&lease, Value::U64(1), &[])
        .await
        .unwrap_or_else(|err| panic!("seed should succeed: {err}"));

    let side_effect = Key::new("cts/replicated-kv/transact-fail-first/side-effect");
    let result = kv
        .transact(vec![
            Operation::ConditionCheck {
                key: lease.clone(),
                conditions: vec![Condition::AttributeEquals {
                    attribute: String::new(),
                    expected: Value::U64(99),
                }],
            },
            Operation::Put {
                key: side_effect.clone(),
                value: Value::Bool(true),
                conditions: vec![],
            },
        ])
        .await;
    let Err(err) = result else {
        panic!("transact must fail");
    };
    assert!(matches!(err, CloudError::ConditionFailed { .. }), "{err:?}");
    let side_effect_result = kv.get(&side_effect).await;
    assert!(matches!(
        side_effect_result,
        Err(CloudError::NotFound { .. })
    ));
}

async fn test_transact_delete_and_put_are_all_or_nothing<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let a = Key::new("cts/replicated-kv/transact-delete-put/a");
    let b = Key::new("cts/replicated-kv/transact-delete-put/b");
    kv.put(&a, Value::U64(1), &[])
        .await
        .unwrap_or_else(|err| panic!("seed a should succeed: {err}"));

    let result = kv
        .transact(vec![
            Operation::Delete {
                key: a.clone(),
                conditions: vec![],
            },
            Operation::Put {
                key: b.clone(),
                value: Value::U64(2),
                conditions: vec![Condition::Exists], // fails: b doesn't exist
            },
        ])
        .await;
    let Err(err) = result else {
        panic!("transact must fail");
    };
    assert!(matches!(err, CloudError::ConditionFailed { .. }), "{err:?}");

    // `a` was not deleted.
    let a_item = kv
        .get(&a)
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(a_item.value, Value::U64(1));
    assert!(matches!(kv.get(&b).await, Err(CloudError::NotFound { .. })));
}

/// Ratified contract point 2 (see module docs): an [`Operation::Update`]
/// inside `transact` that targets a missing key is
/// [`CloudError::ConditionFailed`], not `NotFound` -- and, like every other
/// failed `transact`, applies none of the transaction's other operations.
async fn test_transact_update_on_missing_key_is_condition_failed<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv,
{
    let kv = factory().await;
    let present = Key::new("cts/replicated-kv/transact-update-missing/present");
    let missing = Key::new("cts/replicated-kv/transact-update-missing/missing");
    kv.put(&present, Value::U64(1), &[])
        .await
        .unwrap_or_else(|err| panic!("seed should succeed: {err}"));

    let result = kv
        .transact(vec![
            Operation::Put {
                key: present.clone(),
                value: Value::U64(2),
                conditions: vec![],
            },
            Operation::Update {
                key: missing.clone(),
                expr: UpdateExpression::new().increment("n", 1),
                conditions: vec![],
            },
        ])
        .await;
    let Err(err) = result else {
        panic!("transact with an Update on a missing key must fail");
    };
    assert!(matches!(err, CloudError::ConditionFailed { .. }), "{err:?}");

    // Nothing applied -- not even the sibling Put that came first.
    let present_item = kv
        .get(&present)
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(present_item.value, Value::U64(1));
    assert!(matches!(
        kv.get(&missing).await,
        Err(CloudError::NotFound { .. })
    ));
}

/// Property test (spec §19.2): N concurrent conditional `put`s racing on one
/// key admit exactly one winner per round -- the invariant the lease/epoch
/// protocol's `Condition::NotExists` insert-only writes build on (spec §9.5:
/// "of N concurrent conditional puts racing on one key, at most one may
/// succeed per state transition").
async fn test_concurrent_conditional_put_yields_exactly_one_winner<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv + 'static,
{
    let kv = Arc::new(factory().await);
    let mut runner = TestRunner::default();

    for round in 0..CONCURRENCY_ROUNDS {
        let tasks = CONCURRENCY_TASK_RANGE
            .new_tree(&mut runner)
            .unwrap_or_else(|reason| panic!("proptest task-count generation failed: {reason}"))
            .current();
        // A fresh key per round: each round is an independent CAS race
        // starting from "key absent," the scenario Condition::NotExists
        // guards.
        let key = Key::new(format!("cts/replicated-kv/concurrent-put/{round}"));

        let mut joins = Vec::with_capacity(tasks);
        for task in 0..tasks {
            let kv = Arc::clone(&kv);
            let key = key.clone();
            joins.push(tokio::spawn(async move {
                kv.put(
                    &key,
                    Value::U64(u64::try_from(task).unwrap_or(u64::MAX)),
                    &[Condition::NotExists],
                )
                .await
            }));
        }

        let mut wins = 0usize;
        let mut losses = 0usize;
        for join in joins {
            let result = join
                .await
                .unwrap_or_else(|err| panic!("round {round}: task must not panic: {err}"));
            match result {
                Ok(()) => wins += 1,
                Err(CloudError::ConditionFailed { .. }) => losses += 1,
                Err(other) => panic!("round {round}: unexpected error {other:?}"),
            }
        }
        assert_eq!(
            wins, 1,
            "round {round} ({tasks} tasks): exactly one task must win the CAS"
        );
        assert_eq!(
            losses,
            tasks - 1,
            "round {round} ({tasks} tasks): everyone else must lose the CAS"
        );

        // The stored value is whichever task won -- read-back is
        // consistent, never a torn write from two "winners."
        let item = kv
            .get(&key)
            .await
            .unwrap_or_else(|err| panic!("round {round}: winner's write must be visible: {err}"));
        let Value::U64(winner_task) = item.value else {
            panic!("round {round}: expected a U64 value");
        };
        assert!(usize::try_from(winner_task).unwrap_or(usize::MAX) < tasks);
    }
}

/// Same invariant as
/// [`test_concurrent_conditional_put_yields_exactly_one_winner`], but racing
/// `atomic_update` (epoch-checked increment) instead of `put` -- the
/// primitive the lease/epoch protocol's counter allocation actually uses
/// (spec §8.3).
async fn test_concurrent_atomic_update_yields_exactly_one_winner<F, Fut, KV>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = KV> + Send,
    KV: ReplicatedKv + 'static,
{
    let kv = Arc::new(factory().await);
    let key = Key::new("cts/replicated-kv/concurrent-atomic-update");
    kv.put(
        &key,
        Value::Map(BTreeMap::from([
            ("count".to_string(), Value::U64(0)),
            ("epoch".to_string(), Value::U64(0)),
        ])),
        &[],
    )
    .await
    .unwrap_or_else(|err| panic!("seed counter should succeed: {err}"));

    let mut runner = TestRunner::default();
    for round in 0..CONCURRENCY_ROUNDS {
        let tasks = CONCURRENCY_TASK_RANGE
            .new_tree(&mut runner)
            .unwrap_or_else(|reason| panic!("proptest task-count generation failed: {reason}"))
            .current();
        let round_u64 = u64::from(round);

        let mut joins = Vec::with_capacity(tasks);
        for _ in 0..tasks {
            let kv = Arc::clone(&kv);
            let key = key.clone();
            joins.push(tokio::spawn(async move {
                kv.atomic_update(
                    &key,
                    UpdateExpression::new()
                        .increment("count", 1)
                        .set("epoch", Value::U64(round_u64 + 1)),
                    &[Condition::AttributeEquals {
                        attribute: "epoch".to_string(),
                        expected: Value::U64(round_u64),
                    }],
                )
                .await
            }));
        }

        let mut wins = 0usize;
        let mut losses = 0usize;
        for join in joins {
            let result = join
                .await
                .unwrap_or_else(|err| panic!("round {round}: task must not panic: {err}"));
            match result {
                Ok(_) => wins += 1,
                Err(CloudError::ConditionFailed { .. }) => losses += 1,
                Err(other) => panic!("round {round}: unexpected error {other:?}"),
            }
        }
        assert_eq!(
            wins, 1,
            "round {round} ({tasks} tasks): exactly one task must win the epoch CAS"
        );
        assert_eq!(losses, tasks - 1);
    }

    let final_item = kv
        .get(&key)
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    let Value::Map(map) = final_item.value else {
        panic!("expected a map");
    };
    // Exactly one increment landed per round -- never zero, never more.
    assert_eq!(
        map.get("count"),
        Some(&Value::U64(u64::from(CONCURRENCY_ROUNDS)))
    );
}
