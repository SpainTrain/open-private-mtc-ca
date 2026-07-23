//! [`ReplicatedKv`] — replicated key-value coordination state (spec §9.1).
//!
//! Backends: `DynamoDB` Global Tables / Firestore / Cosmos DB / Etcd /
//! Postgres+CDC / pure memory.
//!
//! This is the coordination substrate for the lease/epoch protocol and the
//! write path's linearization point (spec §8.3, §11). Three §9.5 capabilities
//! anchor the contracts here:
//!
//! - **Conditional KV writes** — atomic compare-and-swap on attributes
//!   (lease/epoch protocol).
//! - **KV transactional writes** — atomic multi-item update (linearization
//!   point of write-path step 8).
//! - **KV cross-region replication** — eventually consistent multi-region
//!   (coordination state replication).
//!
//! All DTOs are domain types; vendor SDK types (e.g.
//! `aws_sdk_dynamodb::types::AttributeValue`) never cross this boundary
//! (spec §22.8). The value model is deliberately minimal — extend it when a
//! concrete need appears (spec §9.8), never speculatively.

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;

use crate::errors::CloudError;

/// A KV item key (newtype — .claude/rules/use-newtypes).
///
/// Keys are opaque UTF-8 strings with `/`-separated segments by convention;
/// [`ReplicatedKv::query`] matches on plain string prefixes of the rendered
/// key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(String);

impl Key {
    /// Wraps a rendered key string.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Borrows the rendered key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the key, returning the rendered string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A KV attribute value.
///
/// A closed sum type (spec §22.3) covering the coordination-state needs of the
/// CA: counters and epochs (`U64` — `Index`/`TreeSize`/`Epoch` are `u64`
/// newtypes), identifiers (`String`), opaque blobs (`Bytes`), flags (`Bool`),
/// and structured items (`Map`). No floating point — coordination state never
/// needs it and it would forfeit `Eq`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Boolean flag.
    Bool(bool),
    /// Unsigned counter/epoch/index value.
    U64(u64),
    /// UTF-8 string.
    String(String),
    /// Opaque byte blob.
    Bytes(Vec<u8>),
    /// Nested attribute map; the top-level value of a structured item.
    /// `BTreeMap` keeps attribute iteration deterministic.
    Map(BTreeMap<String, Self>),
}

/// A stored item: a [`Key`] plus its current [`Value`].
///
/// Returned by [`ReplicatedKv::get`], [`ReplicatedKv::atomic_update`] (the
/// post-update state), and [`ReplicatedKv::query`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The item's key.
    pub key: Key,
    /// The item's value at read time.
    pub value: Value,
}

/// A precondition evaluated atomically with a write.
///
/// Conditions are the CAS primitive of the lease/epoch protocol (spec §9.5):
/// a write whose conditions do not all hold fails with
/// [`CloudError::ConditionFailed`] and has no effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// No item may exist at the target key (insert-only write).
    NotExists,
    /// An item must already exist at the target key.
    Exists,
    /// The attribute named `attribute` (a top-level entry of the item's
    /// [`Value::Map`], or the whole value when `attribute` is empty) must
    /// equal `expected` — the compare half of compare-and-swap.
    AttributeEquals {
        /// Attribute name within the item's top-level map; `""` addresses the
        /// item's whole value (for scalar items).
        attribute: String,
        /// The value the attribute must currently hold.
        expected: Value,
    },
}

/// One mutation within an [`UpdateExpression`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateAction {
    /// Set `attribute` to `value`, creating it if absent.
    Set {
        /// Attribute name within the item's top-level map.
        attribute: String,
        /// The new value.
        value: Value,
    },
    /// Add `by` to the [`Value::U64`] stored at `attribute` (the counter
    /// primitive behind index allocation — spec §8.3). Fails with
    /// [`CloudError::ConditionFailed`] if the attribute is absent or not a
    /// `U64`.
    Increment {
        /// Attribute name within the item's top-level map.
        attribute: String,
        /// The amount to add.
        by: u64,
    },
    /// Remove `attribute` from the item if present.
    Remove {
        /// Attribute name within the item's top-level map.
        attribute: String,
    },
}

/// An ordered list of [`UpdateAction`]s applied atomically to one item by
/// [`ReplicatedKv::atomic_update`].
///
/// ```
/// use cloud_types::{UpdateExpression, Value};
///
/// let expr = UpdateExpression::new()
///     .increment("next_index", 32)
///     .set("last_writer", Value::String("us-east-1".to_string()));
/// assert_eq!(expr.actions.len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateExpression {
    /// Actions applied in order, atomically as a whole.
    pub actions: Vec<UpdateAction>,
}

impl UpdateExpression {
    /// An empty expression; chain builder methods to add actions.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            actions: Vec::new(),
        }
    }

    /// Appends a [`UpdateAction::Set`] action.
    #[must_use]
    pub fn set(mut self, attribute: impl Into<String>, value: Value) -> Self {
        self.actions.push(UpdateAction::Set {
            attribute: attribute.into(),
            value,
        });
        self
    }

    /// Appends an [`UpdateAction::Increment`] action.
    #[must_use]
    pub fn increment(mut self, attribute: impl Into<String>, by: u64) -> Self {
        self.actions.push(UpdateAction::Increment {
            attribute: attribute.into(),
            by,
        });
        self
    }

    /// Appends a [`UpdateAction::Remove`] action.
    #[must_use]
    pub fn remove(mut self, attribute: impl Into<String>) -> Self {
        self.actions.push(UpdateAction::Remove {
            attribute: attribute.into(),
        });
        self
    }
}

/// One operation within a [`ReplicatedKv::transact`] transaction.
///
/// Every variant carries its own conditions; if any condition of any
/// operation fails, the entire transaction applies nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// Write `value` at `key` subject to `conditions`.
    Put {
        /// Target key.
        key: Key,
        /// Value to store.
        value: Value,
        /// Preconditions on the item at `key`.
        conditions: Vec<Condition>,
    },
    /// Apply `expr` to the item at `key` subject to `conditions`.
    Update {
        /// Target key.
        key: Key,
        /// Update actions to apply atomically.
        expr: UpdateExpression,
        /// Preconditions on the item at `key`.
        conditions: Vec<Condition>,
    },
    /// Delete the item at `key` subject to `conditions`.
    Delete {
        /// Target key.
        key: Key,
        /// Preconditions on the item at `key`.
        conditions: Vec<Condition>,
    },
    /// Assert `conditions` on the item at `key` without writing it — e.g.
    /// "the lease item still carries our epoch" guarding a commit (spec §11).
    ConditionCheck {
        /// Key whose item the conditions are evaluated against.
        key: Key,
        /// Preconditions that must hold for the transaction to commit.
        conditions: Vec<Condition>,
    },
}

/// Replicated key-value store with conditional and transactional writes.
///
/// Object-safe and shared as `Arc<dyn ReplicatedKv>` from the `Backend`
/// factory (spec §9.4); `Send + Sync` supertraits allow concurrent use across
/// tasks.
#[async_trait]
pub trait ReplicatedKv: Send + Sync {
    /// Reads the item at `key`.
    ///
    /// In-region reads observe the latest locally committed write
    /// (read-your-writes within the writing region).
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// KV cross-region replication: eventually consistent multi-region —
    /// remote-region reads may lag but converge; coordination state is
    /// replicated everywhere the CA can fail over to.
    ///
    /// # Errors
    ///
    /// - [`CloudError::NotFound`] — no item at `key`.
    /// - [`CloudError::Transport`] — transport/service failure (see
    ///   `retryable`).
    async fn get(&self, key: &Key) -> Result<Item, CloudError>;

    /// Writes `value` at `key` iff every condition in `conditions` holds.
    ///
    /// Condition evaluation and the write are a single atomic step; an empty
    /// `conditions` slice makes the put unconditional.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// Conditional KV writes: atomic compare-and-swap on attributes — the
    /// primitive the lease/epoch protocol builds on. Of N concurrent
    /// conditional puts racing on one key, at most one may succeed per state
    /// transition; "check then write" emulation without atomicity does not
    /// meet the bar.
    ///
    /// # Errors
    ///
    /// - [`CloudError::ConditionFailed`] — a condition did not hold; nothing
    ///   was written.
    /// - [`CloudError::Transport`] — transport/service failure.
    async fn put(
        &self,
        key: &Key,
        value: Value,
        conditions: &[Condition],
    ) -> Result<(), CloudError>;

    /// Atomically applies `expr` to the item at `key` iff every condition
    /// holds, returning the item's post-update state.
    ///
    /// This is the epoch-checked counter primitive (spec §8.3): e.g.
    /// increment `next_index` conditioned on `epoch == expected`.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// Conditional KV writes: condition check, update, and read-back are one
    /// atomic step; no interleaved writer can observe or produce a partial
    /// update.
    ///
    /// # Errors
    ///
    /// - [`CloudError::NotFound`] — no item at `key` (and `expr` requires
    ///   one).
    /// - [`CloudError::ConditionFailed`] — a condition did not hold (or an
    ///   [`UpdateAction::Increment`] targeted a non-`U64` attribute); nothing
    ///   was applied.
    /// - [`CloudError::Transport`] — transport/service failure.
    async fn atomic_update(
        &self,
        key: &Key,
        expr: UpdateExpression,
        conditions: &[Condition],
    ) -> Result<Item, CloudError>;

    /// Applies `ops` as a single all-or-nothing transaction.
    ///
    /// If any operation's conditions fail — or any operation cannot be
    /// applied — the whole transaction has no effect.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// KV transactional writes: atomic multi-item update — the linearization
    /// point of write-path step 8 (spec §11). Observers see either none or
    /// all of the transaction's writes, never a partial application.
    ///
    /// # Errors
    ///
    /// - [`CloudError::ConditionFailed`] — some operation's condition did not
    ///   hold; zero operations were applied.
    /// - [`CloudError::Transport`] — transport/service failure.
    async fn transact(&self, ops: Vec<Operation>) -> Result<(), CloudError>;

    /// Returns every item whose key starts with `prefix`, sorted by key.
    ///
    /// Backends with paginated APIs must drain pagination internally. An
    /// empty result is `Ok(vec![])`, not an error.
    ///
    /// # Capability bar (spec §9.5)
    ///
    /// KV cross-region replication: like [`ReplicatedKv::get`], in-region
    /// queries observe locally committed writes; remote regions converge.
    ///
    /// # Errors
    ///
    /// - [`CloudError::Transport`] — transport/service failure.
    async fn query(&self, prefix: &str) -> Result<Vec<Item>, CloudError>;
}

#[cfg(test)]
mod tests {
    use super::{Key, UpdateAction, UpdateExpression, Value};

    #[test]
    fn key_round_trips() {
        let key = Key::new("lease/primary");
        assert_eq!(key.as_str(), "lease/primary");
        assert_eq!(key.to_string(), "lease/primary");
        assert_eq!(key.into_string(), "lease/primary");
    }

    #[test]
    fn update_expression_builder_preserves_order() {
        let expr = UpdateExpression::new()
            .increment("next_index", 32)
            .set("region", Value::String("us-east-1".to_string()))
            .remove("stale_marker");
        assert_eq!(
            expr.actions,
            vec![
                UpdateAction::Increment {
                    attribute: "next_index".to_string(),
                    by: 32,
                },
                UpdateAction::Set {
                    attribute: "region".to_string(),
                    value: Value::String("us-east-1".to_string()),
                },
                UpdateAction::Remove {
                    attribute: "stale_marker".to_string(),
                },
            ]
        );
    }
}
