# Contributing

This repository is a reference blueprint (see [`README.md`](README.md) and spec
§1), developed by both humans and coding agents. Consistent, small, reviewable
changes keep it navigable for both. The design of record is
[`docs/mtc-architecture-spec.md`](docs/mtc-architecture-spec.md); work is tracked
as beads issues under `docs/planning/`.

## Pull requests (spec §25.3)

Every change lands as a small, self-contained PR:

- **One reviewable unit, one revertable unit.** A PR should do a single thing and
  be safe to revert on its own.
- **Target 200–500 net lines**, tests included. Outliers are fine when natural
  (vendored imports, generated code).
- **Vertical slices over horizontal slices.** A PR that carries one feature
  through storage + service + handler + tests beats one that adds a single column
  to every layer.
- **Tests ship in the same PR as the code.** Always. Acceptance criteria must be
  automatically validated — a test, a lint, or a documented manual demo step.
- **Each PR ships independently to `main`.** No long-lived branches; use feature
  flags to merge incomplete work dark.
- **Every PR description includes a rollback plan** — how to back the change out
  if it misbehaves.

## Repository conventions

- **Toolchain** is pinned by [`rust-toolchain.toml`](rust-toolchain.toml); do not
  rely on a system-wide Rust. `cargo fmt` and `cargo clippy` are expected to pass.
- **Directory layout** is fixed by the table in [`README.md`](README.md): Rust in
  `crates/`, CDK in `infra/`, the API contract in `api/`, docs in `docs/`, shell
  helpers in `scripts/`, and end-to-end tests in `tests/e2e/`. The `crates/` and
  `infra/` sides never share types — config crosses the boundary via SSM + env
  vars (spec §4).
- **No real cloud.** Nothing here targets a real AWS account. The CDK app is
  synth-only and the dev environment is LocalStack + SoftHSM2 (spec §1, §18).

## Adding `make` targets

`make` is the single entry point for dev workflows (spec §18.8), and its target
suite is assembled from fragments so parallel work never collides on one file:

- The **root `Makefile` is not edited to add targets.** It only sets shared
  variables and `include`s every `mk/*.mk` fragment.
- **Add targets by creating a new `mk/<name>.mk` fragment** (or extending an
  existing topic fragment you own). Group related targets in one fragment.
- **Every target carries a `## help text` comment** on its rule line; `make help`
  harvests these into the self-documenting catalog. A target without a `##`
  comment is invisible to `make help` — so always add one.
- Parameterized targets take arguments as `make <target> name=X days=N msg="..."`
  per the spec §18.8 conventions.

Example fragment (`mk/example.mk`):

```make
.PHONY: hello
hello: ## Print a friendly greeting (example fragment)
	@echo "hello from mtc-ca"
```

See [`docs/dev-environment.md`](docs/dev-environment.md) for the full target
catalog and the rationale behind this convention.

## Before you open a PR

- `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings`
  are clean.
- `cargo test` (and `cargo test --doc`) pass.
- New behavior has tests in the same PR.
- The PR description states the acceptance criteria met and a rollback plan.
