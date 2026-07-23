# kani-for-critical-paths

> Spec: §19.12 (Formal verification); §22.13 (Required CI checks).

## Rule

Code touching the lease/epoch protocol or the write-path linearization point
must include or update Kani harnesses in the same PR.

## Rationale

§19.12: Kani is the primary formal-verification tool, applied to the
lease/epoch protocol implementation (no two regions hold a current-epoch lease
simultaneously), the write-path linearization point (atomic transition between
pre-commit and committed states), and Merkle tree append/proof primitives.
These are the paths where a bug is catastrophic — overlapping index ranges or a
torn commit corrupts the log irreparably — and where traditional testing cannot
enumerate the interleavings. Machine-checked proofs are also part of the trust
story shown to compliance auditors. A change to a verified path that leaves its
harness stale either breaks the proof (CI red) or, worse, narrows it silently.

## Compliant example

```text
PR: "Add epoch check to index allocation"
- crates/storage/src/allocate.rs           # the change
- crates/storage/proofs/allocate.rs        # updated Kani harness, e.g.
  #[kani::proof] fn verify_no_overlapping_allocations() { ... }
- CI: `cargo kani` green
```

## Non-compliant example

```text
PR: "Simplify lease renewal state machine"
- Rewrites the epoch transition logic
- proofs/ untouched; the existing harness still proves the *old* transition
  relation and no longer covers the new code paths
```

## Enforcement

- **CI gate**: for PRs touching critical paths, `cargo kani` runs and the
  proofs must pass (§22.13).
- **Review**: diffs under lease/epoch or write-path code without a
  corresponding `proofs/` change require an explicit statement of why the
  existing harnesses still cover the new behavior.
