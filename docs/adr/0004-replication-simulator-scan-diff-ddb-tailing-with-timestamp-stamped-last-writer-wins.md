# ADR-0004: Replication simulator: scan-diff DDB tailing with timestamp-stamped last-writer-wins

- **Status**: Accepted
- **Date**: 2026-07-23
- **Spec sections**: §18.3 (multi-region simulation locally), §8.1 (S3 CRR),
  §8.2 (DynamoDB Global Tables), §19.9 (chaos-crr-stall, chaos-ddb-lag),
  §22.8 (repository pattern boundary), §22.11 (the `Clock` trait)

## Context

Ticket `dev-crr-replication-sim` (spec §18.3) needs a local simulator that
copies S3 object versions and DynamoDB items between LocalStack instances
with configurable lag. A near-duplicate ticket, `mr-replication-sim`
(multi-region epic), was closed as a duplicate but added two requirements
this crate must also satisfy: per-link lag configurable **at runtime**,
including **stall** (infinite lag, for `chaos-crr-stall`), and **conflict
semantics (last-writer-wins) documented** for the DynamoDB side.

Three design forks needed resolving before writing code:

1. **How does the DynamoDB side discover changes?** The ticket's own AC
   explicitly offers a choice: "tails the source table (Streams or
   scan-diff)".
2. **How are conflicting writes to the same coordination-table item
   resolved**, given the mr-replication-sim AC requires this to be
   documented, not just implicit?
3. **How is "lag" measured and applied**, given the mr-replication-sim AC
   requires it to be runtime-adjustable per link, including an infinite-lag
   state?

## Decision

We will tail DynamoDB via **scan-diff** (a full `Scan` per poll cycle,
diffed against the previous scan's `(pk, sk)` set and a stable content
fingerprint per item), not DynamoDB Streams.

We will resolve conflicting DynamoDB writes with **last-writer-wins**,
implemented concretely as: every replicated write carries a hidden
bookkeeping attribute (`_dev_replicator_ts`, millis since the Unix epoch,
read from the replicator's own injected `Clock` at *apply* time) and both
`PutItem` and `DeleteItem` on the target carry a `ConditionExpression`
(`attribute_not_exists(#ts) OR #ts < :ts`) that only accepts the write if it
is strictly newer than whatever is already stored. A write that loses this
race is not an error — it is LWW working as designed (counted `stale`, not
`failed`).

We will make lag a small, generic, **infra-free** state machine
(`LagScheduler<K, E>` in `lag.rs`) shared by both the S3 and DynamoDB
pollers: each discovered change is tagged with a discovery timestamp (via
the injected `Clock`, never `SystemTime::now()` directly — rule
`no-systemtime-now-in-prod`), and a `LagPolicy` of either `Fixed(Duration)`
or `Stalled` (infinite lag) determines when it becomes due. The policy is
runtime-swappable via a `tokio::sync::watch` channel driven by the control
HTTP endpoint (`control.rs`), satisfying the runtime-adjustable and stall
requirements without any per-resource special-casing.

## Alternatives Considered

### DynamoDB Streams

Rejected for this dev-tooling scope. Streams add a second lifecycle to
manage (enabling the stream, `GetShardIterator`/`GetRecords` polling, shard
lifecycle, checkpointing per shard) on top of the lag/apply logic we need
regardless, and LocalStack's Streams emulation fidelity has historically
varied across community-edition releases in ways `ListObjectVersions` and
`Scan` (which this crate already depends on) do not. Scan-diff is simpler,
depends only on APIs this crate already uses elsewhere, and is fully
sufficient at dev-environment scale — polling a coordination table with at
most a few thousand items every few hundred milliseconds is cheap. The
trade-off (scan-interval-grained visibility: an item overwritten twice
between two polls is only ever seen in its final state) is documented as a
known limitation in `ddb.rs`, not hidden.

### Vector clocks / CRDT-style conflict resolution

Rejected as over-engineered for what this crate needs to simulate. Real
DynamoDB Global Tables itself resolves conflicts with last-writer-wins using
an internal timestamp — there is no stronger guarantee to emulate. The spec
is explicit that production usage never actually depends on this resolution
in practice (§8.2 commentary: "We use this with single-writer discipline
[the lease] so we never actually depend on conflict resolution") — so LWW is
not just the simplest option, it is the *faithful* one.

### One dedup key per `(pk, sk)`, ignoring content

Rejected. Unlike an S3 object version (immutable and globally unique per
`(key, version_id)`), a DynamoDB item's `(pk, sk)` is mutable — the same key
legitimately changes many times and every change must be considered for
replication. Keying the shared `LagScheduler`'s idempotency dedup on
`(pk, sk)` alone would mean only the *first* change at a key ever
replicates. Folding a content fingerprint into the dedup key
(`(pk, sk, fingerprint)`) instead lets the one generic scheduler handle both
resource kinds correctly: identical content re-discovered at a key is
idempotent (never re-queued), but new content at the same key is a distinct
key and queues normally.

### One `dev-replicator` process per resource kind (separate S3-only and DDB-only binaries)

Rejected. The ticket's own "Runs as N instances for arbitrary directed-link
topologies" and mr-replication-sim's "Runs as a compose sidecar" both frame
the unit of deployment as one process **per directed link** (source region →
target region), not per resource kind. One process instance replicates
whichever of {S3 bucket, DynamoDB table} are configured for that link via
`REPL_S3_BUCKET`/`REPL_DDB_TABLE` env vars, sharing one lag/pause control
surface. This halves the container count for a three-region topology
(`dev-multiregion-harness`) versus a per-resource-kind split, and matches
how a real region-to-region CRR relationship is one conceptual link carrying
multiple resource types.

## Consequences

### Positive

- No dependency on DynamoDB Streams emulation fidelity — the simulator works
  identically against any LocalStack edition that supports `Scan`.
- The lag/dedup/idempotency logic is written and tested exactly once
  (`lag.rs`, infra-free unit and property tests), reused unchanged by both
  the S3 and DynamoDB pollers.
- LWW conflict semantics are explicit and testable: a property/integration
  test can seed a "newer" write directly on the target and assert a stale
  replicated write is rejected (`ddb_last_writer_wins_rejects_a_stale_conflicting_write`).
- Runtime lag changes (including stall) require no restart and no
  per-resource plumbing — one `watch::Receiver<LagPolicy>` read once per poll
  cycle in `link.rs`.

### Negative

- Change visibility is scan-interval-grained: intra-interval write history on
  the DynamoDB side is lost (documented in `ddb.rs`).
- Every replicated DynamoDB item on the target carries one extra internal
  attribute (`_dev_replicator_ts`) the application never reads — a visible
  emulation gap versus real Global Tables, where the equivalent timestamp is
  internal and invisible to `GetItem`/`Scan` callers. Acceptable for a dev
  simulator; documented in `deploy/local/README.md` and the `ddb.rs` module
  docs alongside this repo's other documented LocalStack emulation gaps.
- Content fingerprinting is not attribute-filtered, so a table replicated
  through more than one hop (a future full-mesh topology) will see extra,
  harmless (idempotent) discovery events from its own bookkeeping attribute.
