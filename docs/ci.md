# CI pipeline and required status checks

This page documents the repository's CI pipeline and the list of status checks
that branch protection on `main` should require. The authoritative list of
checks every PR must pass is spec §22.13 (`docs/mtc-architecture-spec.md`);
this page tracks which of them are implemented and what their status-check
names are.

## Workflow

- File: [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)
- Triggers: every pull request, and pushes to `main`.
- Concurrency: one group per workflow+ref — pushing a new commit to a PR or
  branch cancels the superseded in-flight run. Runs on `main` are never
  cancelled, so every mainline commit keeps a complete verdict.
- Toolchain: rustup installs the toolchain pinned in `rust-toolchain.toml`
  (with `rustfmt` and `clippy` components), so CI uses exactly the same
  compiler as local dev.
- Caching: `Swatinem/rust-cache` caches the cargo registry and `target/` per
  job, keeping a green run well under the ~10-minute budget.
- Pinning: all actions are pinned to exact commit SHAs with the release tag in
  a trailing comment (supply-chain hygiene). Bump the SHA and comment together.

## Required status checks (current)

Configure branch protection on `main` to require these checks. Each row is one
job in the `CI` workflow; the status-check name to select in branch protection
is the job name (PR checks display them as `CI / <job name>`).

| Check name | Command | Spec |
|------------|---------|------|
| `fmt`      | `cargo fmt --all --check` | §22.13 |
| `clippy`   | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | §22.13 |
| `test`     | `cargo test --workspace --all-features` | §22.13 |
| `doctest`  | `cargo test --workspace --doc` | §22.13 |
| `cargo-deny` | `cargo deny check` (license + advisory + duplicate-dependency + source checks; config `deny.toml`) | §22.13, §6 |
| `cargo-audit` | `cargo audit` (RustSec security advisories; config `.cargo/audit.toml`) | §22.13 |

Note: branch protection itself is repository configuration on GitHub and is
not applied by this repo's code. When enabling it, require all six checks
above and "Require branches to be up to date before merging".

License and supply-chain policy rationale (permissive-only allow-list, why
AGPL is banned, the duplicate-dependency allowlist): see
[`docs/license-policy.md`](license-policy.md).

## Planned required checks (owned by other tickets)

The remaining §22.13 checks land with their own tickets and must be appended
to the table above when they merge:

| Future check | Ticket (beads slug) |
|--------------|---------------------|
| API spec validation + breaking-change + codegen drift | `fnd-ci-api-spec-check` |
| `cargo kani` (path-filtered, critical-path crates only) | `fnd-ci-kani-gate` |
| infra/ lane: `tsc`, eslint, CDK assertions, `cdk synth` (path-filtered) | `fnd-ci-infra-checks` |
| Integration lane: `cargo test --features integration` vs LocalStack | `fnd-ci-integration-lane` |
| Custom dylint lints | `fnd-dylint-custom-lints` |

## Running the same checks locally

From the repo root (rustup picks up the pinned toolchain automatically):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --workspace --doc
cargo deny check
cargo audit
```

These are byte-for-byte the commands CI runs, so a locally green tree should
be CI-green modulo environment differences.
