# no-systemtime-now-in-prod

> Spec: §22.11 (The `Clock` trait); enforcement via §22.12 (Linting setup).

## Rule

`SystemTime::now()` is forbidden outside test modules. Production code accesses
time only through an injected `Clock` (as `Arc<dyn Clock>`).

## Rationale

§22.11: production code accesses time only through an injected `Arc<dyn Clock>`;
tests inject a `FakeClock` that supports time advancement. Direct calls to
`SystemTime::now()` make time-dependent behavior (lease expiry, checkpoint
signing timestamps, batch windows) untestable and non-deterministic. The
injected-clock pattern lets tests advance time deterministically and lets the
lease/epoch protocol be exercised — and formally verified — without wall-clock
sleeps.

## Compliant example

```rust
pub struct LeaseRenewer {
    clock: Arc<dyn Clock>,
}

impl LeaseRenewer {
    pub fn is_expired(&self, lease: &Lease) -> bool {
        self.clock.now() >= lease.expires_at
    }
}
```

## Non-compliant example

```rust
impl LeaseRenewer {
    pub fn is_expired(&self, lease: &Lease) -> bool {
        SystemTime::now() >= lease.expires_at // forbidden outside tests
    }
}
```

## Enforcement

- **Lint**: custom `dylint` lint denies any direct call to `SystemTime::now()`
  outside test code (§22.12).
- **CI gate**: lint runs under `-D warnings` as part of the required checks
  (§22.13).
- **Review**: any new time-dependent code path must take a `Clock`.
