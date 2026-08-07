//! [`DynamoDbReplicatedKv`] -- [`ReplicatedKv`] over `aws-sdk-dynamodb`
//! (spec §9.3, §9.5).
//!
//! # Key mapping: one opaque `Key` -> `DynamoDB` `(PK, SK)`
//!
//! `cloud-types::Key` is a single opaque, `/`-segmented string (see its
//! rustdoc). The coordination-table schema this backend targets (spec §8.2,
//! `deploy/local/localstack/init/ready.d/01-init-mtc.sh`) is a classic
//! single-table design: `PK: String` (partition key), `SK: String` (sort
//! key). `crates/coordination` already renders its lease item key exactly
//! this way (`ddb_replicated_kv`'s sibling ticket, spec §8.2):
//! `Key::new(format!("log#{log_id}/{LEASE_SORT_KEY}"))` -- i.e. the domain
//! convention already *is* "partition key, then `/`, then sort key". This
//! module takes that convention as its contract: [`split_key`] splits a
//! rendered [`Key`] on its **first** `/` into `(PK, SK)`, and every method
//! round-trips through that split. A key with no `/`, or whose split would
//! leave the SK component empty, cannot be represented (`DynamoDB` forbids
//! empty-string key-attribute values) and is rejected with
//! [`CloudError::Transport`] (a construction-time, non-retryable failure --
//! never a panic).
//!
//! # Value encoding: one reserved `value` attribute, not flattened
//!
//! Every item stores the domain [`Value`] -- whatever its shape, scalar or
//! [`Value::Map`] -- nested whole under one reserved top-level attribute
//! (`VALUE_ATTR`), rather than flattening a `Map`'s entries onto the item's
//! top level. This is what lets whole-value conditions
//! (`Condition::AttributeEquals { attribute: "".into(), .. }`, used by the
//! shared suite against scalar items) and named-attribute conditions/updates
//! (used by `crates/coordination` against its `Value::Map` lease item) share
//! one uniform translation: "" addresses the reserved attribute itself,
//! anything else addresses a document path *into* it
//! (`#value.#attribute_name`) via `DynamoDB`'s native nested-path syntax. The
//! alternative -- flattening `Map` entries to real top-level attributes,
//! closer to spec §8.2's illustrative example -- was rejected: it cannot
//! express a whole-value condition/update on a `Map` item as a single
//! `DynamoDB` path, and that concrete production table layout is explicitly
//! out of this ticket's scope (mtc-lf7 Out of Scope: "DDB table schema for
//! batches/leases (storage-facade epic, §8.2)"). See `docs/journal.md`
//! (mtc-lf7 entry) for the full record of this decision.
//!
//! # `Increment` never auto-vivifies
//!
//! `DynamoDB`'s native `ADD`/`SET x = x + :n` both create a missing numeric
//! attribute from zero. [`UpdateAction::Increment`]'s contract is the
//! opposite: absent or non-`U64` is [`CloudError::ConditionFailed`]. Every
//! `Increment` therefore contributes an `attribute_exists(..) AND
//! attribute_type(.., :n)` clause to the request's `ConditionExpression` --
//! evaluated *before* the update applies -- rather than relying on the
//! update expression's own arithmetic to fail.
//!
//! # `atomic_update`: `NotFound` vs `ConditionFailed`
//!
//! [`ReplicatedKv::atomic_update`] must distinguish "no item at this key"
//! ([`CloudError::NotFound`]) from "item exists but a condition/increment
//! target failed" ([`CloudError::ConditionFailed`]) -- but a single
//! `DynamoDB` `ConditionExpression` failure (`ConditionalCheckFailedException`)
//! carries no such detail; every clause is `ANDed` into one pass/fail result.
//! This module always includes `attribute_exists(PK)` as the first clause
//! (so a missing item is guaranteed to fail the check, never silently
//! auto-created by the update actions), and on a
//! `ConditionalCheckFailedException` issues one strongly-consistent
//! follow-up `GetItem` purely to *classify* the already-final "nothing was
//! written" outcome. This costs one extra round trip only on the failure
//! path and only narrows the error's specificity -- the write decision
//! itself is unaffected (already atomic and final by the time the
//! conditional `UpdateItem` returns), so the follow-up read's own,
//! vanishingly-unlikely race (another writer changing existence in the gap)
//! can at worst mislabel one error variant as its sibling, never produce an
//! incorrect write. The success path needs no such read: `ReturnValues:
//! ALL_NEW` on the same `UpdateItem` call returns the true post-update state
//! atomically, in the same request that wrote it -- deliberately *not*
//! implemented via `TransactWriteItems` (whose
//! `ReturnValuesOnConditionCheckFailure` would solve the failure-path
//! disambiguation race-free, but which has no `ALL_NEW`-equivalent on
//! success, forcing a follow-up read on the success path instead -- the
//! wrong path to spend a race window on).
//!
//! # `query`: `Query` when the prefix pins a partition, `Scan` otherwise
//!
//! [`ReplicatedKv::query`] takes a plain string prefix with no schema
//! awareness. When `prefix` contains a `/`, everything it can match shares
//! one exact partition key (the text before that `/`) -- proven by
//! `prefix`'s own first `/` occurring at the same offset as any matching
//! key's, since a shorter string that is a *prefix* of a longer one agrees
//! character-for-character up to its own length. That case runs a `Query`
//! (`PK = :pk AND begins_with(SK, :skPrefix)`) -- the fast, intended access
//! pattern (spec §8.2's per-log partitioning). A `prefix` with no `/` is a
//! prefix of the *partition key itself* and can match many partitions, which
//! `Query` cannot express (it requires an exact `PK`); that case, and the
//! empty-prefix "everything" case, fall back to `Scan` with a
//! `begins_with(PK, ..)` filter (or no filter at all when `prefix` is
//! empty). Every path paginates via `LastEvaluatedKey` and results are
//! always explicitly re-sorted by the rendered key before returning, since
//! only the `Query` path's ordering guarantee lines up with "sorted by key"
//! for free.
//!
//! # Known limitation: `u64` overflow on `Increment` is not pre-checked
//!
//! `cloud-memory` pre-checks `current.checked_add(by)` and never persists an
//! overflowing sum. This backend's `Increment` guard only checks
//! *existence/type*, not range: `DynamoDB` numbers hold up to 38 digits, so
//! `current + by` always succeeds server-side even when the mathematical
//! result exceeds `u64::MAX`; decoding that sum back into [`Value::U64`]
//! then fails with [`CloudError::Transport`] instead of the write itself
//! failing with [`CloudError::ConditionFailed`]. Pre-checking would require
//! a consistent read before every increment, undermining the single-round-trip
//! atomicity this module otherwise achieves, to guard a case (a counter
//! already within ~1 of `u64::MAX`) with no realistic path in this system's
//! index/epoch counters. Documented, not fixed speculatively.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use async_trait::async_trait;
use aws_sdk_dynamodb::config::{BehaviorVersion, Builder, Credentials, Region};
use aws_sdk_dynamodb::primitives::Blob;
use aws_sdk_dynamodb::types::{AttributeValue, ConditionCheck, Delete, Put, ReturnValue, Update};
use aws_sdk_dynamodb::Client;
use cloud_types::{
    CloudError, Condition, Item, Key, Operation, ReplicatedKv, UpdateAction, UpdateExpression,
    Value,
};

use crate::error::{
    ddb_generic_error, ddb_is_update_condition_failed, map_put_item_error,
    map_transact_write_items_error,
};

/// Long-term static credentials for a non-IAM endpoint.
///
/// Mirrors `crate::config::StaticCredentials` exactly; kept as a separate
/// type (rather than reused across both SDKs) so this module's config
/// surface stays self-contained -- see the module's "new module" framing in
/// the mtc-lf7 ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamoDbCredentials {
    /// Access key ID.
    pub access_key_id: String,
    /// Secret access key.
    pub secret_access_key: String,
}

/// Configuration for [`DynamoDbReplicatedKv::new`].
///
/// Same shape and rationale as `S3Config` (`crate::config`): `endpoint_url`/
/// `credentials` are `Some` for `LocalStack` and `None` for real AWS, where
/// the SDK's standard endpoint resolution and credential provider chain
/// apply instead. Table lifecycle (creation, key schema) is out of this
/// crate's scope (mtc-lf7 Out of Scope: "DDB table schema for batches/leases
/// (storage-facade epic, §8.2)") -- this type only names an
/// already-existing table with a `(PK: S, SK: S)` key schema (see the
/// module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamoDbConfig {
    /// Table every [`DynamoDbReplicatedKv`] operation targets. Must already
    /// exist with a `(PK: String HASH, SK: String RANGE)` key schema.
    pub table: String,
    /// AWS region string. `LocalStack` accepts any region-shaped string.
    pub region: String,
    /// Explicit endpoint URL (`LocalStack`: `http://127.0.0.1:4566`). `None`
    /// uses the SDK's standard endpoint resolution (real AWS).
    pub endpoint_url: Option<String>,
    /// Explicit static credentials. `None` uses the SDK's standard
    /// credential provider chain (real AWS: IAM role / environment / ...).
    pub credentials: Option<DynamoDbCredentials>,
}

impl DynamoDbConfig {
    /// Convenience constructor for a `LocalStack` target: dummy static
    /// credentials and `us-east-1` -- mirrors `S3Config::localstack`
    /// (`DynamoDB`, unlike S3, needs no path-style-addressing flag).
    #[must_use]
    pub fn localstack(table: impl Into<String>, endpoint_url: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            region: "us-east-1".to_string(),
            endpoint_url: Some(endpoint_url.into()),
            credentials: Some(DynamoDbCredentials {
                access_key_id: "test".to_string(),
                secret_access_key: "test".to_string(),
            }),
        }
    }
}

/// Builds an `aws_sdk_dynamodb::Client` from `config` -- the one place a
/// vendor SDK config type is constructed (rule no-sdk-types-in-domain, spec
/// §22.8); stays inside this backend crate and never crosses the
/// `cloud-types` trait boundary.
fn build_client(config: &DynamoDbConfig) -> Client {
    let mut builder = Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(config.region.clone()));
    if let Some(endpoint_url) = &config.endpoint_url {
        builder = builder.endpoint_url(endpoint_url.clone());
    }
    if let Some(creds) = &config.credentials {
        builder = builder.credentials_provider(Credentials::new(
            creds.access_key_id.clone(),
            creds.secret_access_key.clone(),
            None,
            None,
            "cloud-aws-ddb-static",
        ));
    }
    Client::from_conf(builder.build())
}

/// Partition-key attribute name (spec §8.2).
const PK_ATTR: &str = "PK";
/// Sort-key attribute name (spec §8.2).
const SK_ATTR: &str = "SK";
/// Reserved top-level attribute holding the domain [`Value`] (see module
/// docs, "Value encoding").
const VALUE_ATTR: &str = "value";
/// Reserved bookkeeping attribute [`update_clauses`] touches when a
/// [`UpdateExpression`] has zero actions -- `UpdateItem` requires a
/// non-empty `UpdateExpression`, but "verify conditions/existence, change
/// nothing" is a legitimate `atomic_update` call (see module docs).
const NOOP_MARKER_ATTR: &str = "_touched";

/// `DynamoDB`-backed [`ReplicatedKv`] (spec §9.3).
///
/// Cheap to [`Clone`]: `aws_sdk_dynamodb::Client` is internally `Arc`-shared
/// (mirrors `S3ObjectStore`'s identical rationale) -- the sharing pattern
/// `Arc<dyn ReplicatedKv>` needs from the `Backend` factory (spec §9.4).
#[derive(Clone)]
pub struct DynamoDbReplicatedKv {
    client: Client,
    table: String,
}

impl DynamoDbReplicatedKv {
    /// Creates a `ReplicatedKv` targeting `config.table` via a client built
    /// from `config`. Does not verify the table exists or is configured
    /// correctly -- table lifecycle is out of this crate's scope (see the
    /// module docs).
    #[must_use]
    pub fn new(config: DynamoDbConfig) -> Self {
        Self {
            client: build_client(&config),
            table: config.table,
        }
    }
}

// ---------------------------------------------------------------------
// Key mapping
// ---------------------------------------------------------------------

/// Splits a rendered [`Key`] into `(PK, SK)` on its first `/` (see module
/// docs). Both halves must be non-empty -- `DynamoDB` forbids an empty
/// string as a key-attribute value.
fn split_key(key: &Key) -> Result<(String, String), CloudError> {
    let rendered = key.as_str();
    match rendered.split_once('/') {
        Some((pk, sk)) if !pk.is_empty() && !sk.is_empty() => Ok((pk.to_string(), sk.to_string())),
        _ => Err(CloudError::Transport {
            retryable: false,
            reason: format!(
                "{rendered:?}: DynamoDbReplicatedKv requires keys shaped \
                 `<partition>/<sort...>` with both segments non-empty (spec \
                 §8.2's `log#{{logId}}/{{item-kind}}` convention) -- this key \
                 has no `/`, or splits to an empty partition or sort segment"
            ),
        }),
    }
}

/// Renders a `(PK, SK)` pair back into the single opaque [`Key`] (inverse of
/// [`split_key`]).
fn render_key(pk: &str, sk: &str) -> Key {
    Key::new(format!("{pk}/{sk}"))
}

// ---------------------------------------------------------------------
// Value <-> AttributeValue
// ---------------------------------------------------------------------

/// Encodes a domain [`Value`] into a `DynamoDB` `AttributeValue`, recursing
/// through [`Value::Map`].
fn encode_value(value: &Value) -> AttributeValue {
    match value {
        Value::Bool(b) => AttributeValue::Bool(*b),
        Value::U64(n) => AttributeValue::N(n.to_string()),
        Value::String(s) => AttributeValue::S(s.clone()),
        Value::Bytes(b) => AttributeValue::B(Blob::new(b.clone())),
        Value::Map(m) => AttributeValue::M(
            m.iter()
                .map(|(k, v)| (k.clone(), encode_value(v)))
                .collect(),
        ),
    }
}

/// Decodes a `DynamoDB` `AttributeValue` back into a domain [`Value`].
///
/// Only the five shapes [`encode_value`] ever produces round-trip; anything
/// else (a `DynamoDB` type this module never writes, or a numeric string
/// that doesn't fit `u64`) is [`CloudError::Transport`] -- data this module
/// did not itself write, not a normal protocol outcome.
fn decode_value(attr: &AttributeValue) -> Result<Value, CloudError> {
    match attr {
        AttributeValue::Bool(b) => Ok(Value::Bool(*b)),
        AttributeValue::N(s) => s
            .parse::<u64>()
            .map(Value::U64)
            .map_err(|_| malformed(&format!("stored N {s:?} is not a valid u64"))),
        AttributeValue::S(s) => Ok(Value::String(s.clone())),
        AttributeValue::B(b) => Ok(Value::Bytes(b.clone().into_inner())),
        AttributeValue::M(m) => {
            let mut out = BTreeMap::new();
            for (k, v) in m {
                out.insert(k.clone(), decode_value(v)?);
            }
            Ok(Value::Map(out))
        }
        other => Err(malformed(&format!(
            "unsupported DynamoDB attribute type: {other:?}"
        ))),
    }
}

fn malformed(reason: &str) -> CloudError {
    CloudError::Transport {
        retryable: false,
        reason: format!("malformed stored item: {reason}"),
    }
}

/// Reads a required `AttributeValue::S` attribute named `attr` from `item`.
fn required_string_attr(
    item: &HashMap<String, AttributeValue>,
    attr: &str,
) -> Result<String, CloudError> {
    match item.get(attr) {
        Some(AttributeValue::S(s)) => Ok(s.clone()),
        Some(other) => Err(malformed(&format!(
            "attribute {attr:?} is not a String (got {other:?})"
        ))),
        None => Err(malformed(&format!("attribute {attr:?} is missing"))),
    }
}

/// Decodes a full `DynamoDB` item (as returned by `Query`/`Scan`) into an
/// [`Item`], reconstructing its [`Key`] from the item's own `PK`/`SK`.
fn decode_scanned_item(attrs: &HashMap<String, AttributeValue>) -> Result<Item, CloudError> {
    let pk = required_string_attr(attrs, PK_ATTR)?;
    let sk = required_string_attr(attrs, SK_ATTR)?;
    decode_item_attrs(render_key(&pk, &sk), attrs)
}

/// Decodes a `DynamoDB` item's attributes into an [`Item`] at the
/// already-known `key` (used when `key` was the request's own input, e.g.
/// `get`/`atomic_update`, rather than read back from the response).
fn decode_item_attrs(
    key: Key,
    attrs: &HashMap<String, AttributeValue>,
) -> Result<Item, CloudError> {
    let raw = attrs.get(VALUE_ATTR).ok_or_else(|| {
        malformed(&format!(
            "{key}: item is missing the reserved {VALUE_ATTR:?} attribute"
        ))
    })?;
    let value = decode_value(raw)?;
    Ok(Item { key, value })
}

// ---------------------------------------------------------------------
// Expression building
// ---------------------------------------------------------------------

/// `ExpressionAttributeNames` shape (`#placeholder` -> real attribute name).
type ExprNames = HashMap<String, String>;
/// `ExpressionAttributeValues` shape (`:placeholder` -> value).
type ExprValues = HashMap<String, AttributeValue>;

/// Accumulates `ExpressionAttributeNames`/`Values` placeholder aliases for
/// one `DynamoDB` request. A fresh instance per request -- each
/// `TransactWriteItems` item gets its own independent namespace, so
/// `transact` constructs one builder per [`Operation`], never shares one
/// across the batch.
#[derive(Default)]
struct ExprBuilder {
    names: ExprNames,
    values: ExprValues,
    next: u32,
    /// Cache for the (very hot -- referenced by nearly every clause) `value`
    /// attribute alias, so one request reuses a single placeholder for it
    /// rather than minting a fresh one per clause.
    value_alias: Option<String>,
}

impl ExprBuilder {
    /// Mints a fresh `#nN` placeholder for `name`.
    fn alias_name(&mut self, name: &str) -> String {
        let placeholder = format!("#n{}", self.next);
        self.next += 1;
        self.names.insert(placeholder.clone(), name.to_string());
        placeholder
    }

    /// Mints a fresh `:vN` placeholder for `value`.
    fn alias_value(&mut self, value: AttributeValue) -> String {
        let placeholder = format!(":v{}", self.next);
        self.next += 1;
        self.values.insert(placeholder.clone(), value);
        placeholder
    }

    /// The (cached) placeholder for the reserved [`VALUE_ATTR`] attribute.
    fn value_alias(&mut self) -> String {
        if let Some(alias) = &self.value_alias {
            return alias.clone();
        }
        let alias = self.alias_name(VALUE_ATTR);
        self.value_alias = Some(alias.clone());
        alias
    }

    /// Document path addressing `attribute` within the reserved value
    /// attribute: the attribute itself when `attribute` is `""` (whole-value
    /// semantics of `Condition::AttributeEquals`/`UpdateAction`), else a
    /// nested path into its top-level `Map`.
    fn value_path(&mut self, attribute: &str) -> String {
        let v = self.value_alias();
        if attribute.is_empty() {
            v
        } else {
            let a = self.alias_name(attribute);
            format!("{v}.{a}")
        }
    }

    /// Consumes the builder, returning `(names, values)` shaped exactly for
    /// `set_expression_attribute_names`/`set_expression_attribute_values`:
    /// `None` when the respective map is empty.
    ///
    /// ⚠️ Every call site must go through this (never
    /// `Some(builder.names)`/`Some(builder.values)` directly): `DynamoDB`
    /// rejects an *explicitly empty* `ExpressionAttributeNames`/`Values` map
    /// with `ValidationException` rather than treating it the same as
    /// omitting the parameter -- and a `ConditionExpression` built from only
    /// [`Condition::NotExists`]/[`Condition::Exists`] (e.g. the lease/epoch
    /// protocol's insert-only `acquire`) aliases a name (`PK`) but needs no
    /// value placeholder at all, so `values` is empty in exactly that
    /// legitimate, common case.
    fn finish(self) -> (Option<ExprNames>, Option<ExprValues>) {
        (
            (!self.names.is_empty()).then_some(self.names),
            (!self.values.is_empty()).then_some(self.values),
        )
    }
}

/// Renders one [`Condition`] as a `ConditionExpression` clause.
fn condition_clause(builder: &mut ExprBuilder, condition: &Condition) -> String {
    match condition {
        Condition::NotExists => {
            let pk = builder.alias_name(PK_ATTR);
            format!("attribute_not_exists({pk})")
        }
        Condition::Exists => {
            let pk = builder.alias_name(PK_ATTR);
            format!("attribute_exists({pk})")
        }
        Condition::AttributeEquals {
            attribute,
            expected,
        } => {
            let path = builder.value_path(attribute);
            let val = builder.alias_value(encode_value(expected));
            format!("{path} = {val}")
        }
    }
}

/// ANDs every condition's clause; `None` when `conditions` is empty (spec:
/// an empty conditions slice makes a `put` unconditional).
fn conditions_clause(builder: &mut ExprBuilder, conditions: &[Condition]) -> Option<String> {
    if conditions.is_empty() {
        return None;
    }
    Some(
        conditions
            .iter()
            .map(|c| condition_clause(builder, c))
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

/// Builds the full `UpdateExpression` string for `expr` (`SET`/`REMOVE`
/// clauses), plus one `attribute_exists(..) AND attribute_type(.., N)` guard
/// clause per [`UpdateAction::Increment`] target (see module docs,
/// "`Increment` never auto-vivifies").
///
/// Guarantees a non-empty `UpdateExpression` even when `expr` has zero
/// actions, by touching [`NOOP_MARKER_ATTR`] instead (see its doc comment).
fn update_clauses(builder: &mut ExprBuilder, expr: &UpdateExpression) -> (String, Vec<String>) {
    let mut set_clauses = Vec::new();
    let mut remove_clauses = Vec::new();
    let mut increment_guards = Vec::new();

    for action in &expr.actions {
        match action {
            UpdateAction::Set { attribute, value } => {
                let path = builder.value_path(attribute);
                let val = builder.alias_value(encode_value(value));
                set_clauses.push(format!("{path} = {val}"));
            }
            UpdateAction::Increment { attribute, by } => {
                let path = builder.value_path(attribute);
                let delta = builder.alias_value(AttributeValue::N(by.to_string()));
                set_clauses.push(format!("{path} = {path} + {delta}"));
                let num_type = builder.alias_value(AttributeValue::S("N".to_string()));
                increment_guards.push(format!(
                    "(attribute_exists({path}) AND attribute_type({path}, {num_type}))"
                ));
            }
            UpdateAction::Remove { attribute } => {
                remove_clauses.push(builder.value_path(attribute));
            }
        }
    }

    if set_clauses.is_empty() && remove_clauses.is_empty() {
        let marker = builder.alias_name(NOOP_MARKER_ATTR);
        let val = builder.alias_value(AttributeValue::Bool(true));
        set_clauses.push(format!("{marker} = {val}"));
    }

    let mut expr_str = String::new();
    if !set_clauses.is_empty() {
        expr_str.push_str("SET ");
        expr_str.push_str(&set_clauses.join(", "));
    }
    if !remove_clauses.is_empty() {
        if !expr_str.is_empty() {
            expr_str.push(' ');
        }
        expr_str.push_str("REMOVE ");
        expr_str.push_str(&remove_clauses.join(", "));
    }
    (expr_str, increment_guards)
}

/// Builds the full `ConditionExpression` for an `atomic_update`-shaped write
/// (plain `UpdateItem`, or a `transact` `Operation::Update`): always
/// requires the item to already exist, ANDs in the caller's own
/// `conditions`, and ANDs in every `Increment` action's existence/type
/// guard.
fn update_condition_expression(
    builder: &mut ExprBuilder,
    conditions: &[Condition],
    increment_guards: Vec<String>,
) -> String {
    let pk = builder.alias_name(PK_ATTR);
    let mut parts = vec![format!("attribute_exists({pk})")];
    if let Some(caller) = conditions_clause(builder, conditions) {
        parts.push(caller);
    }
    parts.extend(increment_guards);
    parts.join(" AND ")
}

// Error mapping lives in `crate::error` (imported at the top of this file),
// alongside S3's -- see that module's docs for why DynamoDB's classifiers
// are flat/context-free rather than Op-scoped.

// ---------------------------------------------------------------------
// ReplicatedKv
// ---------------------------------------------------------------------

#[async_trait]
impl ReplicatedKv for DynamoDbReplicatedKv {
    async fn get(&self, key: &Key) -> Result<Item, CloudError> {
        let (pk, sk) = split_key(key)?;
        let output = self
            .client
            .get_item()
            .table_name(&self.table)
            .key(PK_ATTR, AttributeValue::S(pk))
            .key(SK_ATTR, AttributeValue::S(sk))
            .consistent_read(true)
            .send()
            .await
            .map_err(|err| ddb_generic_error(&err))?;
        output.item().map_or_else(
            || {
                Err(CloudError::NotFound {
                    key: key.as_str().to_string(),
                })
            },
            |attrs| decode_item_attrs(key.clone(), attrs),
        )
    }

    async fn put(
        &self,
        key: &Key,
        value: Value,
        conditions: &[Condition],
    ) -> Result<(), CloudError> {
        let (pk, sk) = split_key(key)?;
        let mut builder = ExprBuilder::default();
        let condition_expr = conditions_clause(&mut builder, conditions);
        let mut req = self
            .client
            .put_item()
            .table_name(&self.table)
            .item(PK_ATTR, AttributeValue::S(pk))
            .item(SK_ATTR, AttributeValue::S(sk))
            .item(VALUE_ATTR, encode_value(&value));
        if let Some(expr) = condition_expr {
            let (names, values) = builder.finish();
            req = req
                .condition_expression(expr)
                .set_expression_attribute_names(names)
                .set_expression_attribute_values(values);
        }
        req.send()
            .await
            .map(|_| ())
            .map_err(|err| map_put_item_error(key.as_str(), &err))
    }

    async fn atomic_update(
        &self,
        key: &Key,
        expr: UpdateExpression,
        conditions: &[Condition],
    ) -> Result<Item, CloudError> {
        let (pk, sk) = split_key(key)?;
        let mut builder = ExprBuilder::default();
        let (update_expr_str, increment_guards) = update_clauses(&mut builder, &expr);
        let condition_expr =
            update_condition_expression(&mut builder, conditions, increment_guards);

        let (names, values) = builder.finish();
        let result = self
            .client
            .update_item()
            .table_name(&self.table)
            .key(PK_ATTR, AttributeValue::S(pk))
            .key(SK_ATTR, AttributeValue::S(sk))
            .update_expression(update_expr_str)
            .condition_expression(condition_expr)
            .set_expression_attribute_names(names)
            .set_expression_attribute_values(values)
            .return_values(ReturnValue::AllNew)
            .send()
            .await;

        match result {
            Ok(output) => output.attributes().map_or_else(
                || {
                    Err(malformed(&format!(
                        "{key}: UpdateItem ReturnValues=ALL_NEW returned no attributes"
                    )))
                },
                |attrs| decode_item_attrs(key.clone(), attrs),
            ),
            Err(err) if ddb_is_update_condition_failed(&err) => {
                // The write decision is already final (nothing was applied,
                // atomically) -- this follow-up read only classifies *why*
                // for the caller. See module docs, "atomic_update: NotFound
                // vs ConditionFailed".
                match self.get(key).await {
                    Err(CloudError::NotFound { key }) => Err(CloudError::NotFound { key }),
                    Ok(_) => Err(CloudError::ConditionFailed {
                        reason: format!("{key}: condition or increment precondition not satisfied"),
                    }),
                    Err(other) => Err(other),
                }
            }
            Err(err) => Err(ddb_generic_error(&err)),
        }
    }

    async fn transact(&self, ops: Vec<Operation>) -> Result<(), CloudError> {
        let mut items = Vec::with_capacity(ops.len());
        for op in &ops {
            items.push(build_transact_item(&self.table, op)?);
        }
        self.client
            .transact_write_items()
            .set_transact_items(Some(items))
            .send()
            .await
            .map(|_| ())
            .map_err(|err| map_transact_write_items_error(&err))
    }

    async fn query(&self, prefix: &str) -> Result<Vec<Item>, CloudError> {
        let mut items = if let Some((pk, sk_prefix)) = prefix.split_once('/') {
            self.query_within_partition(pk, sk_prefix).await?
        } else if prefix.is_empty() {
            self.scan(None).await?
        } else {
            self.scan(Some(prefix)).await?
        };
        items.sort_by(|a, b| a.key.as_str().cmp(b.key.as_str()));
        Ok(items)
    }
}

impl DynamoDbReplicatedKv {
    /// `Query`-based prefix match: everything under exact partition `pk`
    /// whose sort key begins with `sk_prefix` (or all of `pk`'s items when
    /// `sk_prefix` is empty). See module docs, "`query`".
    async fn query_within_partition(
        &self,
        pk: &str,
        sk_prefix: &str,
    ) -> Result<Vec<Item>, CloudError> {
        let mut out = Vec::new();
        let mut exclusive_start_key = None;
        loop {
            let mut builder = ExprBuilder::default();
            let pk_alias = builder.alias_name(PK_ATTR);
            let pk_val = builder.alias_value(AttributeValue::S(pk.to_string()));
            let mut key_condition = format!("{pk_alias} = {pk_val}");
            if !sk_prefix.is_empty() {
                let sk_alias = builder.alias_name(SK_ATTR);
                let sk_val = builder.alias_value(AttributeValue::S(sk_prefix.to_string()));
                // write! (not push_str(&format!(..))) avoids an extra
                // intermediate allocation; infallible for a String target.
                let _ = write!(key_condition, " AND begins_with({sk_alias}, {sk_val})");
            }
            let (names, values) = builder.finish();
            let mut req = self
                .client
                .query()
                .table_name(&self.table)
                .key_condition_expression(key_condition)
                .set_expression_attribute_names(names)
                .set_expression_attribute_values(values)
                .consistent_read(true);
            if let Some(esk) = exclusive_start_key.take() {
                req = req.set_exclusive_start_key(Some(esk));
            }
            let output = req.send().await.map_err(|err| ddb_generic_error(&err))?;
            for item in output.items() {
                out.push(decode_scanned_item(item)?);
            }
            exclusive_start_key = output.last_evaluated_key().cloned();
            if exclusive_start_key.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// `Scan`-based prefix match over the whole table: used when `prefix`
    /// does not pin one partition (see module docs, "`query`"). `pk_prefix
    /// = None` scans unfiltered ("" matches everything); `Some(p)` filters
    /// to partition keys beginning with `p`.
    async fn scan(&self, pk_prefix: Option<&str>) -> Result<Vec<Item>, CloudError> {
        let mut out = Vec::new();
        let mut exclusive_start_key = None;
        loop {
            let mut req = self.client.scan().table_name(&self.table);
            if let Some(prefix) = pk_prefix {
                let mut builder = ExprBuilder::default();
                let pk_alias = builder.alias_name(PK_ATTR);
                let pk_val = builder.alias_value(AttributeValue::S(prefix.to_string()));
                let (names, values) = builder.finish();
                req = req
                    .filter_expression(format!("begins_with({pk_alias}, {pk_val})"))
                    .set_expression_attribute_names(names)
                    .set_expression_attribute_values(values);
            }
            if let Some(esk) = exclusive_start_key.take() {
                req = req.set_exclusive_start_key(Some(esk));
            }
            let output = req.send().await.map_err(|err| ddb_generic_error(&err))?;
            for item in output.items() {
                out.push(decode_scanned_item(item)?);
            }
            exclusive_start_key = output.last_evaluated_key().cloned();
            if exclusive_start_key.is_none() {
                break;
            }
        }
        Ok(out)
    }
}

/// Builds one `TransactWriteItem` for `op`, targeting `table`.
fn build_transact_item(
    table: &str,
    op: &Operation,
) -> Result<aws_sdk_dynamodb::types::TransactWriteItem, CloudError> {
    use aws_sdk_dynamodb::types::TransactWriteItem;

    match op {
        Operation::Put {
            key,
            value,
            conditions,
        } => {
            let (pk, sk) = split_key(key)?;
            let mut builder = ExprBuilder::default();
            let condition_expr = conditions_clause(&mut builder, conditions);
            let mut put = Put::builder()
                .table_name(table)
                .item(PK_ATTR, AttributeValue::S(pk))
                .item(SK_ATTR, AttributeValue::S(sk))
                .item(VALUE_ATTR, encode_value(value));
            if let Some(expr) = condition_expr {
                let (names, values) = builder.finish();
                put = put
                    .condition_expression(expr)
                    .set_expression_attribute_names(names)
                    .set_expression_attribute_values(values);
            }
            let put = put
                .build()
                .map_err(|err| transact_build_error(key.as_str(), &err))?;
            Ok(TransactWriteItem::builder().put(put).build())
        }
        Operation::Update {
            key,
            expr,
            conditions,
        } => {
            let (pk, sk) = split_key(key)?;
            let mut builder = ExprBuilder::default();
            let (update_expr_str, increment_guards) = update_clauses(&mut builder, expr);
            let condition_expr =
                update_condition_expression(&mut builder, conditions, increment_guards);
            let (names, values) = builder.finish();
            let update = Update::builder()
                .table_name(table)
                .key(PK_ATTR, AttributeValue::S(pk))
                .key(SK_ATTR, AttributeValue::S(sk))
                .update_expression(update_expr_str)
                .condition_expression(condition_expr)
                .set_expression_attribute_names(names)
                .set_expression_attribute_values(values)
                .build()
                .map_err(|err| transact_build_error(key.as_str(), &err))?;
            Ok(TransactWriteItem::builder().update(update).build())
        }
        Operation::Delete { key, conditions } => {
            let (pk, sk) = split_key(key)?;
            let mut builder = ExprBuilder::default();
            let condition_expr = conditions_clause(&mut builder, conditions);
            let mut del = Delete::builder()
                .table_name(table)
                .key(PK_ATTR, AttributeValue::S(pk))
                .key(SK_ATTR, AttributeValue::S(sk));
            if let Some(expr) = condition_expr {
                let (names, values) = builder.finish();
                del = del
                    .condition_expression(expr)
                    .set_expression_attribute_names(names)
                    .set_expression_attribute_values(values);
            }
            let del = del
                .build()
                .map_err(|err| transact_build_error(key.as_str(), &err))?;
            Ok(TransactWriteItem::builder().delete(del).build())
        }
        Operation::ConditionCheck { key, conditions } => {
            let (pk, sk) = split_key(key)?;
            let mut builder = ExprBuilder::default();
            let condition_expr = conditions_clause(&mut builder, conditions).unwrap_or_else(|| {
                // ConditionCheck requires a ConditionExpression; an empty
                // cloud-types conditions list has no natural DynamoDB
                // encoding, so fall back to a tautology (never blocks the
                // transaction on this item).
                let pk_alias = builder.alias_name(PK_ATTR);
                format!("attribute_exists({pk_alias}) OR attribute_not_exists({pk_alias})")
            });
            let (names, values) = builder.finish();
            let check = ConditionCheck::builder()
                .table_name(table)
                .key(PK_ATTR, AttributeValue::S(pk))
                .key(SK_ATTR, AttributeValue::S(sk))
                .condition_expression(condition_expr)
                .set_expression_attribute_names(names)
                .set_expression_attribute_values(values)
                .build()
                .map_err(|err| transact_build_error(key.as_str(), &err))?;
            Ok(TransactWriteItem::builder().condition_check(check).build())
        }
    }
}

fn transact_build_error(key: &str, err: &aws_sdk_dynamodb::error::BuildError) -> CloudError {
    CloudError::Transport {
        retryable: false,
        reason: format!("{key}: failed to construct DynamoDB transact item: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv() -> DynamoDbReplicatedKv {
        DynamoDbReplicatedKv::new(DynamoDbConfig::localstack(
            "test-table",
            "http://127.0.0.1:4566",
        ))
    }

    // --- split_key / render_key --------------------------------------

    #[test]
    fn split_key_splits_on_first_slash_only() {
        assert_eq!(
            split_key(&Key::new("log#1/primary-region-lease")).unwrap(),
            ("log#1".to_string(), "primary-region-lease".to_string())
        );
        assert_eq!(
            split_key(&Key::new("cts/replicated-kv/query/coord/a")).unwrap(),
            ("cts".to_string(), "replicated-kv/query/coord/a".to_string())
        );
    }

    #[test]
    fn split_key_rejects_no_slash() {
        let err = split_key(&Key::new("no-slash-here")).unwrap_err();
        assert!(matches!(
            err,
            CloudError::Transport {
                retryable: false,
                ..
            }
        ));
    }

    #[test]
    fn split_key_rejects_empty_partition_or_sort() {
        assert!(split_key(&Key::new("/sort-only")).is_err());
        assert!(split_key(&Key::new("partition-only/")).is_err());
    }

    #[test]
    fn render_key_is_split_keys_inverse() {
        let key = Key::new("log#1/primary-region-lease");
        let (pk, sk) = split_key(&key).unwrap();
        assert_eq!(render_key(&pk, &sk), key);
    }

    // --- Value <-> AttributeValue round trips --------------------------

    #[test]
    fn value_round_trips_every_scalar_variant() {
        for value in [
            Value::Bool(true),
            Value::Bool(false),
            Value::U64(0),
            Value::U64(u64::MAX),
            Value::String(String::new()),
            Value::String("hello".to_string()),
            Value::Bytes(vec![]),
            Value::Bytes(vec![0, 1, 2, 255]),
        ] {
            let encoded = encode_value(&value);
            let decoded = decode_value(&encoded).unwrap_or_else(|err| {
                panic!("round trip of {value:?} should decode: {err}");
            });
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn value_round_trips_nested_map() {
        let value = Value::Map(BTreeMap::from([
            ("epoch".to_string(), Value::U64(7)),
            ("holder_id".to_string(), Value::String("inst-1".to_string())),
            (
                "nested".to_string(),
                Value::Map(BTreeMap::from([("flag".to_string(), Value::Bool(true))])),
            ),
        ]));
        let encoded = encode_value(&value);
        assert_eq!(decode_value(&encoded).unwrap(), value);
    }

    #[test]
    fn decode_value_rejects_non_u64_number_strings() {
        for bad in ["-1", "1.5", "not-a-number", ""] {
            let err = decode_value(&AttributeValue::N(bad.to_string())).unwrap_err();
            assert!(
                matches!(err, CloudError::Transport { .. }),
                "{bad:?}: {err:?}"
            );
        }
    }

    #[test]
    fn decode_value_rejects_unsupported_attribute_types() {
        let err = decode_value(&AttributeValue::Ss(vec!["a".to_string()])).unwrap_err();
        assert!(matches!(err, CloudError::Transport { .. }));
        let err = decode_value(&AttributeValue::Null(true)).unwrap_err();
        assert!(matches!(err, CloudError::Transport { .. }));
    }

    #[test]
    fn decode_item_attrs_requires_the_value_attribute() {
        let attrs = HashMap::from([(PK_ATTR.to_string(), AttributeValue::S("log#1".to_string()))]);
        let err = decode_item_attrs(Key::new("log#1/lease"), &attrs).unwrap_err();
        assert!(matches!(err, CloudError::Transport { .. }));
    }

    // --- expression building -------------------------------------------

    #[test]
    fn condition_clause_not_exists_and_exists_reference_pk() {
        let mut b = ExprBuilder::default();
        let clause = condition_clause(&mut b, &Condition::NotExists);
        assert_eq!(clause, "attribute_not_exists(#n0)");
        assert_eq!(b.names.get("#n0"), Some(&PK_ATTR.to_string()));

        let mut b = ExprBuilder::default();
        let clause = condition_clause(&mut b, &Condition::Exists);
        assert_eq!(clause, "attribute_exists(#n0)");
    }

    #[test]
    fn condition_clause_whole_value_attribute_equals_addresses_the_reserved_attribute() {
        let mut b = ExprBuilder::default();
        let clause = condition_clause(
            &mut b,
            &Condition::AttributeEquals {
                attribute: String::new(),
                expected: Value::U64(3),
            },
        );
        assert_eq!(clause, "#n0 = :v1");
        assert_eq!(b.names.get("#n0"), Some(&VALUE_ATTR.to_string()));
        assert_eq!(
            b.values.get(":v1"),
            Some(&AttributeValue::N("3".to_string()))
        );
    }

    #[test]
    fn condition_clause_named_attribute_equals_is_a_nested_path() {
        let mut b = ExprBuilder::default();
        let clause = condition_clause(
            &mut b,
            &Condition::AttributeEquals {
                attribute: "epoch".to_string(),
                expected: Value::U64(9),
            },
        );
        assert_eq!(clause, "#n0.#n1 = :v2");
        assert_eq!(b.names.get("#n0"), Some(&VALUE_ATTR.to_string()));
        assert_eq!(b.names.get("#n1"), Some(&"epoch".to_string()));
    }

    #[test]
    fn conditions_clause_empty_is_none() {
        let mut b = ExprBuilder::default();
        assert_eq!(conditions_clause(&mut b, &[]), None);
    }

    #[test]
    fn conditions_clause_multiple_are_anded() {
        let mut b = ExprBuilder::default();
        let clause = conditions_clause(
            &mut b,
            &[
                Condition::Exists,
                Condition::AttributeEquals {
                    attribute: "epoch".to_string(),
                    expected: Value::U64(1),
                },
            ],
        )
        .unwrap();
        assert!(clause.contains("attribute_exists(#n0)"));
        assert!(clause.contains(" AND "));
    }

    #[test]
    fn update_clauses_set_and_remove_are_grouped() {
        let mut b = ExprBuilder::default();
        let expr = UpdateExpression::new()
            .set("region", Value::String("us-east-1".to_string()))
            .remove("stale");
        let (expr_str, guards) = update_clauses(&mut b, &expr);
        assert!(expr_str.starts_with("SET "));
        assert!(expr_str.contains(" REMOVE "));
        assert!(guards.is_empty());
    }

    #[test]
    fn update_clauses_increment_produces_an_existence_and_type_guard() {
        let mut b = ExprBuilder::default();
        let expr = UpdateExpression::new().increment("next_index", 32);
        let (expr_str, guards) = update_clauses(&mut b, &expr);
        assert!(expr_str.starts_with("SET "));
        assert_eq!(guards.len(), 1);
        assert!(guards[0].contains("attribute_exists("));
        assert!(guards[0].contains("attribute_type("));
    }

    #[test]
    fn update_clauses_zero_actions_touches_the_noop_marker_not_the_domain_value() {
        let mut b = ExprBuilder::default();
        let (expr_str, guards) = update_clauses(&mut b, &UpdateExpression::new());
        assert!(guards.is_empty());
        assert!(expr_str.starts_with("SET "));
        // The marker's real attribute name is the internal bookkeeping
        // attribute, never a domain path through `value`.
        let marker_used = b.names.values().any(|n| n == NOOP_MARKER_ATTR);
        assert!(marker_used, "expected {NOOP_MARKER_ATTR:?} to be aliased");
    }

    #[test]
    fn update_condition_expression_always_requires_existence() {
        let mut b = ExprBuilder::default();
        let expr = update_condition_expression(&mut b, &[], vec![]);
        assert!(expr.starts_with("attribute_exists("));
    }

    #[test]
    fn update_condition_expression_includes_caller_conditions_and_increment_guards() {
        let mut b = ExprBuilder::default();
        let expr = update_condition_expression(
            &mut b,
            &[Condition::AttributeEquals {
                attribute: "epoch".to_string(),
                expected: Value::U64(2),
            }],
            vec!["(attribute_exists(#x) AND attribute_type(#x, :n))".to_string()],
        );
        // Three top-level clauses ANDed together: the mandatory existence
        // check, the caller's own condition, and the increment guard --
        // checked by structure (prefix/suffix/substring), not by a naive
        // split on " AND ", since the guard's own text also contains that
        // substring internally (between its two parenthesized sub-checks).
        assert!(expr.starts_with("attribute_exists(#n0) AND "), "{expr}");
        assert!(expr.contains("#n1.#n2 = :v3"), "{expr}");
        assert!(
            expr.ends_with("(attribute_exists(#x) AND attribute_type(#x, :n))"),
            "{expr}"
        );
    }

    // --- request shaping (no network) -----------------------------------

    #[test]
    fn kv_clone_shares_the_same_table() {
        let a = kv();
        let b = a.clone();
        assert_eq!(a.table, b.table);
    }
}
