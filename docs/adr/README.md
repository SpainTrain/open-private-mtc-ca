# Architecture Decision Records (ADRs)

Non-trivial decisions in this repository are preserved as standalone,
MADR-style Architecture Decision Records, per spec §23.5 (outer-loop tooling)
and §23.6 ("ADR index" — all ADRs in one indexable place; agents `grep`
decisions before re-deciding). The chronological narrative lives in
`docs/journal.md`; ADRs are the durable, standalone artifacts.

This directory is cross-linked from the `document-decisions` rule
(`.claude/rules/document-decisions.md`, spec §23.2): non-trivial decisions go
in `docs/adr/`.

## Before you decide anything

Grep this index first so you do not re-litigate a settled decision:

```bash
grep -i "<keyword>" docs/adr/README.md docs/adr/*.md
```

If an existing ADR covers your question, follow it or write a superseding ADR
— do not silently diverge.

## Creating an ADR

```bash
make adr title="Short imperative decision title"
```

This scaffolds the next-numbered ADR from [`_template.md`](_template.md) and
adds a row to the index below. Then:

1. Fill in Context, Decision, Alternatives Considered, and Consequences.
2. Cite the relevant spec sections of `docs/mtc-architecture-spec.md`.
3. Set **Status** (see lifecycle below) in both the ADR and its index row.
4. Replace the placeholder summary in the index row with a one-line summary.

## Conventions

- **Numbering**: sequential four-digit numbers (`0001`, `0002`, ...), assigned
  by `make adr`; never reuse or renumber.
- **Filenames**: `NNNN-kebab-case-title.md`.
- **Status lifecycle**: `Proposed` → `Accepted`; later `Deprecated` or
  `Superseded by ADR-NNNN`. Never delete an ADR; supersede it.
- **One decision per ADR.** Split unrelated decisions.
- **Index freshness**: every ADR has exactly one row below, kept in sync with
  the file's title and status. (`make adr` adds the row; a CI freshness check
  is owned by foundation-infra.)
- Decisions already recorded in the architecture spec are **not** backfilled
  as ADRs; the spec remains authoritative for them. New ADRs cite the spec
  instead of restating it.

## Index

<!-- Generated rows vary in width; pipe alignment (MD060) is not enforced. -->
<!-- markdownlint-disable MD060 -->
<!-- adr-index-begin -->
| ADR | Title | Status | Summary |
|-----|-------|--------|---------|
| [ADR-0001](0001-adopt-architecture-decision-records.md) | Adopt architecture decision records | Accepted | Preserve non-trivial decisions as MADR-style ADRs in docs/adr/ with a grep-able index and `make adr` scaffolding (spec §23.5–23.6). |
| [ADR-0002](0002-use-step-functions-for-the-pruning-workflow-oq-7.md) | Use Step Functions for the pruning workflow (OQ-7) | Accepted | Orchestrate pruning (§15.2) with a Step Functions Standard state machine over thin Lambda steps, not a Lambda chain; spike proves it runs on the pinned LocalStack community image (§1 zero-cost). |
<!-- adr-index-end -->
<!-- markdownlint-enable MD060 -->
