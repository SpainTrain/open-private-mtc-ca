# document-decisions

> Spec: §23.5 (Outer-loop tooling: ADRs and decision journal).

## Rule

Non-trivial decisions go in `docs/adr/`. Smaller, task-scoped decisions are
recorded in the decision journal (`docs/journal.md`, via `make journal
msg="..."`).

## Rationale

§23.5: ADRs in `docs/adr/` preserve decisions worth keeping as standalone
artifacts; the decision journal gives future agents the context of what was
decided and why. Agents (and humans) working across tasks cannot re-derive
intent from code alone — an undocumented decision gets silently re-litigated,
and the ADR index exists precisely so agents can `grep` decisions before
re-deciding (§23.6). Scaffold with `make adr title="..."` (template:
`docs/adr/_template.md`) and check the index `docs/adr/README.md` first. This is the outer loop that keeps a long-running,
multi-session project coherent.

## Compliant example

```text
PR: "Use DDB ConditionExpression for epoch-checked counter updates"
- docs/adr/0009-epoch-conditional-writes.md
  Context / Decision / Alternatives considered (optimistic locking — rejected:
  violates single-writer guarantees) / Consequences
- journal entry appended via: make journal msg="STORAGE-3: chose
  ConditionExpression over optimistic locking; see ADR-0009"
```

## Non-compliant example

```text
PR: "Rework lease renewal timing"
- Changes the renewal interval and jitter strategy — a correctness-relevant
  protocol decision — with no ADR and no journal entry; rationale exists only
  in the PR comment thread.
```

## Enforcement

- **Review**: PRs embodying a non-trivial decision (protocol, dependency,
  boundary, format) without an ADR or journal entry are sent back.
- **Tooling**: closing a Beads ticket auto-appends a journal entry (§23.5);
  `make agent-precheck` reads recent decisions so undocumented ones are
  invisible to the next task (§23.4).
