# Rule: document-decisions

Non-trivial decisions go in `docs/adr/` (spec §23.2, §23.5).

## Rationale

Agents' context windows do not span project history. A decision that lives
only in a PR description or chat transcript will be re-litigated — or
silently reversed — by a later agent. ADRs in [`docs/adr/`](../../docs/adr/)
are the standalone, grep-able record (§23.5–23.6): check the index at
[`docs/adr/README.md`](../../docs/adr/README.md) before deciding, and record
new non-trivial decisions with `make adr title="..."` (template:
[`docs/adr/_template.md`](../../docs/adr/_template.md)). Chronological
narrative belongs in `docs/journal.md`; durable decisions belong in ADRs.

## Compliant example

Choosing a lock-free queue over a mutex on the write path: grep
`docs/adr/README.md` for prior art, then `make adr title="Lock-free queue
for write-path batching"`, fill in context/decision/alternatives/
consequences with spec citations, mark it Accepted, and update the index
row's summary — all in the same PR as the code.

## Non-compliant example

Explaining the same choice only in the PR description ("went with crossbeam
because benchmarks"), leaving no ADR — the next agent touching the write
path has no discoverable record and re-decides.

## Enforcement

Review: PRs containing non-trivial design choices must reference an ADR (new
or existing). `make adr` keeps numbering and the index mechanical; a CI
freshness check on the index is owned by foundation-infra.
