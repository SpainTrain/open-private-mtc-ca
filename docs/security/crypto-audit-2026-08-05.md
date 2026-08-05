# Holistic Cryptographic Audit — 2026-08-05

Senior, adversarial, read-only pass over the whole implemented crypto surface
(`crates/mtc`, `mtc-verify`, `mtc-read`, `acme-core`, ~10.7k LOC). Scope was
deliberately cross-cutting: composition and domain-separation issues that the
per-bead crypto reviews (each already passed at merge) structurally cannot see.

## Executive assessment

The implemented cryptographic core is **sound**. The end-to-end composition
(leaf hash → inclusion proof → checkpoint signature) actually proves what it
claims: the leaf the tree commits to is byte-identical to what `mtc-verify`
reconstructs and what the proof is checked against, and the signature covers the
same root the proof rebuilds. **No Critical or High findings** in what is built,
asserted only after genuine attempts to break it (end-to-end oracle, a real
ECDSA `(r, n−s)` malleability collision, and byte-for-byte confirmation of the
three crown-jewel layouts against draft-ietf-plants-merkle-tree-certs-03).

The real risks are cross-cutting and forward-looking:
- Domain separation across **signed artifacts** is inconsistent — only the
  checkpoint is self-domain-separated; the other three artifact types
  (pruning / revocation / reporting) currently rely on **unenforced key
  separation** to avoid confusion.
- The tree↔entry leaf framing that makes the whole chain sound is a **caller
  contract, not a type-enforced one**, and the write path that must honor it is
  not built yet.

Both should be closed before the pruning/revocation signers and the ca-service
write path land.

## Validation performed (falsifiable)

- Throwaway oracle crate against the real `mtc`/`mtc-verify` + p256 0.13:
  - `tree.leaf_hash(i) == LogEntry::leaf_hash(entry_i)` → **true** (leaf
    byte-identity across the write/read seam holds).
  - Feeding the tree raw TBS bytes (no `LogEntry` discriminant) → `verify_inclusion`
    returns `WrongRoot`. Confirms framing is a caller contract, not type-enforced.
  - Produced the `(r, n−s)` counterpart of a checkpoint signature by explicit
    modular negation: **two distinct 64-byte signatures both verify** (malleability
    is real and live in verify, as ADR-0003 intends).
  - A `PruningCheckpoint` serializes with **no domain label** (first bytes are
    `pruned_start`); a checkpoint signature input starts with `mtc-subtree/v1\n\0`.
    Aliasing the label would require specific 64-bit field values — not practically
    reachable, but blocked only by numeric range checks, not by construction.
- Fetched draft-03; confirmed byte-for-byte: leaf `HASH(0x00‖entry)` / node
  `HASH(0x01‖l‖r)`; `MTCSubtreeSignatureInput` label `"mtc-subtree/v1\n\0"` with
  field order label→cosigner_id→MTCSubtree{log_id,start,end,hash}; entry
  discriminants `null_entry(0)`/`tbs_cert_entry(1)`. Security-critical layouts are
  draft-accurate, not merely self-consistent.

## Findings (severity-ranked)

### Medium 1 — Signed-artifact domain separation is inconsistent; relies on unenforced key separation
`checkpoint/signature_input.rs` has the `mtc-subtree/v1\n\0` label; `pruning_checkpoint.rs`
`tls_serialize` starts straight at `pruned_start` (no label); revocation (§16.1) and
reporting (§14.1) signers aren't built and define no label. §14.1 gives all four
artifact types their own ECDSA P-256 key, but nothing in code ties a key handle to
an artifact type — the only barrier against reinterpreting a signature over one
artifact as another is that they use different HSM keys (an operational property).
If a pruning/revocation/reporting signer is ever wired to the checkpoint key handle
(plausible misconfig — all four same algorithm), cross-protocol forgery/equivocation
becomes reachable. **Fix:** every signed artifact gets its own 16-byte domain label
in the signed input (`mtc-prune/v1`, `mtc-revoke/v1`, `mtc-report/v1`), as a
system-wide invariant: *no two signed messages under one key can be confused.*
Tracked: mtc-a8f.1 (broaden from pruning-only) + governing invariant bead.

### Medium 2 — Tree↔entry leaf framing is a caller contract, not type-enforced (write-path trap)
`MerkleTree::append(&mut self, entry: &[u8])` takes raw bytes; RP reconstructs via
`LogEntry::leaf_hash` = `hash_leaf(LogEntry::tls_serialize)` (with the `00 00`/`00 01…`
discriminant). Soundness of the whole chain depends on the write path appending
exactly `LogEntry::tls_serialize_to_vec()` bytes; nothing enforces it. Appending raw
TBS bytes yields a tree that silently fails RP verification for every entry
(availability break), and any framing where a cert entry could serialize to the
null-entry `00 00` would undermine null-entry unforgeability (prevented today only
because both sides go through `LogEntry`). The write path (§11.4) is still pseudocode,
so the contract is unpinned where it will be consumed. **Fix:** before ca-service,
make the tree ingest `&LogEntry` (or a `LeafBytes` newtype produced only by
`LogEntry` serialize) so committed-bytes framing is the same code path as RP
reconstruction; at minimum pin it with an end-to-end conformance vector.

### Medium/Low 3 — Malleability inertness depends on downstream discipline not built yet
Verify accepts high-s (ADR-0003 B); two distinct valid signatures exist per checkpoint.
ADR-0003 B.1/B.2 forbid using signature bytes as identity/dedup/idempotency/cache keys,
but those are documented constraints the type system can't enforce, and the consumers
that must honor them (checkpoint-signer's idempotent PutObject §11.1 step 8; the
non-equivocation monitor §3) aren't built. Not exploitable today; a standing trap.
**Fix:** when the signer and monitor land, key S3 idempotency and non-equivocation
dedup on `(log_id, tree_size, root_hash)`, never on signature bytes; add a regression
test that a malleated `(r, n−s)` signature creates no second checkpoint object and no
false equivocation alert.

### Informational
- `signed_at` unauthenticated (correct per draft §5.4.1); freshness must come from the
  distribution/landmark layer — non-equivocation-by-omission isn't detectable from a
  checkpoint alone. Ties mtc-qka.6.
- `log_id` not bound in `verify_inclusion` (documented, surfaced via `Verified::log_id()`,
  deferred to read-verify-cert). Acceptable for single-log/single-key CA; the cert layer
  must close it.
- ACME nonce consumed before signature verification (low impact; nonces are free;
  single-use anti-replay holds). Ties mtc-1hp.
- `cosigner_id == log_id` hardcoded — correct for this cosigner-free CA.
- ACME alg-confusion is closed (exact ES256, EC/P-256 JWK, none/RS256 rejected).

## Cross-cutting conclusions
1. **End-to-end soundness holds.** Committed leaf = reconstructed leaf = proof-checked
   leaf; signature covers exactly the root the proof rebuilds. Only seam is Finding 2.
2. **Domain separation.** Tree hashing strongly separated (leaf/node prefix + entry
   discriminant). Signature inputs separated for checkpoint only — Finding 1. No
   hashed/signed string can be confused with tree preimages (prefixes `0x00`/`0x01` vs
   signature inputs starting `0x6d`). The gap is strictly among the four signed artifacts.
3. **Key separation.** §14.1's four-key separation is assumed by design, enforced nowhere
   in code (Finding 1). Signature bytes aren't used as identifiers anywhere implemented.
   Malleability inert today; see Finding 3.
4. **Unverified-vs-draft gap (mtc-qka.5) is interop, not security** — the security-critical
   layouts were confirmed against draft-03; the remaining invented values are local-only
   (ADR-0003 A.a) or symmetric write/read whose exact number doesn't affect security.

## Gap analysis — production needs, ranked by threat-model load
1. Checkpoint signer + HSM/PKCS#11 (FIPS boundary). Only verify exists. Must land with
   Finding 3's idempotency discipline and Finding 2's framing pinned.
2. Pruning/revocation/reporting signers WITH domain labels (Finding 1 / mtc-a8f.1).
3. Non-equivocation / consistency monitoring (§3 core goal) — primitives exist, nothing
   runs them across published checkpoints; dedup on `(log_id,tree_size,root_hash)`.
4. Certificate layer (read-verify-cert): log_id/trust-anchor + revocation binding.
5. ML-DSA (feature-gated) + landmark/signatureless verification. Abstraction is ready.

## On the accumulated finding beads
Appropriately-scoped tail items with one systemic thread through mtc-a8f.1: **domain
separation of signed artifacts is being added artifact-by-artifact rather than as a
global invariant.** The one item not previously on the list is Finding 2 (tree ingests
raw `&[u8]`), filed against the tree/ca-service seam before the write path is implemented.
