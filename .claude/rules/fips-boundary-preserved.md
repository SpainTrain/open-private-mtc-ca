# fips-boundary-preserved

> Spec: §14.4 (FIPS validation boundary); §23.2.

## Rule

Production builds must pass the FIPS validation CI check. Any dependency change
that alters the FIPS posture requires an explicit ADR before it merges.

## Rationale

§14.4: FIPS validation is a property of a specific build artifact, not of the
source code. Exact versions of FIPS-validated crypto libraries (`qux-pqc`,
RustCrypto FIPS modules where applicable) are pinned, with Cargo's lockfile
guaranteeing reproducibility. A routine-looking dependency upgrade can silently
remove FIPS validation; the CI gate exists so that such a build fails instead
of shipping. A non-FIPS-validated build can never be deployed to production.
Because the change that breaks the boundary looks like any other version bump,
the decision must be surfaced deliberately — as an ADR — rather than buried in
a lockfile diff.

## Compliant example

```text
PR: "Upgrade qux-pqc 0.4.2 -> 0.5.0"
- docs/adr/0017-qux-pqc-0.5-fips-posture.md  (FIPS impact analyzed and accepted)
- Cargo.toml / Cargo.lock pin the exact new version
- CI: fips-validation check green; `is_fips_validated()` still true on CloudHSM builds
```

## Non-compliant example

```text
PR: "chore: cargo update"
- Cargo.lock bumps qux-pqc and swaps a RustCrypto module for a non-validated fork
- No ADR, no mention of FIPS in the PR description
```

## Enforcement

- **CI gate**: every release artifact runs a FIPS-validation check before
  publication; an environment-tagged check blocks non-FIPS builds from
  production (§14.4). Compliance reports include `is_fips_validated()` (§20.3).
- **Review**: crypto-dependency changes without an accompanying ADR in
  `docs/adr/` are rejected.
