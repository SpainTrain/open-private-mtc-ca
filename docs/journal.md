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

## 2026-07-23 — memory-backend: implemented crates/cloud-memory (all four cloud-types traits)

**Ticket**: mtc-sxy
**PR**: —

Decisions:
- MemoryObjectStore and the MemoryObjectLock alias are the *same* struct sharing one Arc<Mutex<BTreeMap>>: a client-side emulation of S3 Object Lock must share the object namespace with the store it locks, so ObjectStore::delete/put(Overwrite) can see retention set via ObjectLock::put_with_retention. Two independent structs would have let the two traits disagree about what is retained.
- atomic_update requires the item to already exist (NotFound if absent) rather than upserting an empty Map — the trait doc is ambiguous here; chose the stricter reading since real usage (epoch/counter items) always bootstraps via put first. transact's Update op instead maps a missing item to ConditionFailed, since transact's documented error surface has no NotFound variant. Both choices are implementation details the cloud-test-suite-kv ticket should validate/pin down.
- transact is two-pass (validate every op's conditions + compute planned writes against the pre-transaction snapshot, then apply) under one Mutex hold, guaranteeing all-or-nothing without any CAS/rollback machinery.
- MemoryHsm: RustCrypto p256 0.13 + getrandom 0.3 (pinned to match crates/acme-core's existing versions rather than p256 0.14, keeping the workspace's duplicate-version footprint down). Signature encoding is the required 64-byte P1363 r||s. Key zeroization relies on p256::ecdsa::SigningKey's built-in ZeroizeOnDrop (elliptic-curve's default "zeroize" feature) rather than a hand-rolled Drop/zeroize wrapper; pinned by a compile-time assert_zeroize_on_drop::<SigningKey>() test (zeroize crate added as a dev-dependency only, to name the trait).
- std::sync::Mutex (not tokio::sync::Mutex or parking_lot) for all interior state: guards are never held across an .await, and poisoning is recovered via unwrap_or_else(PoisonError::into_inner) rather than unwrap() (no-unwrap-in-prod).

Open questions:
- Discovered pre-existing, unrelated bug: crates/clock/src/fake.rs:161 (the non-tokio-feature notify_waiters stub) fails clippy::unused_self + clippy::missing_const_for_fn under the default (no --all-features) feature set — exactly what make agent-precheck/make verify-task run, so both currently FAIL on a clean checkout before this ticket's changes. Confirmed pre-existing (reproduced on an untouched crates/clock, unrelated to cloud-memory) and left unfixed here per single-pr-acceptance/smallest-change scope; needs its own bead (likely impl-clock-crate territory).
