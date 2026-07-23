# use-newtypes

> Spec: §22.1 (Newtypes for domain identifiers); see also §22.5 (Phantom types).

## Rule

Domain identifiers are newtypes, never type aliases. `Index`, `TreeSize`,
`Epoch`, `LogId`, `BatchId`, and every future domain identifier get their own
wrapper struct.

## Rationale

§22.1: with newtypes, the compiler will refuse to pass an `Epoch` where a
`TreeSize` is expected — no agent discipline required; the language enforces
it. A type alias (`type Epoch = u64;`) provides zero protection: every `u64`
converts implicitly, so swapping an epoch for a tree size in an allocation call
compiles and silently corrupts the lease/epoch protocol. Where many IDs share a
representation, phantom-typed IDs (§22.5) give distinct compile-time types at
zero runtime cost.

## Compliant example

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Epoch(pub u64);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct TreeSize(pub u64);

fn allocate(n: usize, epoch: Epoch) { /* cannot receive a TreeSize */ }
```

## Non-compliant example

```rust
pub type Epoch = u64;    // alias: any u64 is accepted
pub type TreeSize = u64; // alias: interchangeable with Epoch

fn allocate(n: usize, epoch: Epoch) { /* a TreeSize compiles here too */ }
```

## Enforcement

- **Review**: PRs introducing a domain identifier as a bare primitive or
  `type` alias are rejected.
- **Lint**: `clippy::pedantic` (§22.12) flags several primitive-obsession
  patterns; the compiler enforces distinctness once newtypes exist.
