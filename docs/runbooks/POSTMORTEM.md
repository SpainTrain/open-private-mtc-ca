# Postmortem: <Incident Title>

<!--
  Shared postmortem template (spec §21.2, fifth runbook section).
  Copy for each incident; keep it blameless and specific. The four sections
  Timeline, Impact, Root cause, and Action items are mandatory and are
  enforced by `make lint-runbooks` on this template.
-->

- **Date of incident:** YYYY-MM-DD
- **Runbook used:** [<runbook name>](<runbook-file>.md)
- **Authors:** <names>
- **Status:** draft | reviewed | closed

## Summary

<!-- 2–4 sentences: what happened, how long it lasted, what the user-visible
     effect was, and how it was resolved. Written for readers with no prior
     context. -->

## Timeline

<!-- UTC, one line per event. Include: first alert fired, responder engaged,
     assessment findings, each mitigation step taken, recovery declared.
     Pull exact timestamps from alerts and the structured logs
     (correlation_id, epoch, batch_id fields — §20.1). -->

| Time (UTC) | Event |
|---|---|
| YYYY-MM-DDTHH:MMZ | <first alert fired> |
| YYYY-MM-DDTHH:MMZ | <responder acknowledged> |
| YYYY-MM-DDTHH:MMZ | <mitigation / recovery events...> |

## Impact

<!-- Quantify: duration, affected SLOs (§20.1) and by how much, number of
     failed/delayed issuances, read-path availability, affected relying
     parties or adapters. State explicitly if log integrity was ever at
     risk and how that was ruled out (self-auditor evidence). -->

## Root cause

<!-- The full causal chain, not just the trigger: what failed, why it
     failed, why detection/mitigation did or didn't work as designed.
     Reference spec invariants where relevant (epoch invariant, CRR
     catch-up gate, etc.). Blameless: name systems and processes, not
     people. -->

## Action items

<!-- Each item: owner, tracking ticket, priority, due date. Include
     follow-ups for detection gaps (new alerts), runbook corrections, and
     chaos scenarios (§19.9) that should cover this failure mode. -->

| Action item | Owner | Ticket | Priority | Due |
|---|---|---|---|---|
| <fix / hardening / runbook update> | <owner> | <ticket> | P0–P3 | YYYY-MM-DD |
