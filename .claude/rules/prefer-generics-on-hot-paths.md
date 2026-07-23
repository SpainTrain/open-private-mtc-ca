# prefer-generics-on-hot-paths

> Spec: §22.7 (Static vs dynamic dispatch — a deliberate boundary).

## Rule

Use generics with trait bounds (`<T: Trait>` / `impl Trait`) on hot paths. Use
`Arc<dyn Trait>` only at architectural seams (runtime-swappable backends,
injected test dependencies) or for heterogeneous collections. Default for
ambiguous cases: lean toward generics.

## Rationale

§22.7: overuse of `Arc<dyn Trait>` introduces vtable lookups and heap
allocation on every call — fine at the top of the call graph (initialized once
at startup), painful on hot paths such as per-entry tree hashing, per-byte
serialization, and per-tile cache lookups. Overuse of generics, conversely,
explodes compile times and complicates heterogeneous storage. The line is
deliberate: `Arc<dyn Trait>` for the four cloud abstractions, injected deps
(`Clock`, `MetricsSink`), heterogeneous collections (`Vec<Arc<dyn Adapter>>`),
and architectural seams; generics where the implementation is known at compile
time and calls are frequent. Switching generic → `Arc<dyn>` later is
mechanical; the reverse can require type-system refactoring — hence the
default toward generics.

## Compliant example

```rust
// Hot path: per-leaf and per-node hashing, monomorphized to SHA-256 (§22.7)
pub struct TreeUpdater<H: Hasher> {
    hasher: H,
}

// Architectural seam: swappable backend chosen at startup
pub struct Backend {
    kv: Arc<dyn ReplicatedKv>,
    clock: Arc<dyn Clock>,
}
```

## Non-compliant example

```rust
// Dynamic dispatch inside the per-entry hashing loop
pub struct TreeUpdater {
    hasher: Arc<dyn Hasher>, // vtable hit per leaf; should be <H: Hasher>
}
```

## Enforcement

- **Review**: dispatch choices are checked against the §22.7 decision table
  (Backend fields, tree hashing, serialization, `Clock`, adapter registry,
  per-entry codec); deviations need justification in the PR.
- **CI gate**: performance regression tests (§19.11) catch dispatch-cost
  creep on hot paths.
