# ADR-0003: ECDSA signature scheme: local IANA codepoints and high-s acceptance

- **Status**: Accepted
- **Date**: 2026-07-23
- **Spec sections**: `draft-ietf-plants-merkle-tree-certs-03` §5.4.1–5.4.2;
  RFC 8446 §4.2.3 (TLS `SignatureScheme` registry); `draft-ietf-tls-mldsa`
  (ML-DSA codepoints); spec §14.1 (key roster), §14.4 (FIPS boundary), §22.11
  (determinism / injected-clock discipline), §28 (references). Rules:
  `.claude/rules/fips-boundary-preserved.md`, `.claude/rules/document-decisions.md`.

## Context

`crates/mtc` ticket `mtclib-signing` introduced the algorithm-agnostic
`SignatureScheme` abstraction and its v1 ECDSA P-256 implementation. Two design
questions arose where the MTC draft under-specifies and the answer binds
downstream tickets (`mtclib-checkpoint`, `mtclib-trust-anchor-id`,
`mtclib-ml-dsa-quxpqc`, and the ca-service checkpoint-signer). The
crypto-reviewer independently fetched draft-03 and recomputed the RFC 6979
vectors from scratch, and approved both decisions; this ADR records them and
their invariants before downstream work builds on them.

**Codepoints.** The `mtclib-signing` acceptance criteria call for "an algorithm
identifier registry matching the draft's codepoints." Draft-03 assigns **no
numeric codepoints**: §5.4.2 enumerates the signature algorithms *by name*
(ECDSA P-256/SHA-256, ECDSA P-384/SHA-384, Ed25519, ML-DSA-44/65/87), and
§5.4.1 defines `MTCSubtreeSignatureInput` with **no algorithm field** — the
algorithm is bound to the cosigner's public key, identified on the wire by its
cosigner / trust-anchor ID. The draft also prescribes no signature *encoding*
("Log clients … are assumed to be configured with all parameters necessary to
verify that cosigner's signatures, including the signature algorithm and version
of the signature format").

**High-s / determinism.** ECDSA signatures are malleable: for a valid `(r, s)`,
`(r, n − s)` is equally valid. RustCrypto `p256` signs deterministically
(RFC 6979) and, by default, does **not** low-s-normalize — its output matches
the raw RFC 6979 vectors byte for byte (our KAT includes the high-s "sample"
vector, which passing byte-exact confirms this). PKCS#11 / CloudHSM ECDSA also
emit raw, non-normalized `r || s` (spec §14). The repository-wide signature
contract is the fixed 64-byte `r || s` IEEE P1363 form documented on
`cloud_types::Hsm::sign`.

## Decision

### Decision A — Codepoints are a local IANA-derived identifier registry

We will represent the algorithm as a closed `SignatureAlgorithm` enum matching
the draft §5.4.2 named set, and assign each variant the **IANA TLS
`SignatureScheme` codepoint** as its numeric identifier (RFC 8446 §4.2.3:
`ecdsa_secp256r1_sha256 = 0x0403`, `ecdsa_secp384r1_sha384 = 0x0503`,
`ed25519 = 0x0807`; `draft-ietf-tls-mldsa`: `mldsa44/65/87 = 0x0904/0x0905/0x0906`).
`from_code` returns a structured `UnknownAlgorithm` for any other value.

- **(a) INVARIANT.** These codepoints are a **local identifier only** — for
  configuration, telemetry, and in-memory dispatch. They **MUST never appear in
  a draft-defined wire structure.** On-wire binding of algorithm to key stays
  via the cosigner / trust-anchor ID, because draft §5.4.1–5.4.2 carry no
  algorithm field. Any downstream serializer (`mtclib-checkpoint`,
  `mtclib-trust-anchor-id`) that emits one of these `u16` values into a
  draft-defined structure is violating this ADR.
- **(b) Provisional ML-DSA codepoints.** `0x0904`–`0x0906` come from an
  unpublished TLS draft (`draft-ietf-tls-mldsa`) and **could shift before RFC**.
  They are safe as ephemeral in-memory identifiers. Any feature that **persists**
  a codepoint (config files, stored records, metrics with long retention) must
  own an explicit migration story for a codepoint change; prefer persisting the
  stable `name()` string over the numeric code.

### Decision B — Sign raw RFC 6979 (high-s permitted); verify accepts high-s

We will produce raw RFC 6979 deterministic signatures without low-s
normalization (matching the RFC 6979 vectors and PKCS#11/HSM native output), and
verification will accept any well-formed `(r, s)` with `r, s ∈ [1, n)` —
high-s included — while rejecting the degenerate `r = 0` / `s = 0` and
out-of-range (`≥ n`) encodings. This keeps a software signer and a future
HSM-backed signer byte-compatible under one `verify` path (spec §14).

- **(1) INVARIANT.** Because signatures are malleable, **signature bytes MUST
  never be used as identifiers, deduplication keys, idempotency keys, or cache
  keys.** Note that `Signature` derives `Eq + Hash` (`crates/mtc/src/signing/mod.rs`,
  the `Signature` newtype) purely for test ergonomics, which makes such misuse
  easy to write by accident. If a future requirement ever needs
  content-addressed or canonical signatures, revisit low-s normalization **at
  that boundary** (e.g. a distinct canonicalizing wrapper), not by weakening
  this crate's HSM-compatible verify.
- **(2) Determinism is not idempotency.** This crate's RFC 6979 determinism
  **MUST NOT** be used to justify the write-path "deterministic idempotent
  checkpoint `PutObject`" (spec §11.1). Production checkpoint signing runs on the
  HSM, whose ECDSA is **randomized** — re-signing the same checkpoint yields
  different bytes. Idempotency of checkpoint publication must come from the
  object key / content addressing chosen by the checkpoint-signer ticket, and
  that caveat belongs to that ticket, not here.
- **(3) No context parameter.** The `SignatureScheme` trait signs raw
  `message: &[u8]` with no context / domain-separation parameter. Consequently:
  the future ML-DSA implementation (`mtclib-ml-dsa-quxpqc`) must fix the FIPS 204
  signing context internally (the empty context is the interoperable default),
  and constructing the domain-separated `MTCSubtreeSignatureInput` (the
  `mtc-subtree/v1` label, cosigner ID, and subtree, per draft §5.4.1) is owned
  solely by `mtclib-checkpoint` — this crate signs the already-assembled bytes.

## Alternatives Considered

### Alternative A — Invent MTC-specific codepoints, or key the registry by name only

Minting our own numeric codepoints would fabricate a registry the draft does
not define and that no other implementation shares. Keying purely by name
satisfies "unknown IDs parse to an error" but loses the compact, standards-
grounded identifier the TLS/PKI ecosystem already uses for exactly these
algorithms. Adopting the IANA TLS values reuses an existing registry without
claiming the draft assigns them. Rejected in favour of Decision A.

### Alternative B — Low-s-normalize on sign and/or reject high-s on verify

Enforcing low-s (BIP-0062 style) would make signatures canonical, but it
**breaks HSM parity**: PKCS#11/CloudHSM emit raw `r || s`, so a low-s verifier
would reject valid HSM signatures, and a low-s signer would diverge from both
the HSM and the RFC 6979 test vectors — costing the byte-exact KAT and the
single shared `verify` path. Malleability is not a threat for a CA verifying its
own checkpoints. We instead forbid signature-bytes-as-keys by invariant
(Decision B.1). Rejected.

## Consequences

### Positive

- One `verify` path serves software and HSM signers; the 64-byte P1363 contract
  matches `cloud_types::Hsm` exactly, so `mtclib-checkpoint` and the
  checkpoint-signer need no per-source branching (spec §14).
- Deterministic RFC 6979 signing gives byte-exact known-answer tests and
  reproducible dev/test fixtures.
- The registry is future-proof: `mtclib-ml-dsa-quxpqc` adds an implementation,
  not an enum variant, and a feature-off build still *parses* an ML-DSA
  codepoint and answers `UnsupportedAlgorithm` rather than panicking.

### Negative

- Signatures are malleable; the "never use signature bytes as a key" invariant
  (B.1) is a standing constraint reviewers must enforce, unaided by the type
  system (`Signature: Eq + Hash` does not prevent misuse).
- The ML-DSA codepoints are provisional (A.b): a `draft-ietf-tls-mldsa` change
  before RFC would require updating three constants here and a migration for any
  persisted codes.
- The abstraction carries no signing context (B.3): domain separation and the
  FIPS 204 context are downstream responsibilities that this crate cannot
  enforce, only document.
- Any later need for canonical signatures reopens the low-s question at a new
  boundary (B.1).
