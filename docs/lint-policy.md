# Lint policy

> Spec: [`mtc-architecture-spec.md`](mtc-architecture-spec.md) §22.12 (linting
> setup), §22.13 (required CI checks). Related rules:
> [`no-unwrap-in-prod`](../.claude/rules/no-unwrap-in-prod.md),
> [`no-unsafe`](../.claude/rules/no-unsafe.md).

The compiler and clippy are agent guardrails (spec §6, §23): anything the lints
can catch, they must catch, so neither humans nor agents have to rely on
discipline. This page records the baseline, every deviation currently in
effect, and how to request a new one.

## Baseline

Lint levels live in the root [`Cargo.toml`](../Cargo.toml)
`[workspace.lints.*]` tables; member crates inherit them via
`[lints] workspace = true` (mandatory for every crate). Behavior knobs live in
[`clippy.toml`](../clippy.toml); formatting in [`rustfmt.toml`](../rustfmt.toml).

| Lint | Level | Why |
|---|---|---|
| `clippy::pedantic` | warn | Comprehensive stylistic checks |
| `clippy::nursery` | warn | Newer, sometimes opinionated lints |
| `clippy::cargo` | warn | Workspace and dependency hygiene |
| `rust::missing_docs` | warn | Every public item documented |
| `clippy::unwrap_used` | **deny** | No panics on the production path |
| `clippy::expect_used` | **deny** | Same |
| `rust::unsafe_code` | **forbid** | No unsafe anywhere (see exception policy) |

"warn" is not soft: CI runs
`cargo clippy --workspace --all-targets --all-features -- -D warnings`
(§22.13), which promotes every warning to a hard failure. Locally, `make lint`
runs the identical command.

## Deviations currently in effect

Each entry states what is relaxed, where, and why.

1. **`unwrap()` / `expect()` allowed in test code** — via
   `allow-unwrap-in-tests = true` and `allow-expect-in-tests = true` in
   `clippy.toml`. This is the documented exemption pattern: clippy skips
   `unwrap_used` / `expect_used` inside `#[test]` functions and `#[cfg(test)]`
   modules. Tests asserting on a `Result`/`Option` they just constructed gain
   nothing from `?`-propagation ceremony. Caveat: non-`#[test]` helper
   functions in integration-test files (`tests/*.rs`) are not auto-exempt; give
   those a scoped `#[allow(clippy::unwrap_used)]` with a one-line reason, never
   a file-wide allow. `crates/placeholder` carries a canary test
   (`unwrap_is_permitted_in_test_code`) that fails clippy if this exemption
   ever regresses.

2. **`clock::SystemClock::now` carries the one sanctioned
   `#[allow(clippy::disallowed_methods)]`** — `clippy.toml` disallows
   `std::time::SystemTime::now` / `std::time::Instant::now` (rule
   [`no-systemtime-now-in-prod`](../.claude/rules/no-systemtime-now-in-prod.md),
   spec §22.11; table seeded by the dev-fake-clock ticket). Some wrapper must
   ultimately read the ambient clock; `SystemClock` in `crates/clock` is that
   wrapper, and its scoped allow is the only one permitted in production code.
   Clippy has no test-exemption knob for this lint, so test code that
   genuinely needs ambient time uses the same scoped allow with a justifying
   comment.

3. **`avoid-breaking-exported-api = false`** (`clippy.toml`) — pedantic/nursery
   fixes are applied even when they would change exported signatures. Pre-1.0
   workspace: we want the strictest form of these lints now, not after the
   public API ossifies.

4. **`clippy::cargo_common_metadata` skips `publish = false` crates** — this is
   clippy's default (`cargo-ignore-publish = false`; despite the name, `true`
   would *force* linting of unpublishable crates). Internal crates such as the
   temporary `crates/placeholder` are not required to carry crates.io metadata
   (`description`, `keywords`, `categories`, ...).

5. **rustfmt: stable options only** — the toolchain is pinned to stable
   (`rust-toolchain.toml`), where unstable rustfmt options are silently
   ignored. Listing them would make `rustfmt.toml` claim more than
   `cargo fmt --check` enforces, so `imports_granularity`, `group_imports`,
   `wrap_comments`, etc. stay out until stabilized.

6. **`clippy::multiple_crate_versions` allowed workspace-wide**
   (`Cargo.toml [workspace.lints.clippy]`) — the lint inspects the full
   workspace dependency graph from every crate's session, so a single
   transitive dupe (the generated admin-api crates pull `syn` 2 and 3)
   fails every crate with a finding no crate can act on. Duplicate-version
   policy belongs to cargo-deny (`fnd-license-policy` ticket), which has a
   real allowlist and reports once, at the workspace level.

## Known gotchas

- **`clippy::duration_suboptimal_units` vs unstable constructors** — this
  nursery lint wants `Duration::from_days(400)`, but on the pinned stable
  toolchain `from_days`/`from_weeks` are still unstable (`duration_constructors`,
  E0658); only `from_mins`/`from_hours` (`duration_constructors_lite`) are
  stable. Spell day-scale durations as `Duration::from_hours(400 * 24)` —
  do not silence the lint and do not enable unstable features. (First hit in
  `crates/clock` test code.)

## The `unsafe_code` exception policy (PKCS#11 FFI)

`unsafe_code = "forbid"` cannot be overridden by an inner `#[allow]` — that is
the point. The sole sanctioned exception (§22.12) is a future PKCS#11 FFI
crate (the HSM boundary calls a C shared library). When that crate lands it
must:

1. **Not** inherit the workspace table. It declares its own `[lints]` tables
   replicating the workspace baseline, but with `unsafe_code = "deny"` instead
   of `forbid` (deny, unlike forbid, permits scoped allows).
2. Scope `#![allow(unsafe_code)]` to the FFI module only, with a `// SAFETY:`
   comment on every `unsafe` block.
3. Land with an ADR documenting the boundary (per
   [`document-decisions`](../.claude/rules/document-decisions.md)).

No other crate may take this exception.

## How to request a deviation

1. **Prefer fixing the code.** Most pedantic/nursery findings are real; a
   deviation is the last resort, not the tiebreaker.
2. **Scope it minimally**: `#[allow(clippy::...)]` on the single item or
   expression, never crate- or module-wide, with a same-line/adjacent comment
   giving the reason. Crate-wide allows or workspace-level changes to
   `[workspace.lints.*]` / `clippy.toml` additionally require an entry in the
   table below and — if the deviation is architectural (new lint relaxed
   everywhere, `unsafe_code` posture, MSRV-related) — an ADR in
   [`docs/adr/`](adr/).
3. **Ship it in the PR that needs it**, where the reviewer sees lint context
   and code together. PRs that broaden an allow beyond what they need are sent
   back.
4. **Never** silence `unwrap_used`, `expect_used`, or `unsafe_code` in
   production code; those denials are the contract. (`unsafe_code` has exactly
   one path — the PKCS#11 policy above.)

Workspace-level deviations are tracked in the
[table above](#deviations-currently-in-effect); item-scoped allows are tracked
by review, not centrally.
