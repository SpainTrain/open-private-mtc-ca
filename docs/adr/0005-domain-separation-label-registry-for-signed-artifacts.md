# ADR-0005: Domain-separation label registry for signed artifacts

- **Status**: Accepted
- **Date**: 2026-08-05
- **Spec sections**: §14.1 (key roster), §5.4.1 (checkpoint signature input);
  draft-ietf-plants-merkle-tree-certs-03 §5.4.1. Related:
  [ADR-0003](0003-ecdsa-signature-scheme-local-iana-codepoints-and-high-s-acceptance.md),
  rules `fips-boundary-preserved`, `document-decisions`.

## Context

The holistic crypto audit
([docs/security/crypto-audit-2026-08-05.md](../security/crypto-audit-2026-08-05.md),
Finding 1 — its single most critical hardening item) observed that domain
separation across **signed artifacts** is inconsistent. Only the checkpoint
signs a domain-separated input: `MTCSubtreeSignatureInput` begins with the
16-byte label `mtc-subtree/v1\n\0` (draft-03 §5.4.1;
`crates/mtc/src/checkpoint/signature_input.rs`). The other three signed-artifact
types in §14.1 — pruning checkpoints (§15.2), revocation lists (§16.1), and
reporting/self-auditor outputs (§14.1) — have no signer yet and no label
defined.

§14.1 gives all four artifact types their own ECDSA P-256 key, but **nothing in
code ties a key handle to an artifact type**. The only barrier preventing a
signature over one artifact from being reinterpreted as another is that they use
different HSM keys — an operational/config property, not a code-enforced one.
The audit's oracle showed a `PruningCheckpoint` serialization is blocked from
aliasing a checkpoint signature input only by numeric range checks happening to
fail, not by construction. If a pruning/revocation/reporting signer is ever
wired to the checkpoint key handle (a plausible misconfiguration — all four are
the same algorithm), cross-protocol signature forgery / equivocation becomes
reachable.

## Decision

**We establish a system-wide invariant: no two signed messages under one key
can be confused. Every signed-artifact signature *input* MUST begin with a
unique 16-byte domain-separation label from the registry below, and every new
signed artifact MUST add its label to this registry.** The label is prepended to
the to-be-signed bytes before signing and re-derived (not read from the wire) by
the verifier, exactly as the checkpoint already does.

### Label encoding

Each label is a fixed **16-byte** field: the ASCII text `mtc-<artifact>/v<version>`
followed by a newline `\n` (0x0A), right-padded with NUL bytes (0x00) to 16
bytes. This reproduces the draft's checkpoint label exactly — `mtc-subtree/v1\n`
is 15 bytes, and its single trailing NUL is the one pad byte, giving the draft's
`mtc-subtree/v1\n\0`. The artifact text (before `/v<version>`) must be short
enough that `text + "/v" + version + "\n"` is ≤ 16 bytes (i.e. name + version
digits ≤ 14 bytes), leaving ≥ 1 NUL pad byte.

### Registry

| Artifact | Label text | Full 16-byte hex | text+nl / pad |
|---|---|---|---|
| Checkpoint (draft §5.4.1, existing) | `mtc-subtree/v1\n` | `6d74632d737562747265652f76310a00` | 15 / 1 |
| Pruning checkpoint (§15.2) | `mtc-prune/v1\n` | `6d74632d7072756e652f76310a000000` | 13 / 3 |
| Revocation list (§16.1) | `mtc-revoke/v1\n` | `6d74632d7265766f6b652f76310a0000` | 14 / 2 |
| Reporting / self-auditor (§14.1) | `mtc-report/v1\n` | `6d74632d7265706f72742f76310a0000` | 14 / 2 |

(Each row is exactly 16 bytes; the trailing `00` bytes are NUL padding.
Distinctness is guaranteed by the differing name prefixes; the fixed length
makes the label an unambiguous, non-extendable prefix of the signed input.
`text+nl / pad` = bytes of ASCII-text-plus-newline / NUL pad bytes.)

The label strings above are **normative**: signer beads implement them verbatim.
Checkpoint keeps its draft-defined `mtc-subtree/v1\n\0`. Pruning/revocation/
reporting labels are this project's own extension (those signers are not in
draft-03) and are versioned independently.

## Alternatives Considered

### A. Rely on key separation alone (status quo)
Rejected. §14.1's four-key separation is real in the design but enforced nowhere
in code; a single mis-wired key handle collapses it. The audit rates this the
highest-value hardening item precisely because the barrier is operational, not
structural. A structural second barrier (the label) is cheap and removes the
single-point-of-failure.

### B. A per-artifact type-tag byte instead of a 16-byte label
Rejected. A 1-byte discriminant would separate artifacts, but it diverges from
the checkpoint's existing draft-mandated 16-byte label, and a fixed human-readable
label is self-documenting in signed-blob dumps and matches the draft's own
convention. Uniformity with the checkpoint is worth the 15 extra bytes.

### C. Add the labels to code now (constants module in `crates/mtc`)
Deferred, not rejected. The label *values* are fixed here, but the per-signer
implementation belongs to each signer bead (pruning-signer `mtc-a8f.1`, and the
future revocation and reporting signers), which will add the constant next to
the code that prepends it and its test asserting the label is present and
distinct. This ADR is the registry of record they implement against.

## Consequences

### Positive
- Cross-artifact signature reinterpretation is blocked by construction, not just
  by key hygiene — closing the audit's Finding 1.
- New signed artifacts have a default they inherit: "add your label to ADR-0005."
- Signed-blob forensics is easier (the leading label names the artifact type).

### Negative
- Each signer bead carries a small, mandatory implementation obligation (prepend
  the label, test its presence/distinctness) that review must enforce.
- The name-length constraint (≤ 14 bytes before the newline) must be checked when
  a new artifact is registered; a longer name needs a different scheme.
- A version bump to any label (e.g. `mtc-prune/v2`) is a signing-format change and
  must itself be an ADR with a migration story, like ADR-0003's codepoint caveat.
