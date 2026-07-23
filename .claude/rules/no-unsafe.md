# no-unsafe

> Spec: §22.12 (Linting setup: `unsafe_code = "forbid"`).

## Rule

`unsafe` blocks are forbidden everywhere except documented FFI boundaries
(PKCS#11). Any FFI exception must be explicit, minimal, and documented.

## Rationale

§22.12 sets `unsafe_code = "forbid"` — no unsafe blocks anywhere, with explicit
exceptions only for FFI to PKCS#11 (the HSM boundary, which requires calling a
C shared library). Rust's ownership model is one of the project's core
guardrails (§6, §23): it eliminates entire bug classes only as long as code
stays in safe Rust. Every `unsafe` block reopens exactly the memory-safety and
concurrency hazards the language was chosen to exclude, in a codebase where a
single corruption on the write path can produce an inconsistent Merkle tree.

## Compliant example

```rust
// crates/hsm-pkcs11/src/ffi.rs — the documented PKCS#11 FFI boundary.
// SAFETY: `C_Sign` is called with a session handle validated above and
// buffers whose lengths are passed alongside their pointers, per PKCS#11 v2.40.
#![allow(unsafe_code)] // explicit, scoped exception for the FFI module

let rv = unsafe { (*fn_list).C_Sign.unwrap()(session, data.as_ptr(), ...) };
```

## Non-compliant example

```rust
// crates/tree/src/hash.rs — domain code, not an FFI boundary
let root: [u8; 32] = unsafe { std::mem::transmute(buf) }; // forbidden
```

Using `unsafe` for convenience, performance guesses, or to silence the borrow
checker in domain code is never acceptable.

## Enforcement

- **Lint**: `unsafe_code = "forbid"` at the workspace level (§22.12); the FFI
  crate carries the only scoped `allow`, with a `SAFETY:` comment per block.
- **CI gate**: `cargo clippy ... -- -D warnings` fails on undeclared unsafe
  (§22.13).
- **Review**: any change to the FFI exception surface requires explicit review
  of the safety comments.
