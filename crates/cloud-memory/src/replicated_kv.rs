//! [`MemoryReplicatedKv`] — pure in-memory [`ReplicatedKv`] (spec §9.3, §9.6).
//!
//! Backed by a single `Mutex`-guarded `BTreeMap<Key, Value>`: conditional
//! writes and transactions are implemented as "validate the whole batch
//! against the current snapshot, then apply" under one lock hold, which is
//! what makes them atomic without any lock-free CAS machinery (spec §9.5:
//! "of N concurrent conditional puts racing on one key, at most one may
//! succeed per state transition").

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use cloud_types::{
    CloudError, Condition, Item, Key, Operation, ReplicatedKv, UpdateAction, UpdateExpression,
    Value,
};

/// Pure in-memory, process-local [`ReplicatedKv`].
///
/// Shareable as `Arc<dyn ReplicatedKv>` across tokio tasks (spec §9.4): all
/// interior state is a single `std::sync::Mutex`, held only for the
/// synchronous duration of one call (never across an `.await`), with no
/// `unsafe` (rule `no-unsafe`).
#[derive(Debug, Default)]
pub struct MemoryReplicatedKv {
    items: Mutex<BTreeMap<Key, Value>>,
}

impl MemoryReplicatedKv {
    /// Creates an empty key-value store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Locks the item map, recovering from poisoning rather than panicking —
    /// see `object_store.rs`'s `lock_objects` for the identical rationale.
    fn lock_items(&self) -> MutexGuard<'_, BTreeMap<Key, Value>> {
        self.items.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Resolves the [`Value`] a [`Condition::AttributeEquals`] compares against:
/// the whole item value when `attribute` is `""`, or the named entry of a
/// top-level [`Value::Map`] otherwise (cloud-types rustdoc on
/// [`Condition::AttributeEquals`]). `None` (no item, or a non-`Map` item with
/// a non-empty `attribute`) never equals any `expected` value.
fn attribute_value<'a>(value: Option<&'a Value>, attribute: &str) -> Option<&'a Value> {
    let value = value?;
    if attribute.is_empty() {
        Some(value)
    } else if let Value::Map(map) = value {
        map.get(attribute)
    } else {
        None
    }
}

/// Evaluates every condition against `value` (the item's state before this
/// write), short-circuiting on the first violation.
fn check_conditions(value: Option<&Value>, conditions: &[Condition]) -> Result<(), CloudError> {
    for condition in conditions {
        match condition {
            Condition::NotExists => {
                if value.is_some() {
                    return Err(CloudError::ConditionFailed {
                        reason: "item already exists".to_string(),
                    });
                }
            }
            Condition::Exists => {
                if value.is_none() {
                    return Err(CloudError::ConditionFailed {
                        reason: "item does not exist".to_string(),
                    });
                }
            }
            Condition::AttributeEquals {
                attribute,
                expected,
            } => {
                if attribute_value(value, attribute) != Some(expected) {
                    return Err(CloudError::ConditionFailed {
                        reason: format!("attribute {attribute:?} did not match expected value"),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Applies `expr`'s actions to `existing` in order, returning the resulting
/// value. `existing` must be [`Value::Map`] — every [`UpdateAction`] names an
/// attribute within the item's top-level map (cloud-types rustdoc on
/// [`ReplicatedKv::atomic_update`]); a non-`Map` item is a
/// [`CloudError::ConditionFailed`], as is [`UpdateAction::Increment`]
/// targeting a missing or non-`U64` attribute (cloud-types rustdoc on
/// [`UpdateAction::Increment`]).
fn apply_update_actions(existing: &Value, expr: &UpdateExpression) -> Result<Value, CloudError> {
    let Value::Map(mut map) = existing.clone() else {
        return Err(CloudError::ConditionFailed {
            reason: "atomic_update requires a Map-valued item".to_string(),
        });
    };
    for action in &expr.actions {
        match action {
            UpdateAction::Set { attribute, value } => {
                map.insert(attribute.clone(), value.clone());
            }
            UpdateAction::Increment { attribute, by } => {
                let next = match map.get(attribute) {
                    Some(Value::U64(current)) => {
                        current
                            .checked_add(*by)
                            .ok_or_else(|| CloudError::ConditionFailed {
                                reason: format!("attribute {attribute:?} would overflow u64"),
                            })?
                    }
                    _ => {
                        return Err(CloudError::ConditionFailed {
                            reason: format!("attribute {attribute:?} is not a U64 attribute"),
                        })
                    }
                };
                map.insert(attribute.clone(), Value::U64(next));
            }
            UpdateAction::Remove { attribute } => {
                map.remove(attribute);
            }
        }
    }
    Ok(Value::Map(map))
}

/// The effect a validated [`Operation`] will have, computed in
/// [`MemoryReplicatedKv::transact`]'s validation pass so the apply pass
/// cannot fail partway through.
enum PlannedWrite {
    Put(Key, Value),
    Delete(Key),
    NoOp,
}

#[async_trait]
impl ReplicatedKv for MemoryReplicatedKv {
    async fn get(&self, key: &Key) -> Result<Item, CloudError> {
        self.lock_items()
            .get(key)
            .cloned()
            .map(|value| Item {
                key: key.clone(),
                value,
            })
            .ok_or_else(|| CloudError::NotFound {
                key: key.as_str().to_string(),
            })
    }

    async fn put(
        &self,
        key: &Key,
        value: Value,
        conditions: &[Condition],
    ) -> Result<(), CloudError> {
        let mut items = self.lock_items();
        check_conditions(items.get(key), conditions)?;
        items.insert(key.clone(), value);
        drop(items);
        Ok(())
    }

    async fn atomic_update(
        &self,
        key: &Key,
        expr: UpdateExpression,
        conditions: &[Condition],
    ) -> Result<Item, CloudError> {
        let mut items = self.lock_items();
        // Existence is checked before conditions: atomic_update mutates an
        // existing item, so a missing key is always NotFound, regardless of
        // what conditions were asked for (cloud-types rustdoc).
        let existing = items.get(key).ok_or_else(|| CloudError::NotFound {
            key: key.as_str().to_string(),
        })?;
        check_conditions(Some(existing), conditions)?;
        let updated = apply_update_actions(existing, &expr)?;
        items.insert(key.clone(), updated.clone());
        drop(items);
        Ok(Item {
            key: key.clone(),
            value: updated,
        })
    }

    // The lock must be held across the entire two-pass validate-then-apply
    // sequence below to guarantee transact's all-or-nothing atomicity; its
    // last use is inside the final loop (varies per iteration), which
    // clippy's significant_drop_tightening can't express as a single earlier
    // drop point (.claude/rules — deviation documented here, not centrally,
    // per docs/lint-policy.md's item-scoped-allow guidance).
    #[allow(clippy::significant_drop_tightening)]
    async fn transact(&self, ops: Vec<Operation>) -> Result<(), CloudError> {
        let mut items = self.lock_items();

        // Pass 1 (validate): evaluate every op's conditions against the
        // pre-transaction snapshot and compute what each op *would* write,
        // without mutating anything. transact's documented errors are just
        // ConditionFailed/Transport (no NotFound), so an Update targeting a
        // missing item is folded into ConditionFailed here.
        let mut planned = Vec::with_capacity(ops.len());
        for op in &ops {
            let step = match op {
                Operation::Put {
                    key,
                    value,
                    conditions,
                } => {
                    check_conditions(items.get(key), conditions)?;
                    PlannedWrite::Put(key.clone(), value.clone())
                }
                Operation::Update {
                    key,
                    expr,
                    conditions,
                } => {
                    let existing = items.get(key);
                    check_conditions(existing, conditions)?;
                    let existing = existing.ok_or_else(|| CloudError::ConditionFailed {
                        reason: format!("{key}: item does not exist"),
                    })?;
                    PlannedWrite::Put(key.clone(), apply_update_actions(existing, expr)?)
                }
                Operation::Delete { key, conditions } => {
                    check_conditions(items.get(key), conditions)?;
                    PlannedWrite::Delete(key.clone())
                }
                Operation::ConditionCheck { key, conditions } => {
                    check_conditions(items.get(key), conditions)?;
                    PlannedWrite::NoOp
                }
            };
            planned.push(step);
        }

        // Pass 2 (apply): every step already validated against the
        // unmodified snapshot, so this cannot fail — all writes land, or (on
        // any pass-1 error above, via `?`) none do.
        for step in planned {
            match step {
                PlannedWrite::Put(key, value) => {
                    items.insert(key, value);
                }
                PlannedWrite::Delete(key) => {
                    items.remove(&key);
                }
                PlannedWrite::NoOp => {}
            }
        }
        Ok(())
    }

    async fn query(&self, prefix: &str) -> Result<Vec<Item>, CloudError> {
        Ok(self
            .lock_items()
            .iter()
            .filter(|(key, _)| key.as_str().starts_with(prefix))
            .map(|(key, value)| Item {
                key: key.clone(),
                value: value.clone(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;

    use super::*;

    fn map_item(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        Value::Map(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let kv = MemoryReplicatedKv::new();
        let key = Key::new("lease/primary");
        kv.put(&key, Value::U64(7), &[]).await.expect("put");
        assert_eq!(kv.get(&key).await.expect("get").value, Value::U64(7));
    }

    #[tokio::test]
    async fn get_missing_is_not_found() {
        let kv = MemoryReplicatedKv::new();
        assert!(matches!(
            kv.get(&Key::new("missing")).await,
            Err(CloudError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn conditional_put_succeeds_then_loses_cas() {
        let kv = MemoryReplicatedKv::new();
        let key = Key::new("coord/counter");
        kv.put(&key, Value::U64(1), &[Condition::NotExists])
            .await
            .expect("first put wins the CAS");
        let err = kv
            .put(&key, Value::U64(2), &[Condition::NotExists])
            .await
            .expect_err("second put loses the CAS");
        assert!(err.is_precondition_failure());
        assert_eq!(kv.get(&key).await.expect("get").value, Value::U64(1));
    }

    #[tokio::test]
    async fn attribute_equals_condition_gates_put() {
        let kv = MemoryReplicatedKv::new();
        let key = Key::new("lease/primary");
        kv.put(&key, Value::U64(3), &[]).await.expect("seed");
        let err = kv
            .put(
                &key,
                Value::U64(4),
                &[Condition::AttributeEquals {
                    attribute: String::new(),
                    expected: Value::U64(99),
                }],
            )
            .await
            .expect_err("wrong expected value must fail");
        assert!(matches!(err, CloudError::ConditionFailed { .. }));
        kv.put(
            &key,
            Value::U64(4),
            &[Condition::AttributeEquals {
                attribute: String::new(),
                expected: Value::U64(3),
            }],
        )
        .await
        .expect("correct expected value succeeds");
        assert_eq!(kv.get(&key).await.expect("get").value, Value::U64(4));
    }

    #[tokio::test]
    async fn atomic_update_increments_and_returns_post_state() {
        let kv = MemoryReplicatedKv::new();
        let key = Key::new("coord/counter");
        kv.put(&key, map_item([("next_index", Value::U64(0))]), &[])
            .await
            .expect("seed");
        let item = kv
            .atomic_update(
                &key,
                UpdateExpression::new().increment("next_index", 32),
                &[],
            )
            .await
            .expect("atomic_update succeeds");
        assert_eq!(item.value, map_item([("next_index", Value::U64(32))]));
    }

    #[tokio::test]
    async fn atomic_update_honors_epoch_condition() {
        let kv = MemoryReplicatedKv::new();
        let key = Key::new("coord/counter");
        kv.put(
            &key,
            map_item([("next_index", Value::U64(0)), ("epoch", Value::U64(1))]),
            &[],
        )
        .await
        .expect("seed");

        let stale = kv
            .atomic_update(
                &key,
                UpdateExpression::new().increment("next_index", 1),
                &[Condition::AttributeEquals {
                    attribute: "epoch".to_string(),
                    expected: Value::U64(2),
                }],
            )
            .await
            .expect_err("stale epoch must lose the CAS");
        assert!(stale.is_precondition_failure());

        kv.atomic_update(
            &key,
            UpdateExpression::new().increment("next_index", 1),
            &[Condition::AttributeEquals {
                attribute: "epoch".to_string(),
                expected: Value::U64(1),
            }],
        )
        .await
        .expect("current epoch succeeds");
    }

    #[tokio::test]
    async fn atomic_update_missing_key_is_not_found() {
        let kv = MemoryReplicatedKv::new();
        assert!(matches!(
            kv.atomic_update(&Key::new("missing"), UpdateExpression::new(), &[])
                .await,
            Err(CloudError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn atomic_update_increment_on_absent_attribute_is_condition_failed() {
        let kv = MemoryReplicatedKv::new();
        let key = Key::new("coord/counter");
        kv.put(&key, map_item([]), &[]).await.expect("seed");
        let err = kv
            .atomic_update(
                &key,
                UpdateExpression::new().increment("missing_attr", 1),
                &[],
            )
            .await
            .expect_err("increment on absent attribute fails");
        assert!(matches!(err, CloudError::ConditionFailed { .. }));
    }

    #[tokio::test]
    async fn query_returns_matching_prefix_sorted_by_key() {
        let kv = MemoryReplicatedKv::new();
        for k in ["coord/b", "coord/a", "other/x"] {
            kv.put(&Key::new(k), Value::Bool(true), &[])
                .await
                .expect("put");
        }
        let items = kv.query("coord/").await.expect("query");
        let keys: Vec<&str> = items.iter().map(|item| item.key.as_str()).collect();
        assert_eq!(keys, vec!["coord/a", "coord/b"]);
    }

    #[tokio::test]
    async fn query_with_no_matches_is_empty_ok() {
        let kv = MemoryReplicatedKv::new();
        assert_eq!(kv.query("nothing/").await.expect("query"), vec![]);
    }

    #[tokio::test]
    async fn transact_applies_all_ops_when_every_condition_holds() {
        let kv = MemoryReplicatedKv::new();
        let lease = Key::new("lease/primary");
        let counter = Key::new("coord/counter");
        kv.put(&lease, Value::U64(1), &[])
            .await
            .expect("seed lease");
        kv.put(&counter, map_item([("next_index", Value::U64(0))]), &[])
            .await
            .expect("seed counter");

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
        .expect("transact commits");

        assert_eq!(
            kv.get(&counter).await.expect("get").value,
            map_item([("next_index", Value::U64(32))])
        );
    }

    #[tokio::test]
    async fn transact_with_one_failing_condition_applies_zero_ops() {
        let kv = MemoryReplicatedKv::new();
        let lease = Key::new("lease/primary");
        let counter = Key::new("coord/counter");
        kv.put(&lease, Value::U64(1), &[])
            .await
            .expect("seed lease");
        kv.put(&counter, map_item([("next_index", Value::U64(0))]), &[])
            .await
            .expect("seed counter");

        let err = kv
            .transact(vec![
                Operation::Update {
                    key: counter.clone(),
                    expr: UpdateExpression::new().increment("next_index", 32),
                    conditions: vec![],
                },
                // This one fails: the lease's epoch is 1, not 99.
                Operation::ConditionCheck {
                    key: lease.clone(),
                    conditions: vec![Condition::AttributeEquals {
                        attribute: String::new(),
                        expected: Value::U64(99),
                    }],
                },
            ])
            .await
            .expect_err("transact must fail");
        assert!(matches!(err, CloudError::ConditionFailed { .. }));

        // Neither op applied — not even the one before the failing op.
        assert_eq!(
            kv.get(&counter).await.expect("get").value,
            map_item([("next_index", Value::U64(0))])
        );
        assert_eq!(kv.get(&lease).await.expect("get").value, Value::U64(1));
    }

    #[tokio::test]
    async fn transact_failing_op_first_still_applies_zero_ops() {
        let kv = MemoryReplicatedKv::new();
        let lease = Key::new("lease/primary");
        kv.put(&lease, Value::U64(1), &[]).await.expect("seed");

        let err = kv
            .transact(vec![
                Operation::ConditionCheck {
                    key: lease.clone(),
                    conditions: vec![Condition::AttributeEquals {
                        attribute: String::new(),
                        expected: Value::U64(99),
                    }],
                },
                Operation::Put {
                    key: Key::new("side-effect"),
                    value: Value::Bool(true),
                    conditions: vec![],
                },
            ])
            .await
            .expect_err("transact must fail");
        assert!(matches!(err, CloudError::ConditionFailed { .. }));
        assert!(matches!(
            kv.get(&Key::new("side-effect")).await,
            Err(CloudError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn transact_delete_and_put_are_all_or_nothing() {
        let kv = MemoryReplicatedKv::new();
        let a = Key::new("a");
        let b = Key::new("b");
        kv.put(&a, Value::U64(1), &[]).await.expect("seed a");

        let err = kv
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
            .await
            .expect_err("transact must fail");
        assert!(matches!(err, CloudError::ConditionFailed { .. }));

        // `a` was not deleted.
        assert_eq!(kv.get(&a).await.expect("get").value, Value::U64(1));
        assert!(matches!(kv.get(&b).await, Err(CloudError::NotFound { .. })));
    }

    proptest! {
        /// Metamorphic property: incrementing a `U64` attribute by `by`
        /// always lands on `start + by` (spec §19.2, §8.3 counter primitive).
        #[test]
        fn increment_action_adds_by_amount(start in 0u64..=u64::MAX / 2, by in 0u64..=u64::MAX / 2) {
            let existing = map_item([("count", Value::U64(start))]);
            let updated = apply_update_actions(
                &existing,
                &UpdateExpression::new().increment("count", by),
            )
            .expect("u64 attribute increments");
            prop_assert_eq!(updated, map_item([("count", Value::U64(start + by))]));
        }

        /// An `Increment` targeting a non-`U64` attribute is always
        /// `ConditionFailed`, never a panic or silent coercion.
        #[test]
        fn increment_on_non_u64_attribute_is_condition_failed(s in ".{0,16}") {
            let existing = map_item([("count", Value::String(s))]);
            let result = apply_update_actions(
                &existing,
                &UpdateExpression::new().increment("count", 1),
            );
            let is_condition_failed = matches!(result, Err(CloudError::ConditionFailed { .. }));
            prop_assert!(is_condition_failed);
        }

        /// `AttributeEquals` holds exactly when the stored and expected
        /// values are equal — no false positives or negatives.
        #[test]
        fn attribute_equals_matches_iff_values_are_equal(a in 0u64..1000, b in 0u64..1000) {
            let value = map_item([("x", Value::U64(a))]);
            let condition = Condition::AttributeEquals {
                attribute: "x".to_string(),
                expected: Value::U64(b),
            };
            let result = check_conditions(Some(&value), std::slice::from_ref(&condition));
            prop_assert_eq!(result.is_ok(), a == b);
        }
    }
}
