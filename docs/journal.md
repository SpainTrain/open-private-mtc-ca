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

## 2026-07-23 — ca-clock: migrated acme-core's local clock seam to the shared clock crate

**Ticket**: mtc-crd
**PR**: —

Decisions:
- Deleted crates/acme-core/src/clock.rs (Clock/MonotonicMillis/MonotonicClock/ManualClock); acme-core now depends on crates/clock (path dep) and uses clock::Clock (Arc<dyn Clock>) at its AcmeState seam, clock::SystemClock in production (main.rs, examples/demo_client.rs), clock::FakeClock in tests/dev.
- NonceStore expiry moved from monotonic-Instant-elapsed MonotonicMillis to wall-clock SystemTime via clock::Clock::now(), matching the workspace Clock trait (spec §22.11: SystemTime-based only, no monotonic variant). TTL semantics unchanged (5-min default; exact-deadline and one-ms-past-deadline behavior still covered by tests).
- The scoped #[allow(clippy::disallowed_methods)] on Instant::now() that lived in acme-core's local seam is gone; crates/clock::SystemClock remains the workspace's one sanctioned ambient-time read site (rule no-systemtime-now-in-prod).
- Fixed two clippy::duration_suboptimal_units findings the migration surfaced in nonce.rs tests (Duration::from_millis(1_000/2_000) -> from_secs(1/2)).

Open questions:
- Discovered, not fixed (out of scope — crates/clock belongs to a different ticket): crates/clock/src/fake.rs's non-tokio `notify_waiters(&self) {}` stub fails clippy (unused_self, missing_const_for_fn) whenever clock builds with default (non-tokio) features — reproduces standalone via `cargo clippy -p clock --all-targets -- -D warnings`, with zero involvement from acme-core. Dormant under `cargo clippy --workspace --all-targets --all-features -- -D warnings` (feature unification turns clock's tokio feature on workspace-wide) but live under `scripts/agent-precheck.sh` / `scripts/verify-task.sh`, which invoke clippy without `--all-features` — both currently FAIL at the "workspace lint" step on this. Needs a follow-up bead against crates/clock.
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
## 2026-07-23 — dev-crr-replication-sim: added crates/dev-replicator (S3 CRR + DDB Global Tables replication simulator).

**Ticket**: dev-crr-replication-sim
**PR**: —

Decisions: scan-diff (not DynamoDB Streams) for DDB tailing + hidden-timestamp
conditional-write LWW (ADR-0004); one process = one directed link, replicating
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

## 2026-08-04 — cloud-test-suite: shared ObjectStore/ObjectLock/ReplicatedKv/Hsm conformance suites (mtc-2zb, mtc-8if, mtc-eef)

**Ticket**: mtc-2zb,mtc-8if,mtc-eef
**PR**: —

Decisions:
- New crates/cloud-test-suite (spec §9.7 factory-closure pattern: `Fn() -> Fut` where `Fut: Future<Output = S>`) generalizes the exact assertions cloud-memory previously ran inline in its src #[cfg(test)] blocks; cloud-memory's src now keeps only the tests that cannot be expressed through a trait boundary (object_lock's clock time-travel expiry case; hsm's ZeroizeOnDrop compile-time assertion; replicated_kv's proptest cases against the private apply_update_actions/check_conditions helpers), and crates/cloud-memory/tests/*_suite.rs wires the shared suites in against MemoryObjectStore/MemoryObjectLock/MemoryReplicatedKv/MemoryHsm.
- cloud-test-suite dev-depends on cloud-memory to self-validate (spec §9.7: the memory backend "proves the suite itself", crates/cloud-test-suite/tests/memory_conformance.rs), and cloud-memory dev-depends on cloud-test-suite to run the suites against itself -- an intentional Cargo dev-dependency cycle (supported: dev-deps sit outside the normal, non-test build graph, so no cycle exists at compile time for real artifacts).
- Ratified the two ReplicatedKv contract points memory-backend flagged for follow-up: cloud-types' existing rustdoc `# Errors` sections already resolve both (not truly ambiguous, just previously untested at the cross-backend level) -- atomic_update on a missing key is CloudError::NotFound (atomic_update mutates an existing item); transact's Update op on a missing key is CloudError::ConditionFailed, not NotFound (transact's documented error surface has no NotFound variant). Added test_atomic_update_missing_key_is_not_found and a new test_transact_update_on_missing_key_is_condition_failed (memory's original test suite didn't cover the transact case) to enforce both as cross-backend contract, not backend-specific behavior.
- Concurrency property tests (spec §19.2) moved from cloud-memory's deleted tests/replicated_kv_concurrency.rs into the suite as two proptest-driven cases (put-based NotExists CAS race and atomic_update-based epoch CAS race). Task count per round is now proptest-generated (4..=48 via proptest::test_runner::TestRunner, 6 rounds) rather than the original's hand-picked constant (100 tasks x 20 rounds), to keep the suite fast when a later ticket runs it against LocalStack DynamoDB rather than pure in-memory locking.
- Object_lock suite takes an injected Arc<dyn Clock> parameter (not SystemTime::now()) to compute retain_until instants -- clippy's disallowed-methods lint has no #[cfg(test)] exemption for rule no-systemtime-now-in-prod, so even test-support library code must go through Clock.
- All four run_*_suite functions and their private helpers needed explicit `F: Fn() -> Fut + Sync` / `Fut: Future<Output = _> + Send` bounds: clippy::future_not_send (nursery, promoted to error via -D warnings) fired on every async helper without them, since `&F` requires `F: Sync` to be Send. Fixed at the source (real bounds, not a suppression) -- the factories used in practice (closures over Arc<Clock>/Arc<FakeClock>) already satisfy Send+Sync trivially.
- Non-workspace deps added to cloud-test-suite: p256 0.13 (features ecdsa, pkcs8; version-pinned to match cloud-memory) as a real (non-dev) dependency, needed to verify Hsm suite signatures against the exported SPKI DER.

Open questions:
- (none new; the pre-existing clock crate non-tokio-feature clippy gotcha noted in the memory-backend/dev-crr-replication-sim journal entries was not reproduced during this ticket's `cargo clippy --workspace --all-targets --quiet -- -D warnings` runs -- appears already resolved on this worktree's base commit.)
## 2026-08-04 — mtclib-inclusion-proofs (+ folded mtc-qka.2): Merkle inclusion/consistency proofs and Subtree hardening

**Ticket**: mtclib-inclusion-proofs (folds in mtc-qka.2)
**PR**: —

Decisions:
- Proofs live in crates/mtc/src/proof/ (InclusionProof, ConsistencyProof,
  ProofError), implemented clean-room from the RFC 9162 constructions
  draft-ietf-plants-merkle-tree-certs-03 adopts: inclusion §2.1.3.1/§2.1.3.2,
  consistency §2.1.4.1/§2.1.4.2. Generation reads sibling hashes via the
  existing MerkleTree::subtree_hash (domain-separated MTH) — no second hashing
  path; verification recomputes with hash_node from tree/digest.rs.
- Verification validates shape BEFORE hashing (crypto crown-jewel): out-of-range
  index, non-monotonic sizes (old > new), and exact path-length mismatch are
  rejected as typed ProofError with no hashing and no panic. Inclusion uses the
  RFC fn/sn reconstruction; consistency the node/last two-root reconstruction.
  old==0 handled (empty tree is a prefix of everything; old_root must equal
  empty_root); old==new handled (roots must match).
- Post-review fix (crypto F1): the m==0 arm is checked before m==n, so the
  degenerate (0,0) pair must ALSO require new_root==empty_root — otherwise
  "same size => same root" is unenforced at size 0 and a garbage new_root rides
  an empty proof. Added the n==0 new_root==empty_root check in the m==0 arm plus
  a regression test (verify(proof(0,0,[]), empty_root, garbage) => RootMismatch).
- Proof wire format (clean-room TLS presentation, RFC 9162 §2.1.3/§2.1.4 shape):
  { uint64 size(s); NodeHash path<0..2^16-1> } with NodeHash = opaque[32].
  Round-trips through the mtc wire framework (TlsSerialize/TlsParse over the
  bounded TlsReader). The framework has no native uint64 yet, so u64 fields are
  composed locally from TlsReader::read_array::<8>() / write_bytes(&be_bytes)
  rather than editing the shared wire reader/writer — this deliberately avoids a
  duplicate-uint64 definition colliding at merge with the parallel
  checkpoint/tiles/log-entries beads. A shared uint64 wire primitive is filed as
  discovered work.
- crypto F3 (mtc-qka.3) minimum-length: the proof path vector has NO positive
  wire minimum — empty paths are valid (single-leaf inclusion; old==0/old==new
  consistency) — so no hand-enforced floor is added at parse; the exact semantic
  length is enforced in verify(). Documented in both codecs.

Subtree hardening (mtc-qka.2 — alignment/inversion invariant):
- Root cause (crypto-tree-primitives flag): Subtree::new enforced nothing and
  Subtree::len computed end-start, which wraps in release for an inverted range.
- Fix: Subtree::new now debug_assert!(start <= end) (panics in debug/test, so an
  inverted range is unconstructable in CI); Subtree::len uses saturating_sub so
  even a release-built inverted range yields 0, never a wrapped near-u64::MAX
  length. Added fallible Subtree::try_new (rejects inversion) and
  Subtree::try_aligned (rejects inversion + empty + non-power-of-two +
  misalignment), returning the new SubtreeError enum.
- Alignment-invariant decision: decompose_range keeps emitting blocks through the
  fast unchecked const new (its outputs are provably aligned), while any range
  derived from untrusted input must go through try_new/try_aligned. A property
  test asserts every decompose_range block round-trips through try_aligned.

Verification: cargo test -p mtc (107 unit + integration + doc tests) green;
cargo fmt --all --check clean; cargo clippy -p mtc --all-targets --all-features
(and default) -D warnings clean; workspace clippy --all-features clean; example
prove_and_verify issues+verifies an inclusion and a consistency proof and shows
tamper rejection.

Open questions:
- Wire framework lacks a native uint64 codec; proofs (and, imminently,
  checkpoint/tiles) compose it locally. Worth a shared primitive once the
  parallel crates/mtc beads merge — bead candidate.
- Kani harnesses for the proof primitives are a separate ticket
  (mtclib-kani-harnesses); property tests are the AC here (spec §19.2).
## 2026-08-05 — mtclib-tiles: TileCoord::path() emits the tlog-tiles serving path, not the spec §8.1 S3 storage key

**Ticket**: mtc-u7m
**PR**: —

Decisions:
- TileCoord::path() renders the canonical c2sp.org/tlog-tiles serving convention: tile/<L>/<N>[.p/<W>], with N as x-prefixed zero-padded 3-digit groups (e.g. tile/1/x001/x234/067, tile/0/003.p/232). This is a deliberate divergence from the ticket AC's literal example / spec §8.1's S3 storage layout (tiles/<L>/....tile). The ticket's own Out-of-Scope excludes S3 storage/fetch (storage-facade epic, §8), and 'per tlog-tiles convention' is the AC's primary instruction, so the read-path/serving address is the correct thing to model here. Crypto-tiles and qa-tiles both reviewed and accepted this.
- The tile bytes themselves are the bare W*32-hash concatenation (no length prefix, no embedded coordinate) exactly as tlog-tiles specifies; the coordinate (level, index, width) travels in the path.

Open questions:
- The storage-facade epic (§8) MUST bridge the two path conventions: it owns the S3 key scheme (tiles/<L>/...zero-padded.../....tile with fixed-width lexicographic ordering, §8.1) and needs a mapping from the serving TileCoord::path() form to its storage key. Neither this bead nor mtc provides that bridge; it belongs with the storage facade. Flagging so it is not silently assumed.
