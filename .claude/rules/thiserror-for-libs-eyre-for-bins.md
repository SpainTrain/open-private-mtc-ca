# thiserror-for-libs-eyre-for-bins

> Spec: §22.6 (Result types are the language); §23.2 (error type discipline).

## Rule

Error type discipline: library crates define typed error enums with
`thiserror`; binary crates use `eyre` + `color-eyre` at the top level.

## Rationale

Libraries are APIs: their errors are part of the contract (§22.6 — failure
modes live in the signature as enums like `AllocateError`, and callers must
handle each variant). `thiserror` keeps those enums ergonomic without hiding
them. Binaries are the end of the call chain: nothing above them matches on
variants, so `eyre` + `color-eyre` is used for the DX benefit of colored stack
traces and precise panic locations (§23.2) — faster diagnosis for both humans
and agents reading CI output. Using `eyre` inside a library would erase the
typed contract; hand-rolling `Display`/`Error` impls in every lib is noise
`thiserror` exists to remove.

## Compliant example

```rust
// crates/storage/src/error.rs (library)
#[derive(Debug, thiserror::Error)]
pub enum AllocateError {
    #[error("lease lost during allocation")]
    LostLease,
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
}

// crates/mtcctl/src/main.rs (binary)
fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    run()?; // library errors convert into the eyre report with context
    Ok(())
}
```

## Non-compliant example

```rust
// crates/storage/src/lib.rs (library)
pub fn allocate(n: usize) -> eyre::Result<(Index, Index)> {
    // opaque report type: callers can no longer match on LostLease
}
```

## Enforcement

- **Review**: library public APIs returning `eyre::Result`/`anyhow::Result`
  (or `Box<dyn Error>`) are rejected; binaries hand-rolling error enums for
  `main` are pushed to `eyre`.
- **Lint**: `cargo deny` dependency hygiene (§22.13) keeps the error-crate set
  intentional.
