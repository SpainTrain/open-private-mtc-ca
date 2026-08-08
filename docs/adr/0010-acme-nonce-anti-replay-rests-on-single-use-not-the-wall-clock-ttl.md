# ADR-0010: ACME nonce anti-replay rests on single-use, not the wall-clock TTL

- **Status**: Accepted
- **Date**: 2026-08-07
- **Spec sections**: RFC 8555 §6.5 (replay protection), §7.2 (new-nonce);
  architecture §22.11 (the `Clock` trait). Related:
  `crates/acme-core/src/nonce.rs`. Bead: `mtc-1hp.4`.

## Context

The ACME server (`acme-core`) issues anti-replay nonces carrying a wall-clock
TTL (`DEFAULT_NONCE_TTL_MILLIS` = 5 minutes). `mtc-1hp.4` asks for a crypto
sign-off: **is a wall-clock TTL a sufficient anti-replay guarantee?**

RFC 8555 §6.5 requires that a nonce be **single-use** — the server must consider
a nonce invalid after the client uses it once. A TTL *alone* (a nonce accepted
repeatedly until it expires) would permit replay within the window and would
**not** satisfy §6.5.

## Decision

**Anti-replay rests on single-use, not on the TTL — and the current
implementation already enforces this.** `NonceStore::consume` removes the nonce
from the issued set on first use (`HashMap::remove`); a replayed, never-issued,
or expired nonce all fail identically with `badNonce`. The wall-clock TTL — read
through the **injected `Clock`** (§22.11; never `SystemTime::now()` directly) —
serves **only** to bound memory: unused, expired nonces are pruned on each
`issue`.

This is verified by `crates/acme-core/src/nonce.rs`'s tests, notably
`issued_nonce_is_consumable_exactly_once` (consume → `Ok`, consume again →
`badNonce`), plus `unknown_nonce_is_rejected` and `expired_nonce_is_rejected`.

Because anti-replay is single-use, the design is **robust to clock anomalies**:
a consumed nonce is already removed, so no backward clock jump can re-enable its
replay. Clock behavior affects only the *expiry of unused nonces* — a client
whose unused nonce expires simply receives `badNonce` and retries with a fresh
one, exactly the recovery RFC 8555 §6.5 prescribes.

## Alternatives Considered

### A. TTL-only expiry (no single-use tracking)
Rejected. It permits replay within the window, violating RFC 8555 §6.5's
single-use requirement. This is the option the sign-off explicitly rules out.

### B. Persistent / cross-region nonce store
Deferred, not rejected. The in-memory single-use store is correct for the
single-region v1 (RFC 8555 permits per-server nonce scoping). Durable or
multi-region nonce sharing, if ever required, is a future ticket; it would not
change the single-use *principle* recorded here, only where the "used" set
lives.

## Consequences

### Positive
- Anti-replay meets RFC 8555 §6.5 and cannot be silently weakened by a change to
  the TTL, which this ADR fixes as a *memory bound, not a security control*.
- No wall-clock dependency in the security argument — the injected `Clock` keeps
  the store deterministically testable (`FakeClock` time-advance tests).

### Negative
- The nonce set is in-memory and single-process: a server restart invalidates
  outstanding nonces (clients retry with `badNonce` — acceptable per §6.5), and
  the design does not yet span regions. Both are explicitly v1 scope, noted here
  so the durability/multi-region question is a deliberate future decision rather
  than an assumed gap.
