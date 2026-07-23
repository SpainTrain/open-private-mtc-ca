# <verb-noun>: <one-line summary of the task>

<!-- Skill template (spec §23.1). To author a new skill:
       cp .claude/skills/_template.md .claude/skills/<verb-noun>.md
     Replace every <angle-bracket> placeholder, delete these HTML comments,
     and keep all four `##` sections — `make skill-lint` enforces them.
     This template is itself linted, so its example paths are real files. -->

## Goal

<What task this skill helps with, in 1–3 sentences. Name the outcome the
agent should produce, not the mechanism.>

## Files involved

<!-- 3–5 bullets. Each bullet MUST start with a backticked repo-relative
     path that exists, then " — " and why the file matters. skill-lint
     fails on dangling paths, so refactors force this list to stay fresh. -->

- `.claude/skills/README.md` — authoring guide, required structure, lint contract
- `docs/mtc-architecture-spec.md` — §23.1 defines skills; §23.8 the ~5-file context budget
- `scripts/skill-lint.sh` — the validator that keeps this file honest
- `mk/skills.mk` — make targets: `skill-lint`, `skill-lint-smoke`

## Pattern

<Example of the standard approach. Prefer a short numbered sequence or a
minimal code sketch; point at a canonical in-repo example instead of
duplicating it.>

1. <first step — e.g. "copy the nearest existing example of X">
2. <second step — the change itself>
3. <verification step — the exact command that proves it works>

## Common pitfalls

- <pitfall an agent is likely to hit> — <how to avoid it>
- <pitfall> — <avoidance>
