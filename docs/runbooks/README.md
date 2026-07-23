# Runbooks

Operational runbooks for the MTC CA, per spec §21.2. Every known failure mode
gets a runbook with five mandatory sections: **Detection**, **Initial
assessment**, **Mitigation steps**, **Recovery procedure**, **Postmortem
template**.

- Author new runbooks from [TEMPLATE.md](TEMPLATE.md).
- Postmortems use the shared [POSTMORTEM.md](POSTMORTEM.md) template.
- Structure is enforced by `make lint-runbooks`
  ([scripts/lint-runbooks.sh](../../scripts/lint-runbooks.sh)): the five
  sections above, in order, exactly once each, plus index completeness for
  the eight required runbooks below.

## Index

Status is `stub` (skeleton only, content tracked in a separate ticket) or
`complete`. "Alerts / detection signals" names the §20.1 metrics, SLOs, and
§20.5 health endpoints that alerts derive from; "Chaos scenarios" lists the
§19.9 scenarios that exercise the failure mode.

| Runbook | Status | Alerts / detection signals (§20.1) | Chaos scenarios (§19.9) |
|---|---|---|---|
| [Primary failure](primary-failure.md) | stub | `lease_renewals_failed_total` spike; lease renewal SLO (99.99%) burn; `/health/primary` flapping; unexpected `epoch_advances_total` increment | `chaos-primary-loss`, `chaos-split-brain`, `chaos-old-primary-recovery`, `chaos-clock-skew` |
| [HSM unavailability](hsm-unavailability.md) | stub | `hsm_signing_latency_seconds` p99 breach; HSM signing SLO (99.9%) burn; `batches_abandoned_total` rising | `chaos-hsm-down` |
| [CRR stall](crr-stall.md) | stub | `crr_replication_lag_seconds` over threshold; `ddb_replication_lag_seconds` over threshold | `chaos-crr-stall`, `chaos-ddb-lag` |
| [Self-auditor anomaly](self-auditor-anomaly.md) | stub | self-auditor anomaly page (pages immediately and freezes issuance, §20.2); `/health/audit` failing or stale | none yet (§19.9 gap — add scenario when chaos suite lands) |
| [Emergency revocation](emergency-revocation.md) | stub | operator- or report-initiated (not alert-driven); revocation-rate anomaly on log inventory dashboard (§20.2) | none (procedure-driven; exercised by game days) |
| [Pruning failure](pruning-failure.md) | stub | pruning watermark stalled on log inventory dashboard (§20.2); storage size by category growing anomalously | none yet (§19.9 gap — add scenario when chaos suite lands) |
| [Suspected key compromise](suspected-key-compromise.md) | stub | external report; self-auditor consistency failure; unexpected entries in admin-actions compliance report (§20.3) | none (procedure-driven; exercised by game days) |
| [Adapter flood](adapter-flood.md) | stub | `entries_by_source_total` per-source rate spike; issuance latency p99 SLO (<10s) burn; adapter activity lag (§20.3) | `chaos-adapter-flood` |
