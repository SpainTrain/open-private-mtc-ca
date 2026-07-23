# no-unwrap-in-prod

> Spec: §22.6 (Result types are the language); enforcement via §22.12 (Linting setup).

## Rule

`unwrap()` and `expect()` are forbidden outside tests. Use `?` and proper error
types instead.

## Rationale

§22.6: the compiler enforces that callers handle errors via `?`, `match`, or
explicit `unwrap()` — and `unwrap()` is deliberately grep-able and lint-able so
it can be banned in production code. A CA service that panics on an unexpected
storage or crypto error loses its lease mid-batch and turns a recoverable error
into an availability incident. Typed error enums (§22.6) keep failure modes in
the function signature, where callers must handle them; `unwrap()` erases them.

## Compliant example

```rust
pub fn parse_batch_id(raw: &str) -> Result<BatchId, ParseError> {
    let id = validate(raw)?; // propagate with `?`
    Ok(BatchId::new(id))
}
```

In tests, `unwrap()`/`expect()` remain fine:

```rust
#[test]
fn parses_valid_id() {
    let id = parse_batch_id("batch-7").unwrap(); // allowed in #[cfg(test)]
    assert_eq!(id.to_string(), "batch-7");
}
```

## Non-compliant example

```rust
pub fn parse_batch_id(raw: &str) -> BatchId {
    BatchId::new(validate(raw).unwrap()) // panics in production on bad input
}
```

## Enforcement

- **Lint**: `clippy::unwrap_used` and `clippy::expect_used` are denied in
  non-test code (§22.12).
- **CI gate**: `cargo clippy --all-targets --all-features -- -D warnings` on
  every PR (§22.13).
- **Review**: error types must be proper enums (§22.6), not stringly-typed
  escapes.
