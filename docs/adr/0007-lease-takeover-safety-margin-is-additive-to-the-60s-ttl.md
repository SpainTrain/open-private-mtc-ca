# ADR-0007: Lease takeover safety margin is additive to the 60s TTL

- **Status**: Accepted
- **Date**: 2026-08-06
- **Spec sections**: §8.2 (primary-region lease schema), §8.3 (lease/epoch
  renewal + takeover). Related: `kani-for-critical-paths`, `use-newtypes`,
  `document-decisions` rules; implemented by bead `mtc-brv6`
  (`crates/coordination`).

## Context

Spec §8.3 defines the primary-region lease with two timing constants —
"Renewed every 20s by holder; 60s TTL" — and then states the takeover
condition in one terse line:

> Expiry beyond 60s safety margin makes lease takeover-eligible.

The implementation (`crates/coordination`) must turn that into a concrete
predicate, and the sentence admits two readings:

- **(A) additive** — the *safety margin* is a second 60-second buffer applied
  *after* the lease's own expiry: takeover-eligible when
  `now ≥ expires_at + 60s`. (`expires_at` is itself `last_renewal + 60s TTL`.)
- **(B) margin ≡ TTL** — the 60s TTL *is* the safety margin, so takeover is
  eligible the moment the lease expires: `now ≥ expires_at`.

Crucially, this choice is **not** a correctness/split-brain knob. §8.3's own
next two bullets — "every takeover atomically increments `epoch`" and "every
write includes `epoch` in ConditionExpression — old primaries cannot write
after epoch advance" — mean a demoted primary is *fenced by epoch regardless of
when takeover happens*. The only thing the margin changes is **failover latency
vs. robustness to cross-region clock skew**: a shorter margin fails over faster;
a longer margin tolerates more skew before a (still-safe) takeover fires,
avoiding spurious primary churn.

## Decision

**We adopt reading (A): a lease is takeover-eligible when
`now ≥ expires_at + TAKEOVER_SAFETY_MARGIN`, where `TAKEOVER_SAFETY_MARGIN =
60s`, a constant distinct from the 60s `LEASE_TTL`.** Worst-case time from a
holder's last successful renewal to another region becoming takeover-eligible is
therefore ≈ **120s** (60s TTL + 60s margin).

This is implemented as the single pure predicate
`takeover_eligible_millis(now, expires_at, margin)` in
`crates/coordination/src/protocol.rs`, with `TAKEOVER_SAFETY_MARGIN` a named
constant, so the reading is expressed in exactly one place.

Rationale:
- **Textual fit.** "Expiry *beyond* [a] 60s safety margin," and the `mtc-brv6`
  acceptance criterion "takeover only permitted when `expires_at` is *past* the
  60s safety margin," both read most naturally as *the expiry event must be 60s
  in the past* — i.e. additive.
- **Correctness is already guaranteed elsewhere.** Epoch fencing (§8.3) makes
  the margin a tuning constant, not a safety mechanism, so we are free to choose
  the more conservative value.
- **Skew tolerance.** The additive margin absorbs up to ~60s of clock skew
  between the failing holder and a standby before a takeover is triggered,
  reducing unnecessary epoch churn.
- **A CA favors safety/correctness over availability.** A slightly slower
  failover is preferable to eager takeovers, given fencing already precludes
  data corruption.

## Alternatives Considered

### Alternative B — margin ≡ TTL (`now ≥ expires_at`)

Rejected as the default. It fails over faster (~60s) but leaves **zero** buffer
for clock skew, so any standby whose clock runs ahead of the holder's would
declare the lease takeover-eligible while the holder still believes it holds it.
Fencing keeps that safe, but it produces avoidable primary churn. It is not
foreclosed: it is reachable by a **one-constant flip**
(`TAKEOVER_SAFETY_MARGIN = Duration::from_secs(0)`), and the entire eligibility
path stays the same pure function — so if operational data later favors faster
failover, the change is trivial and returns here to supersede this ADR.

### Alternative C — make the margin runtime-configurable now

Deferred. A config surface for the margin is reasonable eventually, but v1 pins
a single reviewed constant (matching how `RENEWAL_INTERVAL`/`LEASE_TTL` are
pinned) rather than adding configuration ahead of a demonstrated need.

## Consequences

### Positive

- Single-writer correctness is unaffected by the choice (epoch fencing), so this
  decision carries no split-brain risk.
- Up to ~60s of cross-region clock skew is tolerated without spurious takeovers.
- The reading lives in one named constant + one pure predicate, keeping it
  testable (the `mtc-brv6` proptest and `run_lease_suite` exercise the boundary)
  and trivially tunable.

### Negative

- Worst-case failover latency is ~120s rather than ~60s; if that proves too slow
  operationally, a superseding ADR flips `TAKEOVER_SAFETY_MARGIN` toward 0.
- The two 60s constants (`LEASE_TTL` and `TAKEOVER_SAFETY_MARGIN`) are numerically
  equal but semantically distinct; they must stay separate constants so a future
  change to one does not silently move the other.
