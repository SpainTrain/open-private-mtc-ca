# ADR-0009: S3 Overwrite of an ever-retained key is backend-defined, not a conformance invariant

- **Status**: Accepted
- **Date**: 2026-08-07
- **Spec sections**: §9.5 (Object Lock retention bar), §9.6 (backend
  conformance suite), §9.3, §8 (append-only log). Related:
  `crates/cloud-aws` `S3ObjectStore`, `crates/cloud-test-suite`. Bead:
  `mtc-gyo.2`.

## Context

The `cloud-aws` S3 backend makes
[`PutMode::Overwrite`](cloud_types::PutMode::Overwrite) **permanently refuse any
key that has ever carried Object Lock retention** — an app-level
`GetObjectRetention` check before the write. It must: a plain S3 `PutObject`
never touches an existing version, it just creates a new *current* version, so
without this check it would silently "succeed" over a locked object while S3
rejects nothing — defeating the §9.5 bar ("cannot delete/overwrite during the
retention window, even by admins") from the write side.

`cloud-memory` has no such rule: its `Overwrite` is a straightforward replace.
So the two backends **diverge** on one case — `Overwrite` of a key that has
ever held retention — while the §9.6 conformance suite exists precisely to prove
backends are interchangeable. `mtc-gyo.2` asks how to resolve the divergence.

## Decision

**This behavior is backend-defined, and the shared conformance suite does not
assert on it.** `cloud_test_suite::run_object_lock_suite` /
`run_object_store_suite` cover the retention *contract* (a retained object
cannot be deleted or overwritten during its window; a plain `put`/`delete`
round-trips) but explicitly **exclude** `Overwrite` of a key that has *ever*
been retained — that outcome is left to each backend.

The justification is that this edge is **operationally unreachable**: objects
written via
[`ObjectLock::put_with_retention`](cloud_types::ObjectLock::put_with_retention)
are append-only log content by construction (§8) and are **never** legitimate
`PutMode::Overwrite` targets — pruning goes through
[`ObjectStore::delete`](cloud_types::ObjectStore::delete), which defers to S3's
live, time-accurate retention enforcement. S3 refusing `Overwrite` on a
once-retained key is therefore *safe strictness* the log never exercises;
forcing `cloud-memory` to replicate it would cost permanent per-key
"ever-retained" bookkeeping for a case that cannot legitimately occur.

## Alternatives Considered

### A. Reconcile in `cloud-memory` (make it refuse too)
Rejected. `cloud-memory` would have to retain, forever, the set of every key
that ever held retention — real memory + complexity — to make a case uniform
that no correct caller ever hits.

### B. Make S3 match `cloud-memory` (allow Overwrite once retention expires)
Rejected. It would need a client-side "has retention expired" computation, which
requires a locally-tracked `now` that production code cannot obtain outside the
injected `Clock` and which would disagree with S3's own clock anyway (see the
`s3_object_store` module docs). It is also simply wrong for append-only log
content.

## Consequences

### Positive
- The conformance suite stays honest about what it guarantees (the retention
  *contract*) without asserting a behavior that has no single right answer.
- No added state or complexity in `cloud-memory`; S3 keeps its safe strictness.

### Negative
- The `cloud-types` `ObjectStore::put` docs must state that **`Overwrite` of a
  key that has ever held Object Lock retention is backend-defined** (S3 refuses
  permanently; `cloud-memory` allows) — so no caller relies on it. Callers must
  not `Overwrite` retained keys (they never should — those keys are append-only).
- One documented gap in the "backends are interchangeable" story, bounded to an
  unreachable case and recorded here.
