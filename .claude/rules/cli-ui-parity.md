# cli-ui-parity

> Spec: §17 (Admin surface: CLI and UI); §17.1 (Why parity matters).

## Rule

Every admin operation must be available in both the CLI (`mtcctl`) and the UI.
Neither surface may gain an operation the other lacks.

## Rationale

§17: the admin surface has full CLI/UI parity — every operation available in
the UI is available in the CLI, and vice versa; both consume the same admin
API. Parity matters (§17.1) for operator ergonomics (scripts, automation,
runbooks), agent affordance (agents script the CLI deterministically; UI
automation is hard), E2E testing (tests drive the CLI to exercise the full
system, §19.13), and audit trail (CLI commands are logged with full
reproducibility). A UI-only or CLI-only operation breaks all four properties
at once.

## Compliant example

Adding a new admin operation follows §17.2 end-to-end in one change:

```text
1. api/admin.openapi.yaml     # add the operation to the OpenAPI spec
2. regenerate                 # server stubs, Rust client, TS client
3. server: implement handler
4. mtcctl: wire new subcommand (clap)
5. UI: wire handler/view
6. E2E test drives it via `mtcctl`
```

## Non-compliant example

```text
PR: "Add batch-abandon button to admin UI"
- UI gains an 'Abandon batch' action calling a new ad-hoc endpoint
- mtcctl has no `batch abandon` subcommand
- Operation is invisible to scripts, runbooks, and CLI-driven E2E tests
```

## Enforcement

- **Review**: PRs touching the admin surface are checked for both wire-ups;
  the OpenAPI-first flow (§17.2) makes both surfaces gain the operation
  simultaneously.
- **CI gate**: E2E tests are written against `mtcctl` (§19.13), so an
  operation missing from the CLI cannot be E2E-covered — a red flag at review.
