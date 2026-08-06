# ADR-0005: cloud-softhsm PKCS#11 Hsm backend against SoftHSM2 via cryptoki

- **Status**: Accepted
- **Date**: 2026-08-05
- **Spec sections**: `docs/mtc-architecture-spec.md` §9.3 (backend crates), §14
  (HSM / key roster), §14.1 (v1 ECDSA P-256), §14.3 (<100ms p99 signing),
  §14.4 (FIPS boundary), §18.1 (local dev), §22.12 (`unsafe_code` lint). Rules:
  `.claude/rules/no-unsafe.md`, `.claude/rules/fips-boundary-preserved.md`,
  `.claude/rules/no-sdk-types-in-domain.md`. Prior art: ADR-0003 (repository
  P1363 / high-`s` signature contract); `docs/lint-policy.md` PKCS#11 exception
  policy.

## Context

The `softhsm-backend` ticket implements `crates/cloud-softhsm` — the
`cloud_types::Hsm` trait over PKCS#11 against SoftHSM2, the dev-mode stand-in
for CloudHSM (spec §9.3, §14, §18.1). Three decisions bind downstream work (the
future CloudHSM backend `cloud-aws-cloudhsm`, the shared `run_hsm_suite`
conformance wiring, and the checkpoint-signer that consumes `Hsm::sign`) and are
exactly the surface crypto-review audits, so they are recorded before that work
builds on them.

**FFI vs. safe wrapper.** Talking PKCS#11 means calling a C shared library.
`docs/lint-policy.md` reserves one `unsafe_code` exception for a hand-rolled
`pkcs11-sys` FFI crate: it must declare its own `[lints]` table with
`unsafe_code = "deny"` (not the workspace `forbid`), carry a scoped
`#![allow(unsafe_code)]` with `// SAFETY:` comments, and land with an ADR. That
exception exists only if no safe wrapper will do.

**Signature encoding.** `Hsm::sign` mandates the fixed 64-byte `r || s`
IEEE P1363 encoding (ADR-0003), and the shared conformance suite verifies with
RustCrypto `p256` against the exported SPKI DER public key — so the mechanism,
the digest step, and the public-key export must all agree with that verifier or
the suite fails.

**FIPS posture.** SoftHSM2 is a software token and is explicitly not
FIPS-validated (spec §14.4); the boundary rule forbids private key material
leaving the HSM.

## Decision

**We will implement `cloud-softhsm` in pure safe Rust over the `cryptoki`
crate**, a well-maintained safe PKCS#11 wrapper that owns all `unsafe` FFI
internally and `dlopen`s the module at runtime. The crate therefore inherits the
workspace `unsafe_code = "forbid"` lint via `[lints] workspace = true` and
**does not take the `docs/lint-policy.md` PKCS#11 FFI exception** — that
exception remains reserved for a hypothetical `pkcs11-sys` path no crate
currently needs. Because `cryptoki` links the module at runtime, the crate
builds and unit-tests with no C library present; only the feature-gated
integration tests need a live SoftHSM2.

**We will sign with `CKM_ECDSA` over an in-Rust SHA-256 digest of the message.**
`CKM_ECDSA` signs a pre-computed hash and emits raw, non-normalized `r || s`
(32-byte big-endian `r` ‖ 32-byte big-endian `s` = 64 bytes for P-256) — exactly
the P1363 contract of ADR-0003, byte-compatible with the RustCrypto software
signer and (later) CloudHSM. The digest is computed in Rust (not via
`CKM_ECDSA_SHA256`) so the backend's "SHA-256 the message" behavior is explicit,
deterministic, and independent of per-token mechanism support.

**We will export the public key by reading `CKA_EC_POINT` and re-encoding SPKI
DER through RustCrypto `p256`** (`PublicKey::from_sec1_bytes` →
`to_public_key_der`), which guarantees the export round-trips with the same
verifier the conformance suite uses, rather than hand-assembling
`SubjectPublicKeyInfo` ASN.1.

**We will make generated private keys non-exportable** (`CKA_SENSITIVE = true`,
`CKA_EXTRACTABLE = false`) and never wrap or read private attributes, so no
private key material crosses the trait boundary
(`fips-boundary-preserved`). `is_fips_validated()` returns `false`.

**We will identify keys by a token-persistent `CKA_LABEL`** carried in the
`KeyHandle` string. `generate_key` mints a fresh label `mtc-key-<hex>` from 16
bytes of the token RNG (also the `CKA_ID`); `sign` / `get_public_key` locate the
object by label, so a caller can also reference a pre-provisioned key (e.g.
`checkpoint-signing`) by constructing that handle. Unknown handles return
`CloudError::NotFound`, never a panic.

**Concurrency:** each operation opens its own short-lived PKCS#11 session on the
`CKF_OS_LOCKING_OK` library context and runs under `tokio::task::spawn_blocking`
(a `Session` is `!Sync` and the calls are blocking C FFI), giving genuine
parallel signing without sharing a session across threads.

## Alternatives Considered

### Alternative A — Hand-rolled `pkcs11-sys` FFI under the unsafe exception

Rejected. It would reopen the memory-safety hazards `unsafe_code = "forbid"`
exists to exclude (spec §22.12), require the full `docs/lint-policy.md`
exception ceremony (own `[lints]` table, scoped allows, `// SAFETY:` audit
surface), and buy nothing over `cryptoki`, which is maintained by the Parsec
project and already wraps the same C ABI safely. The safe wrapper keeps the
whole crate inside the language's guardrails.

### Alternative B — `CKM_ECDSA_SHA256` (hash-in-token) instead of pre-hashing

Rejected as the default. Letting the token hash removes the explicit SHA-256
step and makes the digest depend on per-token mechanism support and quirks. Both
produce a signature over SHA-256(message), but pre-hashing keeps the digest
deterministic and portable and mirrors the memory backend's behavior exactly,
which matters for the shared conformance suite.

### Alternative C — Low-`s` normalize / hand-built SPKI

Rejected. Low-`s` normalization would break HSM/RFC-6979 byte parity (ADR-0003).
Hand-assembling SPKI ASN.1 would duplicate `p256`'s encoder and risk a subtle
mismatch with the verifier; re-encoding through `p256` is correct by
construction.

## Consequences

### Positive

- The crate stays 100% safe Rust under `unsafe_code = "forbid"`; the FFI
  exception is never spent, and there is less for crypto-review to audit.
- `sign` output is byte-identical in form to the RustCrypto software signer and
  the future CloudHSM backend — one 64-byte P1363 `verify` path serves all
  three (ADR-0003, spec §14). Verified live: a real SoftHSM2 `CKM_ECDSA`
  signature is 64 bytes and verifies under the exported SPKI key with `p256`.
- The PKCS#11 session layer here is the code path `cloud-aws-cloudhsm` will
  reuse with a different module/credentials, keeping the v1 production design
  honest without cloud spend (spec §14.2, §1).

### Negative

- A new third-party dependency surface (`cryptoki` + `cryptoki-sys`) enters the
  crypto path; a version bump must be reviewed for behavioral change like any
  crypto dependency.
- PKCS#11 `C_Initialize`/`C_Finalize` are process-global per module: multiple
  live `SoftHsm` instances in one process, or parallel integration tests, can
  race. Mitigated by sharing one instance and running integration tests
  `--test-threads=1`; a process-global context singleton is deferred to the
  conformance wiring (`cloud-softhsm-conformance`).
- Generated test keys are token-persistent and accumulate across runs; explicit
  teardown/idempotency is owned by `cloud-softhsm-conformance`.
- `is_fips_validated() == false` must remain wired into the compliance report
  and the FIPS CI gate so this dev backend can never reach production (spec
  §14.4, `fips-boundary-preserved`).
