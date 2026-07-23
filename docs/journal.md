# Decision Journal

Append-only log of decisions made while building the MTC CA. Future agents
(and humans) read this for context before re-deciding anything. See
`docs/mtc-architecture-spec.md` §23.5 (outer-loop tooling) and §23.7 (entry
template).

## Conventions

**When to journal**

- On task completion (a Beads ticket closed, a PR merged).
- Whenever you make a non-trivial decision — anything with a rejected
  alternative, a tradeoff, or that a future agent might re-litigate.
- When you leave open questions behind for future work (file a ticket, then
  record it here).

**How to append**

- `make journal msg="..."` appends a timestamped entry (see `mk/journal.mk`
  and `scripts/journal-append.sh`). Optional `ticket=...` and `pr=...` fill
  the template's metadata lines.
- The first line of `msg` becomes the entry title; any further lines become
  the entry body verbatim — supply your own `Decisions:` and
  `Open questions:` sections per the template. A single-line `msg` gets a
  minimal template body generated for it.
- Quoting and multiline text are safe: `msg` travels via the environment,
  so quotes, backticks, and newlines survive. Make-level caveat: make
  expands `$`, so write literal dollar signs as `$$`, or call
  `scripts/journal-append.sh "..."` directly.
- Never edit or reorder past entries; append a correcting entry instead.

**Ordering**

- Chronological, oldest first. New entries are appended at the end of this
  file, so the tail of the file is always the most recent context
  (`tail docs/journal.md`).

**Entry format** (the spec §23.7 template):

```markdown
## 2026-04-15 — Implemented epoch-aware counter UpdateItem

**Ticket**: STORAGE-3 (closed)
**PR**: #42

Decisions:
- Used DDB ConditionExpression on `epoch` rather than optimistic locking
- Rejected: in-memory counter cache (would violate single-writer guarantees)
- Used newtype `Epoch(u64)` rather than raw u64 for type safety
- Added Kani harness `verify_no_overlapping_allocations` to crates/storage/proofs/

Open questions:
- Counter contention at high batch rates — needs benchmarking (filed BENCH-1)
- Should we expose `Epoch` in public API? (filed for ADR)
```

---

## 2026-07-23 — Seeded decision journal and make-journal append tooling

**Ticket**: mtc-z2w
**PR**: —

Decisions:
- Entries are chronological, oldest first; appends go at the end so `tail docs/journal.md` always shows the latest context
- msg reaches scripts/journal-append.sh via the environment (GNU make auto-exports command-line variables), so quotes, backticks, and multiline text survive intact; literal dollar signs must be doubled per standard make escaping, or call the script directly
- First msg line becomes the entry title; remaining lines are the body verbatim, so callers can supply full Decisions / Open questions sections; single-line messages get a minimal template body
- All validation (msg present, journal exists and writable, date format) runs before any write, so a bad invocation leaves the journal untouched
- Rejected: newest-first ordering (would require rewriting the whole file on every append instead of a safe append-only write)
- Rejected: time-of-day in headings (spec section 23.7 template uses date-only headings; file order disambiguates same-day entries)
- Smoke tests live in scripts/journal-append-test.sh (`make journal-test`): ordering, timestamp format, quoting, rejected-invocation immutability, shellcheck when installed

Open questions:
- (none)

## 2026-07-23 — openapi-codegen-pipeline: seeded api/admin.openapi.yaml (/healthz, /readyz, /status) and built the §17.2 Rust codegen pipeline. Decisions: openapi-generator-cli 7.24.0 (pinned in api/openapitools.json; rust-axum for server stubs, rust/reqwest for the client) over progenitor because progenitor has no axum server-stub generator; Redocly CLI 2.40.0 (pinned in scripts/api-gen.sh) for spec lint; generated crates land under crates/ (admin-api-server, admin-api-client) because the workspace member glob is the sanctioned way to add members without editing the root Cargo.toml; generated Cargo.tomls carry a clippy allow block and README build-date lines are stripped so regeneration is byte-identical and fmt/clippy-clean; contract tests live in the separate hand-written crates/admin-api-tests so they survive regeneration.

**Ticket**: mtc-3mi
**PR**: —

Decisions:
- openapi-codegen-pipeline: seeded api/admin.openapi.yaml (/healthz, /readyz, /status) and built the §17.2 Rust codegen pipeline. Decisions: openapi-generator-cli 7.24.0 (pinned in api/openapitools.json; rust-axum for server stubs, rust/reqwest for the client) over progenitor because progenitor has no axum server-stub generator; Redocly CLI 2.40.0 (pinned in scripts/api-gen.sh) for spec lint; generated crates land under crates/ (admin-api-server, admin-api-client) because the workspace member glob is the sanctioned way to add members without editing the root Cargo.toml; generated Cargo.tomls carry a clippy allow block and README build-date lines are stripped so regeneration is byte-identical and fmt/clippy-clean; contract tests live in the separate hand-written crates/admin-api-tests so they survive regeneration.

Open questions:
- (none)

## 2026-07-23 — dev-crr-replication-sim: added crates/dev-replicator (S3 CRR + DDB Global Tables replication simulator).

**Ticket**: dev-crr-replication-sim
**PR**: —

Decisions: scan-diff (not DynamoDB Streams) for DDB tailing + hidden-timestamp
conditional-write LWW (ADR-0003); one process = one directed link, replicating
whichever of {S3 bucket, DDB table} are configured; lag/pause/stall live behind
a small local HTTP control endpoint, runtime-adjustable via a watch channel;
folded in the mr-replication-sim (multi-region epic) duplicate's extra AC
(runtime-adjustable lag incl. stall, documented LWW conflict semantics).
Extended deploy/local/ with docker-compose.replication-sim.yml, a Compose
override adding a second LocalStack instance -- the base docker-compose.yml
and local.env are untouched (verified: config diff empty, service list
unchanged). Verified live: tests/e2e/replication-sim-demo.sh (4/4 integration
tests) plus a manual dev-replicator binary run against two real LocalStack
containers exercising lag, Object Lock metadata preservation, runtime
stall/catch-up, and pause/resume.
Discovered (not fixed, out of scope): crates/clock/src/fake.rs non-tokio
notify_waiters() fails clippy::unused_self + missing_const_for_fn --
pre-existing, breaks 'make lint'/'make agent-precheck' workspace-wide
regardless of this change. Also tests/e2e/make-targets.sh stub_targets
still lists api-gen though openapi-codegen-pipeline already implemented it
(make-targets smoke FAILs on that one line, pre-existing). Neither touched
here; both worth their own beads.
