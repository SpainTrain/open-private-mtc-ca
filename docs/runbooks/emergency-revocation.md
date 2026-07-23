# Runbook: Emergency revocation

> **Status: stub.** Skeleton per [TEMPLATE.md](TEMPLATE.md); full content is
> tracked in a separate ticket. Section structure is enforced by
> `make lint-runbooks`.

## Detection

Operator- or report-initiated (not alert-driven); revocation-rate anomaly on
the log inventory dashboard (§20.2).

TODO: thresholds, severities, and exact alert names once alerting lands.

## Initial assessment

TODO: confirmation commands/queries, blast-radius questions, escalation
decision points.

## Mitigation steps

TODO: ordered stop-the-bleeding steps, with CAUTION markers on destructive
actions.

## Recovery procedure

TODO: steps back to steady state plus the verification checklist (§20.1
metrics/SLOs, §20.5 health endpoints) before declaring recovery.

## Postmortem template

Use the shared [postmortem template](POSTMORTEM.md); feed action items back
into this runbook.
