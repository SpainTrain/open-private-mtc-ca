# ADR-0008: aws-sdk TLS stack upgrade off vulnerable rustls-webpki (aws-lc-rs)

- **Status**: Proposed
- **Date**: 2026-08-07
- **Spec sections**: `docs/mtc-architecture-spec.md` §14.4 (FIPS validation
  boundary), §22.13 (required CI checks — cargo-deny/cargo-audit). Bead
  **mtc-t92.3** (SECURITY). Prior art: ADR-0006 (`is_fips_validated()`
  semantics, the only other ADR touching §14.4). Advisories:
  RUSTSEC-2026-0098, RUSTSEC-2026-0099, RUSTSEC-2026-0104.

## Context

`rustls-webpki 0.101.7` — pulled transitively into `crates/cloud-aws` (the v1
production backend, spec §9.3: S3 `ObjectStore`/`ObjectLock`, DynamoDB
`ReplicatedKv`) and `crates/dev-replicator` (the dev-only CRR/Global-Tables
replication simulator, spec §18.3) — carries three RUSTSEC advisories:
name-constraint URI/wildcard bypass (RUSTSEC-2026-0098, -0099) and a
CRL-parse panic (RUSTSEC-2026-0104). Both crates pinned
`aws-sdk-s3`/`aws-sdk-dynamodb` with `default-features = false, features =
["rt-tokio", "rustls"]`.

The root cause is a feature-naming trap in the AWS SDK for Rust, confirmed via
`cargo info aws-sdk-s3` / `cargo info aws-smithy-runtime` /
`cargo info aws-smithy-http-client` against the exact pinned versions in this
workspace's `Cargo.lock`. The SDK's own `rustls` feature
(`aws-sdk-s3`/`aws-sdk-dynamodb` → `aws-smithy-runtime`'s `tls-rustls`
feature) does **not** mean "the rustls backend" — it means the *legacy*
hyper-0.14 client (`aws-smithy-http-client`'s `legacy-rustls-ring` feature),
which hard-pins `rustls 0.21.12` and therefore `rustls-webpki 0.101.7`. The
modern client — hyper 1.x + `hyper-util`, backed by rustls 0.23 with the
`aws-lc-rs` crypto provider — lives behind a differently named feature,
`default-https-client` (→ `aws-smithy-runtime`'s `default-https-client` →
`aws-smithy-http-client`'s `rustls-aws-lc` feature), which resolves
`rustls-webpki ≥0.102`. Both features exist side by side in the
already-pinned `aws-sdk-s3 1.x`/`aws-sdk-dynamodb 1.x` line; this is a feature
*selection* defect in this workspace's `Cargo.toml`, not a defect requiring a
newer SDK major version.

Confirmed by grep: neither `cloud-aws` nor `dev-replicator` source references
`hyper`, `rustls`, or any TLS/connector type directly. Both build their
`aws_sdk_*::config::Builder` explicitly (`.behavior_version
(BehaviorVersion::latest())`, `.endpoint_url(...)`, `Credentials::new(...)`),
never through `aws-config`'s ambient env-var defaulting — `aws-config` is not
even a dependency of this workspace.

Independently, `mtc-admin-api-client`'s existing `rustls` feature (a
`reqwest`-level feature, unrelated to and spelled the same as the AWS SDK's
misleadingly-named one) already pulled a *second*, modern rustls-webpki
(`0.103.13`, via `hyper-rustls 0.27.9` / `rustls 0.23.42`) into the
`--all-features` graph that `deny.toml`'s `[graph] all-features = true`
checks — so, prior to this change, the workspace already carried two
unrelated `rustls`/`rustls-webpki` stacks side by side (one vulnerable, one
not), an undocumented duplicate that predates this ticket.

## Decision

**We will switch both `aws-sdk-s3` and `aws-sdk-dynamodb` in
`crates/cloud-aws/Cargo.toml` and `crates/dev-replicator/Cargo.toml` from the
SDK's `rustls` feature to its `default-https-client` feature**, keeping
`default-features = false` and the existing `rt-tokio` feature otherwise
unchanged:

```toml
aws-sdk-s3 = { version = "1", default-features = false, features = ["rt-tokio", "default-https-client"] }
aws-sdk-dynamodb = { version = "1", default-features = false, features = ["rt-tokio", "default-https-client"] }
```

No `Cargo.toml` version-requirement change was needed — `version = "1"` was
already unconstrained within the 1.x line that carries the fix.
`cargo update -p aws-sdk-s3 -p aws-sdk-dynamodb` picked up the small,
incidental transitive patch bumps needed to re-resolve
(`aws-sdk-s3` 1.139.0→1.141.0, `aws-sdk-dynamodb` 1.118.0→1.120.0,
`aws-runtime` 1.9.0→1.9.1); nothing else in the workspace changed version.

**We will make no source-code changes.** Both crates already construct their
client config explicitly (see Context), so the change is confined to
`Cargo.toml`/`Cargo.lock`.

**We will update `deny.toml`'s comment (not its structure)** for the
`aws-sdk-s3`/`aws-sdk-dynamodb` `skip-tree` entries in `[bans]`. Verified
empirically (temporarily emptying `skip-tree` and re-running `cargo deny
check bans`): the entries are still required, but now for a narrower reason
than before. What they used to suppress — the entire legacy hyper-0.14/
rustls-0.21/`rustls-webpki-0.101.7` duplicate stack — is gone outright (not
just skip-tree'd): `cargo tree -i rustls-webpki --all-features` shows exactly
one resolved version, `0.103.13`, workspace-wide, and it now unifies with the
version `mtc-admin-api-client` already used (see Context) rather than
duplicating it. What remains under the skip-tree, confirmed by `cargo tree
-i <crate>`, is unrelated to TLS: (1) the AWS SDK's own SigV4 request-signing
RustCrypto generation (`sha2`/`hmac`/`sha1`/`digest`/`block-buffer`/
`crypto-common`/`const-oid`/`cpufeatures`, pulled via `aws-sigv4` and
`aws-smithy-checksums`), duplicating the main crypto path's `p256 0.13`/
`sha2 0.10` stack; and (2) an internal `http 0.2.x`/`http-body 0.4.x`
compatibility layer `aws-smithy-runtime-api`/`aws-smithy-types` use for
protocol (JSON/XML/eventstream) codecs, duplicating the `http 1.x` stack the
rest of the workspace (`axum`, `reqwest`) is on. Both pre-date this ticket and
are out of scope for it.

**FIPS-posture analysis (§14.4).** This change does not alter the FIPS
validation boundary and needs no exception. `is_fips_validated()`
(`crates/cloud-types/src/hsm.rs:172`) is a method on the `Hsm` trait; its doc
comment states the posture explicitly: "FIPS validation is a property of the
deployed HSM, not of this source" — CloudHSM-backed implementations return
`true`, `SoftHsm`/`MemoryHsm` return `false`. Spec §14.4 itself scopes the
boundary the same way: "HSM operations are validated end-to-end: when
CloudHSM is the backend, FIPS validation comes from the HSM itself; when
SoftHSM is used (dev only), the binary is explicitly marked non-FIPS." A
repo-wide search finds zero mentions of `rustls`, TLS, or `webpki` in the
architecture spec or any existing ADR in connection with §14.4 — the
boundary, as this project has scoped and built it so far, covers the
signing/key-protection path only, not network transport. Concretely,
`crates/cloud-aws` implements no `Hsm` impl at all yet (its own header
comment: "CloudHSM lands in a later ticket (`cloud-aws-cloudhsm`)"), so there
is presently nothing in this crate for a transport-TLS change to intersect
with on the FIPS axis. `aws-lc-rs` itself is not new to this workspace's
build either way: it was already present pre-change (`mtc-admin-api-client`'s
`rustls` feature → `reqwest`'s rustls-0.23 stack already selects the
`aws_lc_rs` crypto provider), confirmed by `git diff Cargo.lock` showing zero
added package names from this change. **This analysis is this agent's read,
not a ruling** — flagged in the task brief for the orchestrator and a crypto
reviewer to confirm before this ADR's Status moves to Accepted.

## Alternatives Considered

### Alternative A — Time-boxed `[advisories] ignore` exception in `deny.toml`

Rejected. This ticket's brief reserves this fallback for the orchestrator to
apply if the upgrade proves too disruptive to resolve cleanly — not a choice
for this agent to make preemptively, and moot here since the upgrade was not
disruptive (feature-flag-only, zero source changes, all tests green). It
would also have been substantively worse than what was available: it
suppresses the CI signal while leaving the actual vulnerable
name-constraint/CRL-parsing code live in the production S3/DynamoDB client,
rather than removing the exposure.

### Alternative B — Pin an older `aws-sdk-s3`/`aws-sdk-dynamodb` release

Rejected. There is no 1.x release that avoids `rustls-webpki 0.101.7` while
still enabling *some* rustls feature — every 1.x release exposes the same
legacy-vs-modern feature split; the vulnerability is in which feature this
workspace selected, not in the SDK version. Pinning older would forgo the
`aws-sdk-s3 1.141.0`/`aws-sdk-dynamodb 1.120.0` patch content for no
corresponding benefit.

### Alternative C — `rustls-aws-lc-fips` instead of `default-https-client`

Rejected for this ticket. `aws-smithy-http-client` also exposes a
`rustls-aws-lc-fips` feature, which swaps in `aws-lc-fips-sys` (a
FIPS-140-3-targeting build of aws-lc) purely for the transport TLS layer.
Per the FIPS-posture analysis above, §14.4's boundary as currently built does
not extend to transport TLS at all, so this would add a heavier build
dependency (a Go-toolchain requirement for `aws-lc-fips-sys`) for no
compliance benefit today. Revisit only if a future spec revision extends the
FIPS boundary to cover transport.

## Consequences

### Positive

- The three webpki RUSTSEC advisories (RUSTSEC-2026-0098, -0099, -0104) are
  cleared from both `cargo deny check advisories` and `cargo audit`, for both
  the production S3/DynamoDB client (`cloud-aws`) and the dev-only CRR
  simulator (`dev-replicator`).
- This also collapses the pre-existing, undocumented rustls/rustls-webpki
  duplicate described in Context: `git diff Cargo.lock` shows **zero new
  package names added** — only eight packages removed outright (`h2` 0.3.27,
  `hyper` 0.14.32, `hyper-rustls` 0.24.2, `rustls` 0.21.12, `rustls-webpki`
  0.101.7, `sct` 0.7.1, `socket2` 0.5.10, `tokio-rustls` 0.24.1) as Cargo
  unifies the aws-sdk client onto the same modern stack
  (`rustls 0.23.42`/`rustls-webpki 0.103.13`/`hyper 1.11.0`/
  `hyper-rustls 0.27.9`) `mtc-admin-api-client` already resolved. Net: fewer
  total dependencies, not more.
- `default-https-client`'s `rustls-aws-lc` feature also flips on rustls's
  `prefer-post-quantum` (confirmed via `cargo info aws-smithy-http-client`'s
  feature graph), so the S3/DynamoDB TLS 1.3 handshake now prefers a hybrid
  post-quantum key exchange (X25519MLKEM768) when the peer supports it,
  falling back to classical ECDHE otherwise — a defense-in-depth improvement
  against "harvest now, decrypt later," incidental to the RUSTSEC fix.
- Zero source-code changes were required in either crate. All existing tests
  pass unmodified: 46 `cloud-aws` unit tests, 29 `dev-replicator` unit tests,
  and all three LocalStack-backed conformance suites (S3 object store, S3
  object lock, DynamoDB `ReplicatedKv`) exercising the new client stack
  end-to-end over real (if plaintext, LocalStack-terminated) HTTP/2
  connections. `cargo clippy --workspace --all-targets --all-features -- -D
  warnings` and `cargo fmt --check` are both clean.

### Negative

- The AWS SDK's `rustls` feature name remains a trap for this workspace: a
  future contributor adding a new `aws-sdk-*` crate (e.g., when
  `cloud-aws-cloudhsm` lands) could reintroduce the same vulnerable stack by
  reasonably assuming `rustls` means "the modern rustls backend." Mitigated
  here by explanatory comments left in both `Cargo.toml` files, but there is
  no automated guard against recurrence elsewhere in the workspace — `cargo
  deny` cannot ban a feature *name* on a crate, only crate/version identity.
  Worth a follow-up (e.g., a grep-based CI check, or a `docs/license-policy.md`
  note) if the team wants one; not filed as a bead by this agent (out of this
  ticket's delegated scope — see task brief).
- The `deny.toml` `skip-tree` entries for `aws-sdk-s3`/`aws-sdk-dynamodb`
  remain in place (still load-bearing, per the empirical check in Decision)
  but now suppress a materially different, narrower duplicate set than their
  original comment described. Addressed in this PR by rewriting that comment;
  flagged here so a reviewer checks the new wording against the empirical
  claim rather than taking it on faith.
