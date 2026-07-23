# Agent Delegation Policy

How the orchestrating session dispatches beads tickets to subagents. Goal:
every bead is worked by the **cheapest persona that can actually handle it**,
with a hard escalation path instead of silent guessing, and an adversarial QA
gate before merge. Personas live in `.claude/agents/`.

## Personas

| Persona | Model | Takes |
|---|---|---|
| `impl-mechanical` | Haiku | Docs from a written source, fixtures, config, renames, stub-to-script wiring, transcription the spec fully specifies. No design judgment. |
| `impl-standard` | Sonnet | Most feature/crate/adapter/endpoint/test work with a clear ticket contract. Judgment within bead scope. |
| `impl-hard` | Opus | Tree/proof math, wire-format parsing, crypto/HSM, lease/epoch, Kani/Loom design, security-critical parsing, cross-crate design. |
| `qa-reviewer` | Sonnet | Adversarial pre-merge review. Read-and-run only (no write tools). Verdict: PASS / FAIL / PASS-WITH-FINDINGS. |
| `crypto-reviewer` | Opus | Specialist crypto audit on crypto-touching beads only. Read-and-run only. Checklist of known misimplementation classes (domain separation, malleability, nonce hygiene, timing, proof verification, key handling, KATs). |

## Triage rubric (orchestrator, at dispatch)

Start from the ticket's size, epic, and AC; pick the **lowest** tier that fits;
record the choice as a bead label `tier:mech` / `tier:std` / `tier:hard` when
marking it in_progress.

- **mech** — AC are checkable without running a design in your head: docs with a
  named source section, fixtures, config, renames, `mk/*.mk` wiring to an
  existing script. Runbook *stubs* yes; runbook *content* no (std).
- **std** — new code against an existing trait/contract, admin endpoints + CLI +
  UI wiring, CDK constructs, test suites for specified behavior, backend impls
  (`cloud-aws`, `cloud-localstack`, …), tooling scripts with branching logic.
- **hard** — anything in `core-mtc-library`'s tree/proof/serialization core,
  `multi-region`'s lease/epoch protocol, HSM/key handling, fuzz/Kani/Loom
  harness *design* (running existing harnesses is std), EntryIntake seam design,
  parsers exposed to untrusted input.

Heuristics: spec §25.5 critical-path anchors default to hard unless clearly
mechanical; `size-M` + P0 leans up a tier; post-v1 backlog leans down a tier.
When honestly torn between two tiers, pick the lower one — the escalation
ladder makes under-assignment cheap (one BLOCKED report), while
over-assignment silently burns the expensive model on trivial work.

## Escalation ladder

`impl-mechanical → impl-standard → impl-hard → orchestrator/user`

- Implementers report `BLOCKED: <reason>` the moment work exceeds their tier —
  guessing is prohibited. A BLOCKED report is a successful outcome.
- The orchestrator re-dispatches to the next tier, passing the BLOCKED
  report so paid-for context is not re-derived.
- `impl-hard` escalates genuine design forks (spec-ambiguous, two defensible
  architectures) to the orchestrator, who decides or asks the user; the
  decision lands as an ADR.
- Two BLOCKEDs on one bead = the ticket is mis-scoped; orchestrator re-reads
  it and either splits it (new beads) or fixes the contract before re-dispatch.

## QA gate and closing protocol

1. Implementer finishes on its branch/worktree, reports, **leaves the bead
   claimed** (implementers never run `bd`).
2. Orchestrator dispatches `qa-reviewer` on the branch with the implementer's
   report. QA re-runs every gate itself and verdicts.
3. **Crypto-touching beads additionally get `crypto-reviewer`** (may run in
   parallel with QA; both must PASS before merge). Triggers — dispatch it when
   the bead or its diff touches any of: hashing/tree/proof code in
   `crates/mtc`; signing, checkpoint, or signature verification; JWS/ACME
   protocol crypto; HSM crates or the `Hsm` trait's contract; key
   generation/import/ceremony; revocation signing; RNG use in a security
   context; serialization of any signed structure. Label the bead
   `crypto-review` when marking it in_progress. When unsure whether it
   qualifies, it qualifies.
4. On **PASS** / **PASS-WITH-FINDINGS** (from every required reviewer):
   orchestrator merges, re-verifies on integrated main, closes the bead
   (`bd close`), files findings as new beads.
5. On **FAIL** (from either reviewer): orchestrator sends the verdict back to
   the implementer (same agent, same context) for rework, or re-dispatches up
   a tier. Rework goes back through the failing reviewer.
6. Only the orchestrator ever merges, closes beads, or pushes.

QA hard rule: the review does not exist until the verdict message is sent.

## Cost discipline

- Persona files carry the repo conventions — dispatch prompts stay short
  (ticket id/slug/epic, special notes, commit trailer). Don't re-paste
  conventions per dispatch.
- Batch mech tickets: one impl-mechanical agent may take 2–4 related mech
  beads in a single dispatch when they touch disjoint files.
- QA batches too: one qa-reviewer may review several small same-epic branches
  in one dispatch; verdicts stay per-bead.
- The orchestrator does not re-do QA's gates on the branch; it re-runs the
  gate suite once, on integrated main, after merge.
