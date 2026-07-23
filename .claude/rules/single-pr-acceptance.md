# single-pr-acceptance

> Spec: §25.2 (PR-based ticket sizing); §25.3 (PR best practices).

## Rule

One PR per task unless the task is explicitly sized large; otherwise decompose
the task before starting. A PR is a single reviewable unit and a single
revertable unit.

## Rationale

§25.2 sizes tickets in PRs, not hours: S is one PR (~50–200 lines net including
tests); M is 2–3 PRs, each independently shippable; L means "needs
decomposition — break down before starting". §25.3 targets 200–500 lines net
per PR, vertical slices over horizontal slices, and every PR shipping
independently to main (no long-lived branches; feature flags instead). Sprawling
multi-concern PRs defeat review (too much to hold), defeat revert (the unit is
too coarse), and defeat the agent workflow, where acceptance criteria are
validated per PR.

## Compliant example

```text
Task: "Implement counter UpdateItem with epoch check" (sized S)
-> One PR: trait method + implementation + unit/property/integration tests,
   ~250 lines net, ships to main, rollback plan in the description.

Task sized M -> decomposed into 2–3 independently shippable PRs
   (e.g., PR1: trait + fake impl + tests; PR2: DDB impl + LocalStack tests).
```

## Non-compliant example

```text
PR: "Storage layer + lease renewer + admin endpoints" (1,800 lines)
- Three tasks' worth of work in one diff, none independently revertable,
  acceptance criteria for each task impossible to validate in isolation.
```

## Enforcement

- **Review**: PRs bundling multiple tasks, or L-sized work started without
  decomposition, are rejected; outliers (vendored imports, generated code) are
  acceptable when natural (§25.3).
- **Process**: Beads tickets carry the PR sizing; `make verify-task` (§23.4)
  runs acceptance-criteria checks per task before it is declared done.
