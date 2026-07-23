# Developer Environment

`make` is the single entry point for every developer workflow in this repo
(spec §18.8). Run `make help` (the default target) for the self-documenting
catalog of what is available.

```bash
make help
```

Everything here runs on a laptop at zero cloud cost (spec §1). The workflows the
targets below drive — the 60-second demo, fixtures, time travel, REPLs, hot
reload — are implemented incrementally by later tickets; until then each target
is a stub that prints a pointer to the beads work that will implement it.

## How the Makefile is organized

The root `Makefile` is deliberately generic. It does three things:

1. sets `make help` as the default goal,
2. `include`s every fragment in `mk/*.mk`, and
3. provides the shared `not_implemented` helper.

**It declares no workflow targets itself, and you should never add targets to
it.** All targets live in topic fragments under `mk/`. This is the same
collision-avoidance idea as the `crates/*` glob in `Cargo.toml`: parallel work
(humans or agents) adds files instead of editing one shared file.

### Adding a target

1. Create a new `mk/<topic>.mk` fragment (or extend a topic fragment you own).
   Copy `mk/example.mk` as a starting point.
2. Declare the target `.PHONY` and give its rule line a `## help text` comment.
   `make help` harvests these comments across the root Makefile and every
   fragment via `$(MAKEFILE_LIST)`. **A target with no `##` comment is invisible
   to `make help`** — always add one.
3. Either implement the recipe — delegating to `scripts/<name>.sh` or to `cargo`
   — or leave it as a stub with `$(call not_implemented,<beads-slug-or-epic>)`.

Example fragment (`mk/example.mk`):

```make
.PHONY: hello
hello: ## Print a friendly greeting (example fragment)
	@echo "hello from mtc-ca"
```

When a later ticket implements a target that ships here as a stub, it edits that
target's fragment — not the root Makefile — so the change stays local and
reviewable.

### Argument conventions

Parameterized targets take `name=value` arguments (spec §18.8):

```bash
make fixture-load name=demo
make fixture-save name=scratch
make time-advance days=400
make journal msg="Chose UpdateItem over transaction for the counter"
```

Missing required arguments are validated by each target's own implementation
once it lands (a stub simply reports "not implemented").

## Target catalog

Every target below is declared today; the **Owner** column names the beads
ticket or epic that replaces the stub with a real implementation. Targets are
grouped into the `mk/` fragment that hosts them.

| Target | Fragment | Purpose | Owner |
|---|---|---|---|
| `help` | `Makefile` | Self-documenting target catalog | dev-make-skeleton (this) |
| `hello` | `mk/example.mk` | Example target demonstrating the convention | dev-make-skeleton (this) |
| `demo` | `mk/demo.mk` | Single-region 60-second demo | dev-demo-single-region |
| `demo-down` | `mk/demo.mk` | Tear down the single-region demo | dev-demo-single-region |
| `demo-multiregion` | `mk/demo.mk` | Three-region simulated environment | dev-multiregion-harness |
| `demo-multiregion-down` | `mk/demo.mk` | Tear down the three-region environment | dev-multiregion-harness |
| `dev` | `mk/demo.mk` | Hot-reload the CA service (cargo-watch) | dev-hot-reload |
| `partition-region` | `mk/demo.mk` | Simulate a network partition (`region=X`) | dev-partition-failover-scenarios |
| `time-advance` | `mk/demo.mk` | Advance the simulated clock (`days=N`) | dev-time-advance |
| `test` | `mk/test.mk` | Run all tests | testing epic (§19) |
| `test-unit` | `mk/test.mk` | Unit tests only | testing epic (§19.1) |
| `test-prop` | `mk/test.mk` | Property-based tests, extended runs | testing epic (§19.2) |
| `test-conformance` | `mk/test.mk` | Spec conformance suite | testing epic (§19.4) |
| `test-chaos` | `mk/test.mk` | Chaos-engineering scenarios | testing epic (§19.9) |
| `test-soak` | `mk/test.mk` | Long-running soak test | testing epic (§19.7) |
| `test-e2e` | `mk/test.mk` | End-to-end tests via the CLI | testing epic (§19.13) |
| `repl` | `mk/dev-tools.mk` | Interactive Rust REPL (evcxr) | dev-repl-evcxr |
| `fixture-load` | `mk/dev-tools.mk` | Load a named fixture (`name=X`) | dev-fixture-targets |
| `fixture-save` | `mk/dev-tools.mk` | Snapshot state as a fixture (`name=X`) | dev-fixture-targets |
| `api-gen` | `mk/api.mk` | Regenerate API code from OpenAPI | openapi-codegen-pipeline |
| `codemap` | `mk/agent.mk` | Generate the repo code map | agent-harnessing epic (§23.6) |
| `agent-context` | `mk/agent.mk` | Generate an agent context summary | agent-harnessing epic (§23) |
| `agent-precheck` | `mk/agent.mk` | Pre-task verification | agent-inner-loop-targets (implemented) |
| `verify-task` | `mk/agent.mk` | Post-task verification | agent-inner-loop-targets (implemented) |
| `watch` | `mk/agent.mk` | Lint+test on every save (bacon / cargo-watch) | agent-inner-loop-targets (implemented) |
| `working-set` | `mk/agent.mk` | Start `./WORKING_SET.md` from the template | agent-inner-loop-targets (implemented) |
| `journal` | `mk/agent.mk` | Append to the decision journal (`msg="..."`) | agent-harnessing epic (§23.7) |
| `fmt` | `mk/quality.mk` | Format all code | fnd-rust-lint-config |
| `lint` | `mk/quality.mk` | Run all linters | fnd-rust-lint-config |
| `audit` | `mk/quality.mk` | Run the self-auditor manually | dev-audit-demo-wiring |
| `bench` | `mk/quality.mk` | Performance benchmarks | testing epic (§19.11) |
| `doctor` | `mk/doctor.mk` | Diagnose the dev environment | dev-doctor |

The E2E smoke test `tests/e2e/make-targets.sh` keeps this skeleton honest: it
asserts `make help` lists every spec §18.8 target and that each remaining stub
exits non-zero.

## Inner-loop tooling (spec §23.4)

Every agent (or human) task runs inside the same fast feedback loop, entirely
offline on a laptop:

1. **`make agent-precheck`** — run first, before editing anything
   (`.claude/rules/run-precheck-first.md`). Verifies required tools (Rust
   toolchain, docker, beads CLI), prints the most recent `docs/journal.md`
   entries so recent decisions are read before code is written, then lints the
   workspace (`cargo fmt --check`, `clippy -D warnings`) and runs the fast
   unit tests. Any failure exits non-zero: fix the baseline (or understand it)
   before starting. Targets ~60s on a warm laptop.
2. **`make watch`** — keep running while you work. Runs lint+test on every
   save via `bacon` (preferred; repo config in `bacon.toml`) or `cargo-watch`.
   If neither is installed it prints the `cargo install` hints and exits;
   `make doctor` owns full diagnostics.
3. **`make verify-task`** — the pre-done acceptance gate. Runs workspace lint,
   the fast test suite (`cargo test --workspace`), and the doc/skill/rules
   lints (`rules-lint`, `skill-lint`, `lint-runbooks`,
   `lint-security-checklist`). A task is not done until it passes.

### `WORKING_SET.md` convention

While a task is in flight, its scratch state lives in `WORKING_SET.md` at the
repo root: the files touched (and why), decisions made (with rejected
alternatives), and open questions. Start one from the checked-in template with
`make working-set` (never overwrites an existing copy), keep it current as you
work, and use it at close-out as the checklist for the journal entry
(`make journal msg="..."`) and for filing beads tickets for open questions.

Working copies are task-scoped and **never committed** — `/WORKING_SET.md` is
gitignored. The template itself is versioned at `docs/templates/WORKING_SET.md`.

`make agent-inner-loop-test` smoke-tests all of the above (script logic,
failure modes, and both gates passing on the current tree; shellcheck when
installed).
