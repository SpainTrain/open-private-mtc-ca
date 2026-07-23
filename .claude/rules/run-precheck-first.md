# run-precheck-first

> Spec: §23.4 (Inner-loop tooling: `make agent-precheck`).

## Rule

Start every task with `make agent-precheck`. Do not begin editing code before
it passes (or before its findings are understood).

## Rationale

§23.4: `make agent-precheck` verifies the environment, reads recent decisions,
lints the current state, and runs fast tests — before work starts. Skipping it
means building on an unknown baseline: pre-existing lint or test failures get
entangled with the new change, and recent decisions (journal, ADRs) that
constrain the task go unread. For agents this is the difference between a
scoped, verifiable diff and a PR that mixes drive-by fixes with its actual
goal.

## Compliant example

```console
$ make agent-precheck        # first command of the task
...environment OK, recent journal entries shown, lint clean, fast tests green
$ git switch -c storage-3-allocate-indices
$ $EDITOR crates/storage/src/lib.rs
```

## Non-compliant example

```console
$ $EDITOR crates/storage/src/lib.rs   # editing immediately
# ...later: cargo test fails on main's pre-existing breakage, and the change
# contradicts a decision recorded in yesterday's journal entry
```

## Enforcement

- **Process/review**: task write-ups and `WORKING_SET.md` (§23.4) reflect the
  precheck as step one; PRs that stumble over pre-existing baseline breakage
  indicate it was skipped.
- **Tooling**: the precheck itself is the gate — it fails loudly when the
  environment or baseline is not fit to build on.
