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
| [ADR-0003](0003-ecdsa-signature-scheme-local-iana-codepoints-and-high-s-acceptance.md) | ECDSA signature scheme: local IANA codepoints and high-s acceptance | Accepted | Draft-03 assigns no signature codepoints/encoding; adopt IANA TLS `SignatureScheme` values as a LOCAL-only identifier (never on-wire), and sign/verify raw RFC 6979 (high-s permitted) for HSM parity — with invariants for downstream checkpoint/trust-anchor/ML-DSA tickets. |
| [ADR-0004](0004-replication-simulator-scan-diff-ddb-tailing-with-timestamp-stamped-last-writer-wins.md) | Replication simulator: scan-diff DDB tailing with timestamp-stamped last-writer-wins | Accepted | dev-replicator tails DynamoDB via Scan-diff (not Streams) and resolves conflicts with a hidden per-item timestamp + conditional write, documenting last-writer-wins explicitly (spec §18.3, §8.2). |
| [ADR-0005](0005-domain-separation-label-registry-for-signed-artifacts.md) | Domain-separation label registry for signed artifacts | Accepted | System-wide invariant: every signed artifact's signature input begins with a unique 16-byte domain label (checkpoint `mtc-subtree/v1`, pruning `mtc-prune/v1`, revocation `mtc-revoke/v1`, reporting `mtc-report/v1`), so cross-artifact signature reinterpretation is blocked by construction, not just by key separation (audit Finding 1). |
| [ADR-0006](0006-cloud-softhsm-pkcs-11-hsm-backend-against-softhsm2-via-cryptoki.md) | cloud-softhsm PKCS#11 Hsm backend against SoftHSM2 via cryptoki | Accepted | Implement the `Hsm` trait over the safe `cryptoki` PKCS#11 wrapper (no unsafe, no FFI exception); sign with `CKM_ECDSA` over an in-Rust SHA-256 digest to emit the 64-byte P1363 `r‖s` of ADR-0003; export SPKI via RustCrypto; non-extractable keys; `is_fips_validated()==false` (spec §9.3, §14). |
| [ADR-0007](0007-lease-takeover-safety-margin-is-additive-to-the-60s-ttl.md) | Lease takeover safety margin is additive to the 60s TTL | Accepted | §8.3's "expiry beyond 60s safety margin" is read as additive (`now ≥ expires_at + 60s`, a constant distinct from the 60s TTL); epoch fencing makes this a clock-skew/availability tuning knob, not a split-brain safety one, so the conservative ~120s worst-case failover is chosen and is a one-constant flip to revisit (mtc-brv6). |
| [ADR-0008](0008-aws-sdk-tls-stack-upgrade-off-vulnerable-rustls-webpki-aws-lc-rs.md) | aws-sdk TLS stack upgrade off vulnerable rustls-webpki (aws-lc-rs) | Proposed | cloud-aws/dev-replicator: swap aws-sdk-s3/aws-sdk-dynamodb's misleadingly-named `rustls` feature (legacy hyper-0.14/rustls-0.21, vulnerable rustls-webpki 0.101.7, RUSTSEC-2026-0098/0099/0104) for `default-https-client` (hyper-1.x/rustls-0.23/aws-lc-rs); feature-flag-only fix, zero source changes, clears the advisory and a pre-existing duplicate; outside the §14.4 FIPS boundary (HSM-signing only, no `Hsm` impl in cloud-aws yet). |
<!-- adr-index-end -->
<!-- markdownlint-enable MD060 -->
