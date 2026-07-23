# ADR-0001: Adopt architecture decision records

- **Status**: Accepted
- **Date**: 2026-07-23
- **Spec sections**: §23.2 (`document-decisions` rule), §23.5 (outer-loop
  tooling: "ADRs in `docs/adr/` for decisions worth preserving as standalone
  artifacts"), §23.6 ("ADR index — all ADRs in one indexable place; agents
  `grep` decisions before re-deciding")

## Context

This repository is built agent-first (spec §23): most code is written by
autonomous agents whose context windows do not span the project's history.
Without durable decision records, each agent re-derives — and risks
re-deciding — settled questions, eroding architectural invariants. The spec
mandates two complementary outer-loop artifacts (§23.5): a chronological
decision journal (`docs/journal.md`) and standalone ADRs, plus a grep-able
ADR index agents consult before re-deciding (§23.6). The journal is owned by
the `agent-journal-tooling` ticket; this ADR establishes the ADR process.

## Decision

We will record non-trivial decisions as MADR-style ADRs in `docs/adr/`:

- Each ADR contains Context, Decision, Alternatives Considered, and
  Consequences, plus Status, Date, and spec citations
  ([`_template.md`](_template.md)).
- ADRs are numbered sequentially (`0001`, `0002`, ...) and never renumbered
  or deleted; obsolete ADRs are marked `Deprecated` or `Superseded by
  ADR-NNNN`.
- [`README.md`](README.md) is the single grep-able index: one row per ADR
  with a one-line summary, maintained as rows are added by tooling.
- `make adr title="..."` (via `mk/adr.mk` and `scripts/adr-new.sh`)
  scaffolds the next-numbered ADR and its index row, so numbering and index
  freshness are mechanical, not remembered.
- The `.claude/rules/document-decisions.md` rule (§23.2) points agents here.
- Decisions already made in `docs/mtc-architecture-spec.md` (e.g. §6, Rust
  for services and TypeScript for CDK) are not backfilled; the spec remains
  authoritative for them, and new ADRs cite it rather than restating it.

## Alternatives Considered

### Journal-only decision capture

Rejected. `docs/journal.md` is chronological and append-only; finding "did
we already decide X?" means scanning history. §23.6 explicitly requires a
standalone, indexable ADR location.

### Full upstream MADR template with all optional fields

Rejected. The extended MADR fields (decision drivers, confirmation, pros/cons
matrices) add authoring weight without payoff for a solo, agent-driven repo.
The four core sections cover what future agents need.

### External ADR tooling (adr-tools, log4brains)

Rejected. Adds a dependency for what a ~90-line shell script does, and the
repo must work offline on a laptop (§1 non-goals, §18). A make target plus
POSIX-ish shell keeps the workflow self-contained.

## Consequences

### Positive

- Settled decisions are discoverable with one grep
  (`grep -i <keyword> docs/adr/README.md docs/adr/*.md`), so agents stop
  re-deciding.
- Scaffolding makes numbering and indexing mechanical; the "next number" and
  index row cannot drift by forgetfulness.
- Status lifecycle preserves the history of reversed decisions instead of
  losing it.

### Negative

- Index rows carry hand-written summaries and statuses that must be updated
  when an ADR's status changes (a CI freshness check is owned by
  foundation-infra).
- Sequential numbering can collide across concurrent branches; the loser of
  the race renumbers before merge.
