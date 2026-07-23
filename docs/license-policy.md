# License and supply-chain policy

> Spec: [`mtc-architecture-spec.md`](mtc-architecture-spec.md) §6 (Language and
> Licensing — licensing constraints table), §22.13 (required CI checks).
> Related: [`lint-policy.md`](lint-policy.md) deviation 6 (duplicate-dependency
> policy delegated to cargo-deny), ticket `fnd-license-policy`.

This page encodes spec §6's licensing constraints as machine-enforced policy
and documents the `cargo deny check` and `cargo audit` required CI checks of
§22.13. Like [`lint-policy.md`](lint-policy.md), it records the baseline, the
concrete allowlists currently in effect, and how to change them.

## Why: permissive-only, copyleft banned

Spec §6 ("Approach to mtc-bridge") states the constraint plainly: DigiCert's
`mtc-bridge` reference implementation is **AGPL-3.0**, so it **cannot be
forked** — only read for design patterns, then clean-room reimplemented in
Rust. Spec §6's licensing table generalizes that one constraint into a
project-wide rule: every dependency this repository takes on must carry a
permissive license, full stop.

| Project | License | Usability |
|---|---|---|
| DigiCert mtc-bridge | **AGPL-3.0** | Cannot fork. Read for design patterns only. |
| `bwesterb/mtc` (Go) | BSD-3-Clause | Read for reference patterns; reimplement in Rust. |
| `qux-pqc` | BSD-3-Clause | FIPS-targeted PQ crypto (ML-DSA, ML-KEM, SLH-DSA). |
| RustCrypto crates | MIT/Apache-2.0 | ECDSA, SHA-2, X25519, etc. |
| Tokio, `axum`, `tracing` | MIT | Async runtime, HTTP, logging |
| `aws-sdk-rust`, AWS CDK | Apache-2.0 | AWS clients, IaC |
| `clap`, `serde`, `thiserror`, `eyre` | MIT/Apache-2.0 | CLI, serialization, errors |
| htmx | BSD-2-Clause | Vendored admin-UI asset |

(Reproduced from spec §6 for convenience; the spec table is authoritative —
if this page and the spec ever disagree, the spec wins and this page is
stale.)

## Enforcement mechanism

`deny.toml`'s `[licenses]` section is the enforcement point. **cargo-deny
0.18.4+ removed the `[licenses] deny` field entirely** — "all licenses are
denied unless explicitly allowed" is now the *only* mechanism the tool
offers. That is intentional and sufficient: AGPL-3.0 (and every other
copyleft license — the GPL, LGPL, and SSPL families) is banned by never
appearing in `deny.toml`'s `allow` list. There is no separate "deny" knob to
misconfigure or forget to update.

`deny.toml`'s `allow` list contains only licenses actually required by a
crate currently in the workspace's `--all-features` dependency graph (verify
with `cargo metadata --all-features`), plus `BSD-2-Clause` — kept even though
no *Rust crate* currently needs it, because spec §6 explicitly names it for
the vendored `htmx.min.js` admin-UI asset (see `SEC-SC-05` in
[`docs/security/review-checklist.md`](security/review-checklist.md)), which
`cargo deny` cannot see (it isn't a Cargo dependency).

## Files

| File | Purpose |
|---|---|
| [`deny.toml`](../deny.toml) | `cargo-deny` config: license allow-list, duplicate-dependency allowlist (`[bans]`), advisory-DB check (`[advisories]`). |
| [`.cargo/audit.toml`](../.cargo/audit.toml) | `cargo-audit` config: advisory ignore-list (starts and stays empty) plus the policy note on how to add an entry. |
| [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) | `cargo-deny` and `cargo-audit` required CI jobs (one job per check, per the file's existing per-check-job convention). |
| [`docs/ci.md`](ci.md) | Required-status-checks table and local reproduction commands. |

Both tools run the **same** RustSec advisory database (`cargo deny check
advisories` and `cargo audit` overlap deliberately) so a misconfiguration in
one doesn't silently drop advisory coverage — spec §22.13 lists both as
independent required checks, not as alternatives.

## Duplicate-dependency allowlist

[`lint-policy.md` deviation 6](lint-policy.md#deviations-currently-in-effect)
allows `clippy::multiple_crate_versions` workspace-wide, because clippy's
version of the lint can't express a scoped exception — it fires on every
crate in the workspace for a single transitive dupe, with no allowlist
mechanism. `deny.toml`'s `[bans]` section is the real policy:
`multiple-versions = "deny"` fails the check on *any* duplicate that isn't
named in `skip`, and every `skip` entry must give a reason.

`deny.toml` sets `[graph] all-features = true`, matching the `clippy`/`test`
CI jobs (both already run with `--all-features`) — so the checked graph
includes optional features like `mtc-admin-api-client`'s `rustls` backend
(alongside the default `native-tls`) and `mtc-admin-api-server`'s
`conversion` (frunk) feature, not just the default-feature set. That's more
duplicate families than a default-features-only graph would show, but the
alternative is a blind spot: a copyleft or vulnerable dependency hiding
behind a feature that `clippy`/`test` already compile but `cargo-deny` never
looked at.

Four duplicate families are allowlisted today (verify current state with
`cargo tree --workspace -e normal,build -d` for the default-feature view, or
`cargo deny check bans` itself, which prints the full inclusion path for
anything not covered by `skip`):

- **`syn` 2.x and 3.x** — the generated admin-api crates (`mtc-admin-api-server`
  via `async-trait`/`serde_derive`/`serde_repr`/`validator_derive`/`thiserror`,
  spec §17.5 openapi-generator output) resolve `syn` 3, while the rest of the
  workspace's proc-macro dependencies (`darling`, `futures-macro`,
  `tokio-macros`, `tracing-attributes`, `strum_macros`, ...) are still on
  `syn` 2. Both sides are proc-macro-only build-time dependencies.
- **`getrandom` 0.2.x, 0.3.x, and 0.4.x** — the RustCrypto `p256`/`ecdsa`/
  `elliptic-curve` chain (`acme-core`'s ES256 JWS verification, spec §10)
  depends on `rand_core` 0.6 → `getrandom` 0.2. `acme-core`'s own direct
  dependency and the workspace's `proptest` dev-dependency pull the newer
  `rand`/`rand_core` 0.9 line → `getrandom` 0.3. `tempfile` (transitively
  pulled by `native-tls` and `proptest`) depends on `getrandom` 0.4.
- **`r-efi` 5.x and 6.x** — a direct consequence of the `getrandom` split
  above: `getrandom` 0.3 and 0.4 each carry a target-conditional dependency
  on `r-efi` (a UEFI-firmware-interface shim), gated to `*-unknown-uefi`
  builds only. This project never builds for that target, so `r-efi` never
  actually compiles — it's a graph artifact of the `getrandom` skew, not a
  dependency this workspace uses.
- **`frunk_core` 0.4.x and 0.5.x** — reachable only through
  `mtc-admin-api-server`'s opt-in `conversion` feature (frunk-based DTO
  mapping). `frunk` 0.4.4 depends on `frunk_core` 0.4.4 directly, while
  `frunk-enum-derive`/`frunk_derives`/`frunk_proc_macros` (used by the same
  feature) depend on `frunk_core` 0.5.0 — a version skew inside the `frunk`
  crate family itself.

All four are tracked, not silenced: each `skip` entry pins a specific major
version and names exactly which crates pull which side, so `cargo deny check
bans` starts failing again — with a pointer to the stale entry — the moment
the actual dependency graph changes underneath it. When one resolves (a macro
crate ships a syn-3-compatible release; a RustCrypto crate ships a
rand_core-0.9-compatible release; `frunk` re-aligns its internal versions),
delete the corresponding `skip` pair(s) rather than leaving them stale.

**Do not** add a new `skip` entry to make a warning go away without following
this pattern: identify the real crates on each side with `cargo tree -i
<crate>@<version>`, write down *why* the duplicate exists and what resolves
it, and scope the version range as tightly as the actual duplicate (a bare
crate name with no version skips *every* future duplicate of that crate,
silently).

## Negative-test evidence

Per the ticket's Testing note, enforcement was verified locally: a scratch
crate declaring `license = "AGPL-3.0-only"` was added as a temporary `path`
dependency of `crates/placeholder` (chosen because it "intentionally has no
dependencies" and is never published), `cargo deny check licenses` was run,
and the edit (plus the resulting `Cargo.lock` churn) was reverted with
`git checkout -- Cargo.lock crates/placeholder/Cargo.toml` — nothing from the
negative test is committed.

```console
$ cargo deny check licenses
error[rejected]: failed to satisfy license requirements
  ┌─ /tmp/.../fake-agpl-crate/Cargo.toml:5:12
  │
5 │ license = "AGPL-3.0-only"
  │            ━━━━━━━━━━━━━
  │            │
  │            rejected: license is not explicitly allowed
  │
  ├ AGPL-3.0-only - GNU Affero General Public License v3.0 only:
  ├   - OSI approved
  ├   - FSF Free/Libre
  ├   - Copyleft
  ├ fake-agpl-crate v0.1.0
    └── mtc-placeholder v0.0.0

licenses FAILED
```

Gotcha worth recording: the scratch crate's manifest must **not** set
`publish = false` for this test. `deny.toml` sets `[licenses.private] ignore
= true` (deliberately, so first-party `crates/*` — all `publish = false` —
aren't re-checked against their own already-permissive `license.workspace`
value); that flag ignores *any* `publish = false` crate in the graph, not
just declared workspace members, so a scratch dependency marked
`publish = false` is silently skipped rather than rejected. This is
documented here so the next person re-running this test doesn't lose an
afternoon to a false pass. It is not a real gap in the policy: crates.io
rejects publishing a crate with `publish = false` in its manifest, so no
genuine third-party dependency can ever carry that flag — `private.ignore`
only ever exempts local-only crates (this workspace's own `crates/*`, and
`path`/`git` dependencies), never something actually pulled from the
registry.

Anyone can reproduce this: add any dependency whose license isn't in
`deny.toml`'s `[licenses] allow` list (and isn't `publish = false`), then run
`cargo deny check licenses` — it fails with the offending license and crate
named in the diagnostic, exactly as above.

## Adding a new license to the allow-list

1. Confirm the license is genuinely permissive (MIT/BSD/Apache-family, or
   comparably unrestrictive — not a copyleft license "with an exception").
2. Identify which crate(s) require it: `cargo metadata --all-features | jq
   -r '.packages[] | select(.license != null) | .license' | sort -u`, or read
   the failing `cargo deny check licenses` diagnostic — it names the crate.
3. Add the SPDX identifier to `deny.toml`'s `[licenses] allow` with a comment
   naming the crate(s) that need it (see the existing `MPL-2.0` /
   `CDLA-Permissive-2.0` entries for the pattern).
4. Update the table in this file if the addition reflects a spec §6 change
   (rare — spec §6 is the source of truth; a new permissive dependency
   license doesn't usually require a spec edit).

## What never changes

AGPL-3.0 and the rest of the copyleft families are not a "the allowlist
doesn't happen to include them yet" state — they are a deliberate, spec-level
constraint (§6). Adding one would require re-litigating spec §6 itself (an
ADR, not a `deny.toml` edit), not a config change to this policy.
