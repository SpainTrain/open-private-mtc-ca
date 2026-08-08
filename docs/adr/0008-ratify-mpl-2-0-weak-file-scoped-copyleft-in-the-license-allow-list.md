# ADR-0008: Ratify MPL-2.0 (weak file-scoped copyleft) in the license allow-list

- **Status**: Accepted
- **Date**: 2026-08-07
- **Spec sections**: §6 (licensing approach / allow-list). Related:
  `deny.toml`, `docs/license-policy.md`, rule `fips-boundary-preserved`
  (dependency-hygiene precedent). Bead: `mtc-t92.2`.

## Context

`cssparser` and `dtoa-short` are licensed **MPL-2.0**. They enter the workspace
as transitive dependencies of `ammonia` — the HTML sanitizer `mtc-admin-api-server`
uses to sanitize operator-supplied strings before they reach the §17 admin UI.
MPL-2.0 already appears in the `deny.toml` `[licenses] allow` list (with a
comment naming these two crates).

§6 encodes the licensing policy as an **allow-list-only** ban: every copyleft
license (GPL, LGPL, AGPL, SSPL, …) is denied by omission — the direct encoding
of §6's constraint that DigiCert's AGPL-3.0 `mtc-bridge` cannot be forked. The
open question this ADR settles: is MPL-2.0 in the same banned copyleft class, or
is it acceptable?

## Decision

**We ratify MPL-2.0 as an accepted license and keep it in the `deny.toml`
allow-list.** MPL-2.0 is **file-scoped weak copyleft**: its reciprocal
obligation attaches only to the MPL-licensed *files themselves* — modifications
to those files must stay MPL — while combining or linking them with code under
any other license (our Apache-2.0 workspace) is unrestricted. This is
categorically different from the **whole-program** copyleft (GPL/AGPL/SSPL) that
§6 bans: MPL-2.0 never reaches out to relicense the combined work.

We consume `cssparser`/`dtoa-short` as unmodified upstream dependencies, so the
reciprocal obligation is never triggered. `ammonia` is the de-facto standard,
actively-maintained Rust HTML sanitizer, and sanitization is security-relevant
surface we do not want to hand-roll.

## Alternatives Considered

### A. Swap `ammonia` for a fully-permissive HTML sanitizer
Rejected. It churns a security-sensitive dependency for zero license benefit,
there is no equally-standard MIT/Apache HTML sanitizer in the Rust ecosystem,
and it would require re-validating the admin UI's XSS-sanitization behavior — a
real risk taken to avoid a license that carries no actual obligation for us.

### B. Vendor and rewrite the MPL-licensed files
Rejected as absurd: rewriting a CSS parser to dodge a file-scoped-copyleft
license we never trip is pure cost.

## Consequences

### Positive
- MPL-2.0 stays allow-listed; `ammonia` (and thus admin-UI sanitization) needs
  no change. `cargo deny check licenses` stays green.
- A clear, grep-able rule for future dependencies: **file-scoped weak copyleft
  (MPL-2.0) is acceptable; whole-program copyleft (GPL/LGPL/AGPL/SSPL) remains
  banned by omission.**

### Negative
- One reciprocal obligation we must honor going forward: **if we ever fork or
  modify an MPL-2.0 file, those modifications must remain MPL-2.0** and be made
  available per the license. This is a review checkpoint, not a present cost.
- Compliance reporting (§20.3) must list MPL-2.0 as an accepted weak-copyleft
  license rather than implying the graph is permissive-only.
