---
title: Security Review Checklist (Phase 8)
spec: docs/mtc-architecture-spec.md
statuses: [unreviewed, pass, finding, N.A.]
verification-methods: [test, code inspection, config check, "n/a (scoped out)"]
lint: scripts/lint-security-checklist.sh
---

# Security Review Checklist (Phase 8)

This is the structured, trackable checklist for the Phase 8 security review
(spec §24, "Production Hardening: Security review"). Items are derived from the
threat model (§3) and the system's stated invariants. Performing the review and
recording outcomes happens under a separate ticket (`ops-security-review-v1`);
this document only defines the item inventory. All items therefore start as
`unreviewed`.

## Conventions

- **ID scheme**: `SEC-<AREA>-<NN>`. Areas: `KH` (key handling / HSM), `AO`
  (append-only / integrity), `EP` (epoch / split-brain), `RT` (retention /
  Object Lock), `RV` (revocation), `ADM` (admin surface), `SC` (supply chain),
  `IP` (input parsing), `ADV` (adversarial scenarios, §19.8), `OOS` (explicitly
  scoped out).
- **Verify** (verification method) is one of: `test` (an automated test
  demonstrates the property), `code inspection` (a reviewer reads the relevant
  code), `config check` (a reviewer inspects IaC / CI / tool configuration), or
  `n/a (scoped out)` (item is out of scope; see rationale in its Statement).
- **Evidence** holds the pointer produced or confirmed during review: a test
  path, file path, or config location. `—` means "to be filled during review".
  For `SEC-ADV-*` items the Evidence cell must, per the acceptance criteria,
  already contain either a pointer to the covering test or an explicit
  `GAP:` marker naming the planned coverage.
- **Status** is one of: `unreviewed` (not yet examined), `pass` (reviewed,
  property holds), `finding` (reviewed, problem found — file a bead and link
  it in Evidence), `N.A.` (not applicable; rationale required in Statement).
- **Machine checking**: every row whose first cell matches `SEC-*-NN` is
  linted by `scripts/lint-security-checklist.sh` (make target
  `lint-security-checklist`): all six cells present and non-empty, valid
  Verify and Status values, unique IDs, a `§` spec citation per item, all
  eight §19.8 scenarios present, and ADV evidence rules as above.

## 1. Key handling and HSM boundary (§14)

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-KH-01 | All four signing keys (checkpoint, pruning checkpoint, revocation list, reporting) are generated and used inside the HSM; private key material never crosses the HSM boundary, so binary compromise is not key compromise. | §3, §14.1 | code inspection | — | unreviewed |
| SEC-KH-02 | The reporting key is a distinct key from the issuance/checkpoint keys, so compliance-report signing cannot be confused with log signing. | §14.1, §20.3 | config check | — | unreviewed |
| SEC-KH-03 | Cross-region key management follows Option A (per-region CloudHSM cluster with a documented key replication ceremony); no cross-region HSM dependency exists on the hot signing path. | §14.2 | config check | — | unreviewed |
| SEC-KH-04 | FIPS boundary: exact versions of FIPS-validated crypto libraries (qux-pqc, applicable RustCrypto modules) are pinned via the Cargo lockfile for reproducible builds. | §14.4, §6 | config check | — | unreviewed |
| SEC-KH-05 | FIPS boundary: cargo-deny rules explicitly forbid Cargo features that swap crypto backends for non-validated implementations. | §14.4, §22.13 | config check | — | unreviewed |
| SEC-KH-06 | FIPS boundary: a CI gate runs a FIPS-validation check on every release artifact before publication; a dependency upgrade that silently removes FIPS validation fails the build. | §14.4, §22.13 | config check | — | unreviewed |
| SEC-KH-07 | FIPS boundary: production container image builds deny UPX, aggressive sccache caching, and similar optimizations that could strip or alter validated code paths. | §14.4 | config check | — | unreviewed |
| SEC-KH-08 | The Hsm trait exposes is_fips_validated(); SoftHSM (dev-only) builds are explicitly marked non-FIPS; compliance reports include the value; an environment-tagged CI check prevents a non-FIPS build from ever deploying to production. | §14.4, §20.3 | test | — | unreviewed |
| SEC-KH-09 | The HSM signing path never produces a signature over malformed input (cryptographic invariant test exists and passes). | §19.6, §14.1 | test | — | unreviewed |

## 2. Append-only and integrity invariants (§8, §11.2)

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-AO-01 | Once written, tile, entry, and checkpoint object bytes are never modified; no code path issues an overwrite of an existing log object key. | §8.1 (S3 invariants), §11.2 | code inspection | — | unreviewed |
| SEC-AO-02 | S3 Versioning is enabled on all log buckets in all three regions (defense in depth behind Object Lock). | §8.1 (S3 invariants) | config check | — | unreviewed |
| SEC-AO-03 | S3 Object Lock is configured in Compliance mode on all log buckets, providing true append-only storage even against privileged principals. | §8.1 (S3 invariants), §15.3 | config check | — | unreviewed |
| SEC-AO-04 | The index counter never decreases; indices from abandoned batches become permanent gaps filled with null_entry rather than being reallocated. | §11.2, §2 | test | — | unreviewed |
| SEC-AO-05 | The batch-commit DDB transaction (write-path step 8) is the single linearization point; writes are ordered S3-first, DDB-second so a failed transaction leaves only harmless orphan S3 objects. | §11.2, §11.3 | code inspection | — | unreviewed |
| SEC-AO-06 | The self-auditor independently re-fetches tiles, recomputes the root, verifies the CA signature, checks history consistency, and records an audit proof; detected anomalies page immediately and freeze issuance. | §20.2 (self-auditor), §3 | test | — | unreviewed |
| SEC-AO-07 | Non-equivocation: the CA publishes a single checkpoint sequence; any two checkpoints at the same tree size have the same root hash. | §3, §19.6 | test | — | unreviewed |

## 3. Epoch-conditional writes and split-brain (§8.3, §13.4)

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-EP-01 | Every coordination write (counter, checkpoint pointer, revocation pointer, batch records) carries the current epoch in a DynamoDB ConditionExpression. | §8.3, §11.2 | code inspection | — | unreviewed |
| SEC-EP-02 | Every lease takeover atomically increments the epoch in the same operation that claims the lease. | §8.3, §13.3 | test | — | unreviewed |
| SEC-EP-03 | A recovered old primary reads the lease, observes a different holder and a higher epoch, and stands down to read-only without attempting writes. | §13.4 | test | — | unreviewed |
| SEC-EP-04 | Standby regions monitor the lease but never write while not holding it; takeover is only attempted after the 60s TTL plus safety margin. | §8.3, §13.1 | code inspection | — | unreviewed |
| SEC-EP-05 | The promotion procedure verifies the latest checkpoint signature and tile-snapshot consistency (waiting for CRR catch-up) before claiming the lease, and abandons in-flight batches before resuming issuance. | §13.3 | code inspection | — | unreviewed |
| SEC-EP-06 | The lease/epoch protocol carries a Kani proof that no two regions can simultaneously hold a current-epoch lease. | §19.12 | test | — | unreviewed |

## 4. Retention and Object Lock (§15.3)

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-RT-01 | Lifecycle policies can delete pruned leaf objects only after the Object Lock (Compliance mode) retention period has expired; retention default is 7 years. | §8.1 (S3 invariants), §15.1, §15.2, §15.3 | config check | — | unreviewed |
| SEC-RT-02 | Cross-region replication preserves Object Lock attributes, so replicas are as tamper-resistant as the source objects. | §15.3 | config check | — | unreviewed |
| SEC-RT-03 | Pruning is never silent: a signed pruning checkpoint declaring the pruned range at a stated tree size is committed and replicated before any deletion occurs. | §15.1, §15.2 | test | — | unreviewed |
| SEC-RT-04 | Pruning checkpoints themselves are retained indefinitely (exempt from lifecycle deletion). | §15.3 | config check | — | unreviewed |
| SEC-RT-05 | Pruning runs only on the primary, enforced by the lease (a standby cannot initiate pruning). | §15.2, §8.3 | code inspection | — | unreviewed |

## 5. Revocation path (§16)

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-RV-01 | The RevocationList signature covers log_id, tree_size, revoked ranges, and signed_at, preventing splicing of ranges between lists or replay under a different tree size. | §16.1 | test | — | unreviewed |
| SEC-RV-02 | The emergency revocation flow writes the signed list to S3 at a deterministic key before atomically updating the latest-revocation DDB pointer (same S3-first ordering as the write path). | §16.3, §11.2 | code inspection | — | unreviewed |
| SEC-RV-03 | Distribution meets the latency target: 99% of relying parties hold the current revocation list within 15 minutes; emergency push triggers immediate refresh. | §16.2, §16.3 | test | — | unreviewed |
| SEC-RV-04 | Revocation add and distribute are privileged operations requiring justification plus incident reference, and are recorded with operator identity in the revocation and admin-actions reports. | §16.3, §17.3, §20.3 | code inspection | — | unreviewed |
| SEC-RV-05 | A revoked certificate's index fails verification; removing and re-adding a revoked index does not restore verifiability. | §19.6 | test | — | unreviewed |

## 6. Admin surface authn/authz and audit trail (§17, §20.3)

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-ADM-01 | Admin API authentication uses AWS IAM signed requests for all callers (CLI and UI); there is no unauthenticated admin endpoint. | §17.2, §17.3 | config check | — | unreviewed |
| SEC-ADM-02 | Privileged commands (cert revoke, failover initiate, revocation add, adapter pause) require explicit confirmation (--confirm or interactive; --yes reserved for automation) and cannot run implicitly. | §17.3 | test | — | unreviewed |
| SEC-ADM-03 | CLI/UI parity means both surfaces consume the same admin API, so authorization is enforced server-side once rather than duplicated (and possibly diverging) per surface. | §17.1, §17.2 | code inspection | — | unreviewed |
| SEC-ADM-04 | Every privileged admin call appears in the Admin actions compliance report with operator identity, and CLI invocations are logged with full reproducibility. | §20.3, §17.1 | test | — | unreviewed |
| SEC-ADM-05 | Compliance reports are signed by the dedicated reporting key, separate from issuance keys, so report forgery does not follow from report-path compromise alone. | §20.3, §14.1 | test | — | unreviewed |
| SEC-ADM-06 | Sensitive data is never logged; structured log entries carry the standard field set (correlation ids, epoch, batch id) and no key material or secrets. | §20.1 | code inspection | — | unreviewed |

## 7. Supply chain (§22.13, §6)

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-SC-01 | cargo audit (security advisories) runs as a required CI check on every PR and the build fails on unaddressed advisories. | §22.13 | config check | — | unreviewed |
| SEC-SC-02 | cargo deny check (license, advisory, and duplicate-dependency checks) runs as a required CI check on every PR. | §22.13 | config check | — | unreviewed |
| SEC-SC-03 | No AGPL code is forked or vendored: DigiCert mtc-bridge (AGPL-3.0) is read for design patterns only, and cargo-deny's license policy would reject an AGPL dependency. | §6 (licensing constraints) | code inspection | — | unreviewed |
| SEC-SC-04 | All dependencies conform to the approved license set in the §6 table (MIT, Apache-2.0, BSD-2/3-Clause), enforced via the cargo-deny configuration rather than by convention. | §6, §22.13 | config check | — | unreviewed |
| SEC-SC-05 | Vendored frontend assets (htmx.min.js, tree-viz.js) are pinned at known versions, embedded in the binary via rust-embed, and reviewed on update; no CDN or runtime-fetched scripts exist in the admin UI. | §17.4 | code inspection | — | unreviewed |
| SEC-SC-06 | FIPS-relevant dependency pins (SEC-KH-04/05/06) are covered by the same cargo-deny and CI gates, so a supply-chain substitution of a crypto backend fails CI. | §14.4, §22.13 | config check | — | unreviewed |

## 8. Input parsing hardening (§19.3)

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-IP-01 | A cargo-fuzz target exists for every externally-parseable type: Checkpoint, MTCProof, RevocationList, TBSCertificateLogEntry, Tile, and the ACME request body. | §19.3 (Layer 2) | test | — | unreviewed |
| SEC-IP-02 | Every spec type has a proptest round-trip property (parse(serialize(x)) == x) run with extended iteration counts (10,000+) in CI. | §19.3 (Layer 1) | test | — | unreviewed |
| SEC-IP-03 | Structured fuzzing via the arbitrary bridge exercises post-parse verification paths with semi-valid inputs (must not panic). | §19.3 (Layer 3) | test | — | unreviewed |
| SEC-IP-04 | Differential parsing against the bwesterb/mtc reference implementation runs as a nightly CI job; accept/reject disagreements are investigated. | §19.3 (Layer 4), §19.5 | test | — | unreviewed |
| SEC-IP-05 | Parser hardening properties hold on all four layers: no panic on any input, no unbounded allocation, bounded recursion depth (no infinite loops), and parsing time bounded by input length rather than content. | §19.3 (properties) | test | — | unreviewed |
| SEC-IP-06 | Fuzz corpora are checked into fuzz/corpus/; every bug-finding input is minimized and preserved as a permanent regression test in the unit suite. | §19.3 (corpus management) | config check | — | unreviewed |
| SEC-IP-07 | Memory-safety posture: unsafe_code is forbidden workspace-wide (explicit exceptions only for PKCS#11 FFI), and unwrap/expect are denied in non-test code. | §22.12 | config check | — | unreviewed |

## 9. Adversarial scenarios (§19.8)

Each §19.8 red-team scenario appears below with a pointer to its covering test,
or an explicit `GAP:` marker naming the planned coverage (no code exists yet in
this repository, so all items are currently gaps by construction).

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-ADV-01 | Malformed tiles fed to the proof generator are rejected without panic and cannot yield a valid-looking proof. | §19.8, §19.3 | test | GAP: no test yet; planned fuzz/fuzz_targets/parse_tile.rs plus adversarial proof-generator suite | unreviewed |
| SEC-ADV-02 | Stalled CRR plus an attempted promotion: promotion blocks (waits for catch-up) rather than promoting onto an inconsistent tile snapshot. | §19.8, §13.3, §19.9 | test | GAP: no test yet; planned chaos scenario chaos-crr-stall | unreviewed |
| SEC-ADV-03 | Wrong signatures from a "compromised" HSM are detected: checkpoint verification and the self-auditor reject them; issuance freezes rather than serving bad checkpoints. | §19.8, §19.6, §20.2 | test | GAP: no test yet; planned §19.6 cryptographic invariant test with fault-injecting mock Hsm | unreviewed |
| SEC-ADV-04 | Forced clock skew (±60s) between regions does not break lease semantics or allow premature takeover. | §19.8, §8.3, §19.9 | test | GAP: no test yet; planned chaos scenario chaos-clock-skew using the Clock trait (§22.11) | unreviewed |
| SEC-ADV-05 | Duplicate index allocation attempts fail: the counter UpdateItem's conditional write makes concurrent allocation of the same index range impossible. | §19.8, §11.2, §19.12 | test | GAP: no test yet; planned Kani proof of the write-path linearization point plus property test | unreviewed |
| SEC-ADV-06 | Stale-epoch writes from a deposed primary are rejected by the epoch ConditionExpression on every coordination write. | §19.8, §8.3, §13.4 | test | GAP: no test yet; planned chaos scenario chaos-split-brain and epoch conditional-write integration test | unreviewed |
| SEC-ADV-07 | Replayed old checkpoints are detected: consumers and the self-auditor reject a checkpoint pointer that regresses tree size or diverges from audited history. | §19.8, §20.2 | test | GAP: no test yet; planned self-auditor history-consistency test | unreviewed |
| SEC-ADV-08 | Crafted public keys attempting key-hash collisions do not allow one entry to impersonate another (TBSCertificateLogEntry binds the public key hash with domain separation). | §19.8, §2, §19.6 | test | GAP: no test yet; planned cryptographic invariant test for domain-separated key hashing | unreviewed |

## 10. Explicitly scoped out (§1 non-goals, §3 out of scope)

The following review areas would appear in a production CA security review but
are deliberately out of scope here. Per the acceptance criteria they are listed
with rationale (status `N.A.`) rather than silently omitted. If the project's
stance changes (§1), these items must be re-opened before any production use.

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-OOS-01 | Cosigner infrastructure review: not applicable — v1 is a single-trust-boundary internal CA with no cosigners; the self-auditor (§20.2) is the compensating control for cosigner-free operation. | §1 (non-goals), §3, §20.2 | n/a (scoped out) | — | N.A. |
| SEC-OOS-02 | Public Web PKI obligations (root program requirements, external CT logging, public trust store handling): not applicable — this is explicitly not a Chrome-trusted public CA. | §1 (non-goals), §3 | n/a (scoped out) | — | N.A. |
| SEC-OOS-03 | Inter-organizational trust-boundary review: not applicable — the CA and all relying parties share a single trust boundary by assumption. | §1 (non-goals), §3 | n/a (scoped out) | — | N.A. |
| SEC-OOS-04 | Insider attack by a fully-trusted CA operator: out of scope per the threat model; mitigations are limited to HSM controls and the key ceremony, and full insider-threat review is deferred. | §3 (out of scope), §14 | n/a (scoped out) | — | N.A. |
| SEC-OOS-05 | Quantum cryptanalysis of the CA signing keys: out of scope per the threat model; the mitigation is the planned ML-DSA migration path (v2 algorithms in §14.1), not review of current-classical-key resistance. | §3 (out of scope), §14.1 | n/a (scoped out) | — | N.A. |
| SEC-OOS-06 | AWS account compromise and general cloud governance review: out of scope — the AWS infrastructure boundary (S3, DynamoDB, CloudHSM) is trusted per the threat model and standard governance is assumed. | §3 (out of scope) | n/a (scoped out) | — | N.A. |
| SEC-OOS-07 | Automated failover decision safety: not applicable in v1 — failover is a deliberate manual decision with tooling; the automated path must be reviewed when v2 introduces it. | §1 (non-goals), §13.2 | n/a (scoped out) | — | N.A. |
| SEC-OOS-08 | Production deployment hardening (penetration test, production account controls, commercial support and disclosure obligations): not applicable — this is a non-production reference blueprint developed entirely against the zero-cost local simulation (§18). | §1 (non-goals) | n/a (scoped out) | — | N.A. |

## Review workflow

1. Run `make lint-security-checklist` before and after editing this file.
2. During the review (`ops-security-review-v1`), set each item to `pass` or
   `finding`, filling Evidence with the test path, code location, or config
   location examined. Findings get a bead; link its ID in Evidence.
3. Items may only move to `N.A.` with a written rationale in the Statement,
   grounded in §1 or §3.
