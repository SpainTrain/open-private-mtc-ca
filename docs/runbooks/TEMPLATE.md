# Runbook: <Failure Mode Name>

<!--
  How to use this template (spec §21.2):

  1. Copy this file to docs/runbooks/<failure-mode-slug>.md (kebab-case).
  2. Replace the title above with "Runbook: <human-readable failure mode>".
  3. Fill in the five sections below. All five headings are mandatory, must
     appear exactly once, and must stay in this order — `make lint-runbooks`
     enforces this.
  4. Add (or update) the runbook's row in README.md: status (stub/complete),
     the §20.1 alert/detection signals that page for it, and the §19.9 chaos
     scenarios that exercise it.
  5. Keep guidance comments like this one out of the final runbook; they are
     for authors, not responders.
-->

## Detection

<!--
  Which alerts fire and what the responder sees first.
  - Name the concrete §20.1 metrics/SLOs behind each alert
    (e.g. `lease_renewals_failed_total`, `crr_replication_lag_seconds`,
    "HSM signing success rate SLO 99.9% burn").
  - Include relevant §20.5 health endpoints (/health/primary, /health/audit).
  - State thresholds and severity (page vs. ticket) where known.
-->

## Initial assessment

<!--
  First 5–10 minutes: confirm the failure mode and rule out look-alikes.
  - Exact commands/queries to run (mtcctl, CloudWatch/Prometheus queries,
    health endpoint curls) and what healthy vs. unhealthy output looks like.
  - Blast radius questions: is issuance affected? read path? one region or
    both? Is this actually a different failure mode (link its runbook)?
  - Decision point: when to escalate and to whom.
-->

## Mitigation steps

<!--
  Stop the bleeding. Ordered, imperative steps a responder can follow verbatim.
  - Safest action first; call out any step that is destructive or
    irreversible with a "CAUTION:" prefix.
  - Include stand-down/freeze actions (e.g. freeze issuance) and the
    invariants that must hold (epoch invariant, no split-brain).
  - State the expected observable effect of each step.
-->

## Recovery procedure

<!--
  Return to steady state after mitigation.
  - Steps to restore full service and re-enable anything frozen.
  - Verification checklist: which §20.1 metrics/SLOs and §20.5 health
    endpoints must look healthy, and for how long, before declaring recovery.
  - Data integrity checks (e.g. self-auditor run passes, checkpoint
    consistency) where applicable.
-->

## Postmortem template

<!--
  Every invocation of this runbook that involved user impact or a page gets a
  postmortem. Use the shared template — do not restate it here.
-->

Use the shared [postmortem template](POSTMORTEM.md). File the completed
postmortem and link it from the incident record; feed action items back into
this runbook if the response revealed gaps.
