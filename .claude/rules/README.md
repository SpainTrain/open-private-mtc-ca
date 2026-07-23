# Repo Rules

Repo-specific rules agents must always follow, seeded from
`docs/mtc-architecture-spec.md` §23.2. Each rule file has the same structure —
`## Rule`, `## Rationale`, `## Compliant example`, `## Non-compliant example`,
`## Enforcement` — validated by `make rules-lint`.

## Code discipline (§22)

| Rule | Gist | Spec |
|---|---|---|
| [always-test-with-pr](always-test-with-pr.md) | Every PR includes tests; never code without tests | §22.13 |
| [no-systemtime-now-in-prod](no-systemtime-now-in-prod.md) | `SystemTime::now()` forbidden outside tests; use injected `Clock` | §22.11 |
| [no-unwrap-in-prod](no-unwrap-in-prod.md) | `unwrap()`/`expect()` forbidden outside tests; use `?` and error types | §22.6 |
| [no-unsafe](no-unsafe.md) | `unsafe` forbidden except documented FFI boundaries (PKCS#11) | §22.12 |
| [use-newtypes](use-newtypes.md) | Domain identifiers are newtypes, never type aliases | §22.1 |
| [thiserror-for-libs-eyre-for-bins](thiserror-for-libs-eyre-for-bins.md) | Error type discipline: `thiserror` in libs, `eyre` + `color-eyre` in bins | §22.6 |
| [no-sdk-types-in-domain](no-sdk-types-in-domain.md) | Vendor SDK types never appear in trait signatures or domain code | §22.8 |
| [prefer-generics-on-hot-paths](prefer-generics-on-hot-paths.md) | `<T: Trait>` on hot paths; `Arc<dyn Trait>` only at seams | §22.7 |

## Workflow

| Rule | Gist | Spec |
|---|---|---|
| [fips-boundary-preserved](fips-boundary-preserved.md) | Production builds pass the FIPS CI check; posture changes need an ADR | §14.4 |
| [cli-ui-parity](cli-ui-parity.md) | Every admin operation available in both CLI and UI | §17 |
| [document-decisions](document-decisions.md) | Non-trivial decisions go in `docs/adr/` | §23.5 |
| [run-precheck-first](run-precheck-first.md) | Start every task with `make agent-precheck` | §23.4 |
| [update-codemap-on-structure-change](update-codemap-on-structure-change.md) | `make codemap` after creating/moving crates | §23.6 |
| [single-pr-acceptance](single-pr-acceptance.md) | One PR per task unless explicitly large; otherwise decompose | §25.2 |
| [kani-for-critical-paths](kani-for-critical-paths.md) | Lease/epoch and write-path changes include/update Kani harnesses | §19.12 |
| [spec-pin-and-track](spec-pin-and-track.md) | Pin to a specific MTC draft revision; track WG main; ticket divergences | §28 |

Command names referenced by these rules (`make agent-precheck`, `make codemap`,
`make verify-task`, `make journal`) are the spec-defined targets (§23.4,
§23.5, §23.6); rules stay valid even where a target has not landed yet.
