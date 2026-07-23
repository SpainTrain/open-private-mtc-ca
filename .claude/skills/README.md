# `.claude/skills/` — Per-Task Skills

Per-task entry points that orient an agent without requiring it to read the
entire repo (spec `docs/mtc-architecture-spec.md` §23.1). Each skill is a small
markdown file describing one recurring task: what it is, which files matter,
the standard pattern, and the mistakes to avoid.

Skills are documentation *for agents first*. Optimize for a cold-start reader
with a limited context window, not for completeness.

## Layout

- One flat markdown file per skill: `.claude/skills/<verb-noun>.md`
  (e.g. `add-metric.md`, `add-storage-method.md`).
- `_template.md` is the authoring template — copy it to start a new skill.
  It is linted like any other skill, so it always stays valid.
- This `README.md` is not a skill and is not linted.
- The lint also accepts the Claude Code native layout
  `.claude/skills/<name>/SKILL.md` (with optional YAML frontmatter), should a
  skill ever need bundled resources.

## Required structure

Every skill file must contain exactly these four `##` sections (§23.1):

| Section | Contents |
|---|---|
| `## Goal` | What task this skill helps with, in 1–3 sentences. |
| `## Files involved` | 3–5 bullet pointers to the relevant files. Each bullet **starts with a backticked repo-relative path** followed by ` — ` and why it matters. Paths must exist. |
| `## Pattern` | Example of the standard approach — the steps or a minimal code sketch. |
| `## Common pitfalls` | Bulleted mistakes an agent is likely to make, each with the avoidance. |

## Authoring a new skill

1. `cp .claude/skills/_template.md .claude/skills/<verb-noun>.md`
2. Replace every angle-bracket placeholder; delete the instructional HTML
   comments.
3. Keep it under roughly one screen. If the Pattern section is growing,
   the task probably needs two skills.
4. Run `make skill-lint` (defined in `mk/skills.mk`). Fix violations before
   committing; CI will run the same check (wired by the foundation-infra
   ci-pipeline ticket).

## Linting

`make skill-lint` runs `scripts/skill-lint.sh`, which fails loudly (nonzero
exit, one `skill-lint: FAIL <file>: <reason>` line per violation) when:

- a required section heading is missing,
- a `Files involved` bullet does not start with a backticked repo-relative
  path,
- a `Files involved` path does not exist in the repo,
- a skill has zero file pointers.

A pointer count outside 3–5 produces a warning (the ~5-file budget below is
the point of the section, but some tasks legitimately touch 2 or 6 files).

`make skill-lint-smoke` runs `scripts/skill-lint-smoke.sh`, a shell smoke
test (§19.13 spirit) asserting the linter passes on the checked-in skills and
fails on fixtures with a deleted section and a dangling path.

## Skill-driven scoping is a context-budget pattern (§23.8)

Skills target **~5 files** on purpose. When the context relevant to a task
exceeds the window, the harness strategy is *skill-driven scoping*: the skill
names the 3–5 files that matter, so an agent reads those instead of exploring
the tree. Combined with the other §23.8 patterns (`make agent-context`, Beads
tickets that specify exactly which files are in scope, scoped tests), this
keeps a task's working set inside a predictable token budget.

Consequences for authors:

- **The `Files involved` section is the contract.** If you can't express the
  task in ~5 file pointers, split the skill (or the task).
- **Pointers over prose.** Say *where* the canonical example lives rather
  than duplicating it; duplicated content rots and burns context twice.
- **The lint enforces freshness.** Dangling paths fail `make skill-lint`, so
  refactors that move files force the skill to be updated.

## Skills to seed (§23.1)

These land with their subject code in later tickets — do not create them
before the code exists:

- `add-admin-endpoint.md` — add a new admin operation end-to-end
  (OpenAPI → server → CLI → UI → test)
- `add-storage-method.md` — extend the Storage interface with a new operation
- `add-metric.md` — add a CloudWatch metric
- `add-chaos-test.md` — write a new chaos scenario
- `add-fixture.md` — capture and document a new test fixture
- `write-runbook.md` — runbook template + how to wire alerts
- `add-adapter.md` — add a new external CA adapter
- `update-spec-version.md` — track a new MTC draft revision
