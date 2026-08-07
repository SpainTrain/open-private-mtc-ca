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
## 2026-08-05 — prune-retention-policy: added crates/retention (RetentionPolicy config + retain_until)

**Ticket**: prune-retention-policy
**PR**: —

Decisions:
- New crate `crates/retention`: `RetentionPolicyConfig` (serde `Deserialize`, 7-year default via `retention_days`, optional `dev_override_minutes`) validates and builds a `RetentionPolicy`; `RetentionPolicy::retain_until(ObjectClass, write_time: SystemTime) -> Result<SystemTime, RetentionError>` is the helper feeding `ObjectLock::put_with_retention` (spec §9.1, `crates/cloud-types`).
- `retain_until` takes `write_time` as an explicit `SystemTime` parameter rather than an injected `clock::Clock` — the AC's helper computes retain_until for a *given* write time, and the crate itself never reads wall-clock time, so adding a `Clock` dependency to production code would be unjustified coupling. Callers obtain `write_time` from their own injected `clock::Clock` (rule no-systemtime-now-in-prod is satisfied by construction, not by an added dependency).
- The indefinite sentinel for `ObjectClass::PruningCheckpoint` (spec §15.3: pruning checkpoints retained indefinitely) is a fixed calendar instant, `9999-12-31T23:59:59Z` (`253_402_300_799`s since `UNIX_EPOCH`) — not `write_time + Duration::MAX`. Verified via `SystemTime::checked_add` that adding `Duration::MAX` to a realistic write time overflows the platform's `SystemTime` representation rather than saturating, so a fixed, write-time-independent sentinel is required. Being independent of `write_time` also makes it trivially monotonic. An `ObjectLock` integration test against `cloud-memory` (`tests/objectlock_integration.rs`) proves a checkpoint written with this sentinel survives a 100-year fake-clock advance without becoming deletable.
- `RetentionDuration` (`crates/retention/src/duration.rs`) is a validated newtype (rule use-newtypes): `from_days`/`from_minutes` are the only constructors and both reject zero/negative inputs, so every `RetentionPolicy` in existence already holds a validated duration.
- `RetentionPolicyConfig::build` validates `retention_days` even when `dev_override_minutes` is set and becomes the effective duration, so a malformed production value can't hide behind an active dev override.
- Added a proptest property (`tests/retain_until_property.rs`, filter `retention_policy`) asserting `retain_until` is monotonic non-decreasing in write time for every object class, strictly monotonic for `Entry`/`Tile`, and constant for `PruningCheckpoint`.
- Verified: `cargo test -p retention` (25 unit + 2 integration + 3 property = 30 tests, 2 doctests) green; `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` (both with and without `--all-features`) clean; full `cargo test --workspace --lib` green; `make codemap-check` passes after regenerating `CODEMAP.md`.

Open questions:
- (none)

## 2026-08-05 — prune-checkpoint-format: PruningCheckpoint is flat content, no typestate, no Signature field

**Ticket**: mtc-jy9s
**PR**: —

Decisions:
- PruningCheckpoint (crates/mtc/src/pruning_checkpoint.rs) models spec §15.2's
  four fields only (pruned leaf-index range, tree_size, pruned_at timestamp,
  signing_key_id) as a single flat struct, deliberately NOT mirroring
  Checkpoint's Signed/Unsigned typestate. The AC lists no Signature field, and
  signing is an explicit separate bead (prune-checkpoint-signer); adding a
  typestate now would be speculative complexity with no second state to hold
  (a Signed variant would need a Signature field this ticket must not add). A
  future signer bead is expected to wrap this type (e.g. a generic Signed<T>)
  rather than this type growing an internal typestate.
- signing_key_id is a phantom-typed Id<SigningKeyTag> (spec §22.5), mirroring
  LogId/BatchId: String-backed, non-empty by construction, and hand-enforced
  non-empty again at wire parse (crypto F3) since the generic opaque<u16>
  reader admits a zero-length field. Modeled but never consumed — no signing,
  no key resolution.
- Range invariants (pruned_start <= pruned_end <= tree_size) are enforced in
  one shared validate_range() helper used by both the in-memory checked
  constructor (try_new, domain PruningCheckpointError) and TlsParse::tls_parse
  (mapped to WireError::InvalidValue per the log-entries precedent, not a new
  bespoke parse-error enum like Checkpoint's) — single source of truth, two
  error shapes for the two call sites.
- Reused the checkpoint.rs/proof.rs local write_u64/read_u64 pattern again
  (the wire framework still has no native uint64 primitive) rather than
  editing the shared reader/writer, per the existing multi-bead precedent —
  this is now the third independent local uint64 duplication in crates/mtc;
  a shared primitive is worth extracting as discovered work.
- Fuzz target (crates/mtc/fuzz, cargo-fuzz layout mirroring
  crates/acme-core/fuzz) plus a checked-in seed corpus
  (fuzz/corpus/parse_pruning_checkpoint/, 13 files) replayed under plain
  `cargo test -p mtc` via crates/mtc/tests/fuzz_corpus.rs, mirroring
  crates/acme-core/tests/fuzz_corpus.rs, so the never-panics property is
  checked on every PR without requiring nightly/cargo-fuzz.

Verification: cargo test -p mtc (13 new pruning_checkpoint tests + 1 new
fuzz_corpus.rs integration test; 196 total unit tests, all green); cargo fmt
--all --check clean; cargo clippy --workspace --all-targets --all-features -D
warnings clean; cargo clippy --all-targets --all-features -D warnings clean
inside crates/mtc/fuzz's own workspace. The fuzz binary itself (built on
stable, no cargo-fuzz/nightly available in this sandbox) was smoke-tested:
every corpus seed replayed individually with no panic, plus a real libFuzzer
mutation run (-max_total_time=30, ~24M executions) found zero crashes.

Open questions:
- No nightly toolchain / cargo-fuzz binary is installed in this environment,
  so `cargo fuzz run parse_pruning_checkpoint -- -max_total_time=60` (the
  ticket's literal demo command) could not be executed as specified. The
  equivalent underlying libFuzzer binary was run directly instead (see
  Verification). CI or a dev machine with cargo-fuzz should still run the
  literal command periodically per spec §19.3.

## 2026-08-05 — test-conformance-runner: JSON+hex conformance vector format, substring error_class matching, generator-not-hand-hex seeding

**Ticket**: test-conformance-runner
**PR**: —

Decisions:
- Vector format: one JSON file per vector under conformance/vectors/<kind>/, an internally-tagged Vector enum (kind = checkpoint | inclusion_proof | log_entry) so serde dispatches to a kind-specific fields/verify shape rather than a stringly-typed generic blob. wire_hex is lowercase hex, no 0x/separators.
- error_class (must-reject vectors) is matched as a substring of the actual error's {:?} (Debug) rendering, not full structural equality. Chosen so a vector names just the fired variant ("TrailingBytes", "RootMismatch", ...) without reproducing its offset/length payload, and so nested composite errors (CheckpointParseError::Wire(WireError::TrailingBytes{..})) still match on the inner variant name. Documented in conformance/vectors/README.md.
- Seed vectors are generator output, not hand-authored hex: crates/conformance/examples/generate_vectors.rs builds real Checkpoint/InclusionProof/LogEntry values via the actual mtc serializers (fixed RFC 6979 Appendix A.2.5 KAT key for reproducible signing) and writes the vector JSON directly. Reject vectors are either a small in-code mutation of real bytes (append/truncate/bit-flip) or, for the two vectors too short to come from any real Checkpoint (empty trust-anchor-id, non-UTF-8 log id), the exact byte sequences crates/mtc/src/checkpoint/mod.rs's own unit tests already assert correct.
- No new CI wiring needed: crates/conformance is an ordinary workspace member with an ordinary #[test], so it rides the existing `cargo test --workspace --all-features` required check (spec §22.13) automatically. mk/test.mk's pre-existing test-conformance stub now runs it (--nocapture, so the demo's per-vector pass/fail + total-count output always prints); added a `conformance` alias matching the ticket's literal demo wording.
- Byte-exact conformance against the draft-ietf-plants-merkle-tree-certs-03 text itself is explicitly out of scope here (bead mtc-qka.5, separate tracked obligation); this suite is clean-room only (self-consistency of our own serializer/parser), and is the harness mtc-qka.5's draft-derived vectors will slot into later — see conformance/README.md and vectors/README.md.

Verification: `cargo test -p mtc-conformance --all-features` green (14 lib unit tests + 2 integration tests + 0 doctests), prints "conformance suite: 10 passed, 0 failed, 10 total"; `cargo test --workspace --all-features` green; `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean; `make codemap` regenerated cleanly.

Open questions:
- `cargo deny check` bans currently fails on pre-existing duplicate crate-version findings (block-buffer/const-oid/cpufeatures/etc., p256 vs aws-sdk-s3's own transitive deps) unrelated to this ticket — predates this change (dev-replicator's aws-sdk deps), owned by dependency reconciliation.
- This worktree branched before wave-3a's mtclib-checkpoint/inclusion-proofs/log-entries/tiles merges landed on main; merged local main into this branch first to pick up the crates/mtc types this ticket needed to study and build against.

## 2026-08-05 — read-verify-core: mtc-verify relying-party inclusion verifier (§12.1 steps 4-6)

**Ticket**: read-verify-core
**PR**: —

New minimal-dependency `mtc-verify` crate: `verify_inclusion(entry, proof,
checkpoint, ca_pubkey) -> Result<Verified, VerifyError>` implementing spec §12.1
steps 4-6 (reconstruct leaf hash, apply inclusion proof, verify ECDSA P-256
checkpoint signature). Depends only on the core `mtc` crate.

Decisions:
- Two-crate split: `mtc-verify` is the MINIMAL relying-party verifier (depends
  only on `mtc` — no storage/service/async — so it embeds in RP code); the
  service-side tile planner lives in a separate `mtc-read` crate. Rejected a
  single read-path crate — it would pull service deps into the RP-embeddable
  verifier.
- `entry: &LogEntry` (not raw TBS bytes): the leaf hash is reconstructed via
  `mtc`'s canonical `LogEntry::leaf_hash`, which folds in the entry-type
  discriminant (null-entry unforgeability, draft §5.3). Cert parse/decode
  (steps 1-3) belong to read-verify-cert.
- proof<->checkpoint binding: an explicit SizeMismatch check binds
  proof.tree_size to checkpoint.tree_size, and the proof must reconstruct
  exactly checkpoint.root_hash — so a proof for a smaller historical tree cannot
  be replayed against a later checkpoint's root. Confirmed by crypto-read-path's
  adversarial oracle (signature checked unconditionally; root bound).
- log_id is deliberately NOT bound (crypto F2): a same-CA-key checkpoint for a
  DIFFERENT log over the same root verifies here. That binding is the
  certificate layer (§12.1 steps 1-3,7; read-verify-cert). Made explicit in the
  crate + function rustdoc and a boundary test; `Verified::log_id()` surfaces
  the checkpoint's log_id so the caller can perform the binding. signed_at is
  unauthenticated (draft §5.4.1) — documented as not-for-freshness.
- VerifyError variants map 1:1 to §20.2 telemetry reasons via a stable
  reason() label (wrong_root, bad_signature, malformed_proof, size_mismatch,
  malformed_entry, index_out_of_range). All signature failures (bad sig,
  wrong/malformed key, algorithm mismatch, un-encodable input) collapse to the
  single bad_signature bucket the AC names.

Open questions:
- log_id/trust-anchor binding and the signatureless landmark path are separate
  tickets (read-verify-cert, read-verify-signatureless) that consume this core.
- CODEMAP.md and Cargo.lock are regenerated centrally at merge (coordinator),
  not per worktree — flagged, not committed here.

## 2026-08-05 — read-tile-plan: mtc-read inclusion tile planner (§12.2 step 3)

**Ticket**: read-tile-plan
**PR**: —

New `mtc-read` read-path service crate with the pure planner
`plan_inclusion(tree_size, index) -> Result<TilePlan, PlanError>` (spec §12.2
step 3, 256-leaf tlog-tiles model). No storage I/O, no hashing.

Decisions:
- Crate placement: created `crates/mtc-read` — the epic's read-path SERVICE
  crate (matches the `-p mtc-read` demos in read-tile-plan and
  read-proof-gen-core). Kept separate from `mtc-verify` (the minimal RP
  verifier) per the two-crate split: service-side planner vs embeddable verifier.
- TilePlan shape: one `PathStep` per audit-path sibling (leaf-to-root), each a
  list of complete-subtree blocks (`TileSlotRun` = coord + slot + slot_count),
  ascending by leaf. Documented combination rule: each block is the balanced MTH
  of its contiguous slot-run; the sibling is the RIGHT-leaning combination of
  its blocks. Proved every audit-path sibling range decomposes into
  strictly-decreasing aligned blocks (so the right-fold is exact); the property
  test confirms it against `mtc`'s own proofs.
- No hashing/storage in this crate (kept in scope): hashing stays in `mtc`
  (tree-primitives), fetching is read-proof-gen-core. The plan is pure data.
- Reuses `mtc` tile geometry (`tiles_for_inclusion`, `tile_width`,
  `decompose_range`, `TileCoord`) rather than reinventing; a test asserts
  `plan.tiles()` equals `mtc::tiles_for_inclusion` for every leaf of every tree
  up to 300.
- `TileSlotRun::slot_count()` (not `len()`) — a run always has >= 1 slot, so a
  `len`/`is_empty` pair would be misleading (and clippy len_without_is_empty).
- Panic-free: index >= tree_size -> PlanError::IndexOutOfRange; all shifts
  guarded so any u64 tree_size is safe; impossible internal geometry ->
  PlanError::TileGeometry, never a panic (§19.8).

Open questions:
- 2^20 coverage: the property test runs proptest to N=2048, deterministic
  boundaries, and level-2 cases to N=2^18 (including the slot_count=2 multi-slot
  level-2 run, crypto F1). A literal N=2^20 sweep exists as an #[ignore]d test
  (`cargo test -- --ignored`, or a release CI lane) so the claim is falsifiable
  in-repo; it is out of the default gate only for debug-build speed.
- CODEMAP.md and Cargo.lock regenerated centrally at merge (coordinator).
## 2026-08-06 — softhsm-backend: cloud-softhsm implements cloud-types Hsm over PKCS#11/SoftHSM2 via safe cryptoki (no unsafe, no FFI exception); CKM_ECDSA over in-Rust SHA-256 -> 64-byte P1363 r||s (ADR-0003), SPKI export via RustCrypto, non-extractable keys, is_fips_validated=false. Integration tests (--features integration) verified live against real SoftHSM2. See ADR-0005.

**Ticket**: —
**PR**: —

Decisions:
- softhsm-backend: cloud-softhsm implements cloud-types Hsm over PKCS#11/SoftHSM2 via safe cryptoki (no unsafe, no FFI exception); CKM_ECDSA over in-Rust SHA-256 -> 64-byte P1363 r||s (ADR-0003), SPKI export via RustCrypto, non-extractable keys, is_fips_validated=false. Integration tests (--features integration) verified live against real SoftHSM2. See ADR-0005.

Open questions:
- (none)

## 2026-08-06 — aws-backend: cloud-aws crate — S3ObjectStore/S3ObjectLock reconciling versioned S3 with the single-object-per-key cloud-types contract

**Ticket**: mtc-xyn
**PR**: —

Decisions:
- ObjectStore::delete issues a versioned DeleteObject (HeadObject first for the current version_id), not a plain unversioned DELETE. A plain DELETE on a locked key would succeed via a delete marker while the locked bytes stay physically present -- the opposite of the spec 9.5 bar. The versioned delete lets S3 itself enforce retention (AccessDenied while locked -> RetentionViolation) against its own real clock, so expiry unblocks deletion correctly with no client-side clock needed.
- ObjectStore::put under PutMode::Overwrite refuses any key that has ever carried Object Lock retention (checked via GetObjectRetention before the write), rather than trying to re-derive has-retention-expired client-side. A plain PutObject never touches an existing version -- it just creates a new current one -- so S3 itself would never block an overwrite of a locked object. This is also correct product behavior: objects written via put_with_retention are append-only log content and are never legitimate Overwrite targets; pruning goes through delete (above), which does defer to S3's live enforcement.
- put_with_retention combines If-None-Match: * (create-only) and the Object Lock headers in one PutObject call -- atomic, no window where the object exists unretained.
- Error mapping (src/error.rs) is Op-scoped: the same S3 code (AccessDenied, InvalidRequest) means different things depending on which cloud-types operation triggered it, so classify() takes an Op enum rather than a single global code table.
- Two real (not LocalStack-specific) S3 platform characteristics, observed empirically running live against LocalStack 4.14.0 and documented in crate-level rustdoc: HeadObject reports missing-key errors as NotFound rather than NoSuchKey (no body to carry an XML code); Object Lock retain-until dates carry only second precision on the wire. Neither suite case is skipped -- the integration test injects a whole-second-truncating clock instead of clock::SystemClock so the round-trip assertion is meaningful at S3's actual precision.
- Integration tests (tests/support/mod.rs) provision a fresh, uniquely-named bucket per test run rather than reusing deploy/local's mtc-log-local bucket: that bucket carries a 1-day default Compliance retention rule (would retain every plain put, breaking test_delete_removes_object), and the ObjectLock suite's fixed key names would collide with still-locked objects from a previous run against a long-lived bucket.
- aws-sdk-s3 pinned to the exact same version spec/features as dev-replicator's existing dependency (version = 1, rt-tokio + rustls, no default features) -- Cargo.lock diff confirms zero new duplicate versions introduced.

## 2026-08-06 — mtc-586 (checkpoint-signer): owns the section 8.1 checkpoints/{tree_size:016}.signed OBJECT format (body = mtc TLS-presentation layout: TrustAnchorID log_id + u64 tree_size + HashValue root_hash + u64 signed_at + opaque signature<0..2^16-1>; addressed by tree_size, NEVER by signature bytes per ADR-0003 B.1/B.2). HSM-signature attach uses Option B (frame object + reuse Checkpoint::signature_input() canonicalization); typed Option A into_signed seam deferred to fast-follow bead. mtc-586 was dispatched on a stale worktree base (6c31848, pre-checkpoint) and re-cut from main 9aaf048.

**Ticket**: —
**PR**: —

Decisions:
- mtc-586 (checkpoint-signer): owns the section 8.1 checkpoints/{tree_size:016}.signed OBJECT format (body = mtc TLS-presentation layout: TrustAnchorID log_id + u64 tree_size + HashValue root_hash + u64 signed_at + opaque signature<0..2^16-1>; addressed by tree_size, NEVER by signature bytes per ADR-0003 B.1/B.2). HSM-signature attach uses Option B (frame object + reuse Checkpoint::signature_input() canonicalization); typed Option A into_signed seam deferred to fast-follow bead. mtc-586 was dispatched on a stale worktree base (6c31848, pre-checkpoint) and re-cut from main 9aaf048.

Open questions:
- (none)

## 2026-08-06 — mtc-kjl (EntryIntake seam, crates/ca-service): SourceType is an OPEN enum (NativeAcme | Adapter(String)) so future intake adapters are additions not edits (spec 10.4); EntryIntake is async-trait/dyn-compatible (Arc<dyn EntryIntake>) as the Stage-1/Stage-2 seam; its LogEntry is the pre-admission SUBMISSION envelope (spec 10.2), deliberately distinct from mtc::LogEntry (the Merkle tree-leaf) -- disambiguated in rustdoc. qa PASS, merged to main.

**Ticket**: —
**PR**: —

Decisions:
- mtc-kjl (EntryIntake seam, crates/ca-service): SourceType is an OPEN enum (NativeAcme | Adapter(String)) so future intake adapters are additions not edits (spec 10.4); EntryIntake is async-trait/dyn-compatible (Arc<dyn EntryIntake>) as the Stage-1/Stage-2 seam; its LogEntry is the pre-admission SUBMISSION envelope (spec 10.2), deliberately distinct from mtc::LogEntry (the Merkle tree-leaf) -- disambiguated in rustdoc. qa PASS, merged to main.

Open questions:
- (none)

## 2026-08-06 — mtc-gja (admin API core, crates/admin): axum app mounts the generated mtc-admin-api-server stubs and returns a bare prefix-agnostic Router; CA-service state injected as Arc<dyn CaStateProvider> (AppState seam) for in-memory testing; health served at /healthz,/readyz,/status (settled codegen decision, no /api prefix); ca-service binary mount deferred with a standalone 'cargo run -p mtc-admin' dev binary as interim. qa PASS, merged to main.

**Ticket**: —
**PR**: —

Decisions:
- mtc-gja (admin API core, crates/admin): axum app mounts the generated mtc-admin-api-server stubs and returns a bare prefix-agnostic Router; CA-service state injected as Arc<dyn CaStateProvider> (AppState seam) for in-memory testing; health served at /healthz,/readyz,/status (settled codegen decision, no /api prefix); ca-service binary mount deferred with a standalone 'cargo run -p mtc-admin' dev binary as interim. qa PASS, merged to main.

Open questions:
- (none)

## 2026-08-06 — mtc-brv6 (coordination lease/epoch): atomic takeover via ONE ReplicatedKv::atomic_update guarded by epoch_condition(E) -- no read-then-write race; epoch (reused mtc::Epoch) is the sole single-writer fence, strictly-monotonic via checked_next; safety margin additive per ADR-0007. Opus adversarial qa PASS (7 break attempts precluded, real single-winner proptest). DDB integration deferred to mtc-lf7; Kani harness present (CI-run). qa N1 invariant (lease item never deleted) captured as module-doc note + follow-up bead.

**Ticket**: —
**PR**: —

Decisions:
- mtc-brv6 (coordination lease/epoch): atomic takeover via ONE ReplicatedKv::atomic_update guarded by epoch_condition(E) -- no read-then-write race; epoch (reused mtc::Epoch) is the sole single-writer fence, strictly-monotonic via checked_next; safety margin additive per ADR-0007. Opus adversarial qa PASS (7 break attempts precluded, real single-winner proptest). DDB integration deferred to mtc-lf7; Kani harness present (CI-run). qa N1 invariant (lease item never deleted) captured as module-doc note + follow-up bead.

Open questions:
- (none)

## 2026-08-06 — mtc-586 (checkpoint-signer): write-path step-7 HSM signer, self-contained Option B (crates/mtc unmodified) -- signs mtc's signature_input() via Arc<dyn Hsm> (64-byte P1363 r||s, ADR-0003; non-64 -> terminal MalformedSignature), frames the §8.1 checkpoints/{tree_size:016}.signed object byte-identical to mtc TLS-presentation (parse oracle + universal proptest), retry/backoff via injected AsyncClock. Fable crypto PASS + qa PASS. mk/quality.mk bench stub filled with 'cargo bench --workspace' (qa: KEEP; testing epic to reconcile). Bench=MemoryHsm ~0.18ms; real-token p99 in cloud-softhsm test. signed_at unauthenticated by design (draft §5.4.1) -> note on mtc-1hp.5. Typed into_signed seam deferred to mtc-qka.12.

**Ticket**: —
**PR**: —

Decisions:
- mtc-586 (checkpoint-signer): write-path step-7 HSM signer, self-contained Option B (crates/mtc unmodified) -- signs mtc's signature_input() via Arc<dyn Hsm> (64-byte P1363 r||s, ADR-0003; non-64 -> terminal MalformedSignature), frames the §8.1 checkpoints/{tree_size:016}.signed object byte-identical to mtc TLS-presentation (parse oracle + universal proptest), retry/backoff via injected AsyncClock. Fable crypto PASS + qa PASS. mk/quality.mk bench stub filled with 'cargo bench --workspace' (qa: KEEP; testing epic to reconcile). Bench=MemoryHsm ~0.18ms; real-token p99 in cloud-softhsm test. signed_at unauthenticated by design (draft §5.4.1) -> note on mtc-1hp.5. Typed into_signed seam deferred to mtc-qka.12.

Open questions:
- (none)

## 2026-08-07 — mtc-lf7 (cloud-aws): DynamoDbReplicatedKv over aws-sdk-dynamodb -- PK/SK split-on-first-slash key mapping, reserved-attribute value encoding, ConditionalCheckFailedException->ConditionFailed verified against LocalStack incl. concurrent-CAS property tests

**Ticket**: mtc-lf7
**PR**: —

Decisions:
- Key mapping: cloud_types::Key (opaque, `/`-segmented string) is split on its FIRST `/` into DynamoDB's (PK, SK). Not an invented convention: crates/coordination's lease_key already renders `log#{logId}/primary-region-lease` this way (spec §8.2's PK/SK pattern), so DynamoDbReplicatedKv adopts that exact split as its universal Key<->(PK,SK) mapping. Keys with no `/`, or whose split leaves either segment empty, are rejected with CloudError::Transport{retryable:false} at the trait boundary (DynamoDB forbids empty-string key-attribute values) -- a documented backend limitation, not hit by coordination or the shared cloud-test-suite (both always render `/`-segmented keys).
- Value encoding: every item nests its whole cloud_types::Value (scalar or Map) under ONE reserved top-level attribute ("value"), rather than flattening a Map's entries onto real top-level attributes (closer to spec §8.2's illustrative counter/lease schema, but rejected: it cannot express a whole-value AttributeEquals/Increment on a Map item as one DynamoDB path, and that concrete production table layout is explicitly out of this ticket's scope). Named-attribute conditions/updates become DynamoDB document paths (`#value.#attr`); whole-value ones (attribute: "") address `#value` directly. Both forms are exercised by the shared cloud-test-suite (scalar items use whole-value AttributeEquals; epoch/lease-shaped Map items use named attributes) and pass unmodified.
- Increment never auto-vivifies: DynamoDB's native ADD / `SET x = x+:n` both create a missing numeric attribute from zero, but UpdateAction::Increment's contract requires ConditionFailed on an absent or non-U64 target. Every Increment contributes its own `attribute_exists(..) AND attribute_type(.., N)` clause to the ConditionExpression (evaluated before the update applies), so the update math itself never has to fail.
- atomic_update NotFound vs ConditionFailed: one DynamoDB ConditionExpression failure carries no per-clause detail, but the trait requires NotFound for a missing item and ConditionFailed for an existing item's failed condition/increment guard. `attribute_exists(PK)` is always ANDed in as the first clause (so a missing item can never be silently auto-created by the update actions); on ConditionalCheckFailedException, one strongly-consistent follow-up GetItem classifies the already-final "nothing was written" outcome -- the write decision itself is unaffected by the follow-up read's own (vanishingly unlikely) race. The success path needs no such read: ReturnValues=ALL_NEW on the same UpdateItem call returns the true post-update state atomically. Deliberately NOT implemented via TransactWriteItems (ReturnValuesOnConditionCheckFailure would solve the failure-path race-free, but has no ALL_NEW equivalent on success -- would force a follow-up read on the more important, success path instead).
- query: Query when the prefix pins one exact partition (contains a `/`, the intended per-log access pattern, spec §8.2), Scan+begins_with fallback otherwise (empty prefix, or a prefix that is itself a partial partition key). Every path paginates via LastEvaluatedKey; results always explicitly re-sorted by rendered key.
- Real DynamoDB/LocalStack gotcha found via integration failure, not obvious from the SDK docs: passing `Some(<empty HashMap>)` to `set_expression_attribute_values`/`set_expression_attribute_names` is NOT the same as omitting the parameter -- DynamoDB rejects it with ValidationException. A ConditionExpression built from only Condition::NotExists/Exists aliases a name (PK) but needs zero value placeholders, which hit this on the very first conditional put in the shared suite. Fixed via `ExprBuilder::finish() -> (Option<Names>, Option<Values>)`, used at all 8 request-building call sites; `Some(builder.names)`/`Some(builder.values)` directly is now a documented anti-pattern in that method's doc comment.
- Error mapping lives in crates/cloud-aws/src/error.rs alongside S3's, not a separate ddb_error.rs: DynamoDB's ConditionalCheckFailedException is unambiguous everywhere it can occur (unlike S3's context-sensitive AccessDenied), so the DynamoDB section is a flat, context-free classifier (map_put_item_error / ddb_is_update_condition_failed / map_transact_write_items_error / ddb_generic_error) rather than S3's Op-scoped classify(). TransactWriteItems failures are classified by scanning CancellationReasons for the (differently-spelled, no "Exception" suffix) "ConditionalCheckFailed" per-item code.
- Verified against real LocalStack DynamoDB, not just unit tests: the full shared cloud-test-suite ReplicatedKv conformance suite (including the concurrent-CAS property tests) passes unmodified via `cargo test -p cloud-aws --features integration --test replicated_kv_suite`, against a freshly-provisioned per-run table (tests/support/mod.rs::provision_test_table, same PK/SK schema as `01-init-mtc.sh`'s mtc-log-coordination table).

Open questions:
- crates/coordination/tests/lease_ddb.rs's placeholder sketch names a hypothetical `DynamoDbReplicatedKv::connect(endpoint, table)` async constructor; the actual API is `DynamoDbReplicatedKv::new(DynamoDbConfig)` (sync, mirrors S3Config/S3ObjectStore::new exactly -- does not verify the table exists, matching that method's own documented behavior). Wiring coordination's integration test to this backend is out of this ticket's scope (mtc-lf7 Out of Scope: "shared-suite CI wiring") -- the follow-up ticket should use the `new` + `DynamoDbConfig` shape, not `connect`.
- u64 overflow on Increment is not pre-checked (documented limitation in the module's rustdoc): DynamoDB numbers hold 38 digits so `current+by` always succeeds server-side even past u64::MAX; decoding the oversized sum back into Value::U64 then fails with CloudError::Transport instead of the write itself failing with ConditionFailed (cloud-memory's behavior). No realistic path in this system's index/epoch counters; pre-checking would need a consistent read before every increment, undermining the single-round-trip atomicity this backend otherwise achieves.
