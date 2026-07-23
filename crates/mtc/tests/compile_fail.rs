//! Compile-fail proofs for the newtype guarantees (spec sections 22.1 and
//! 22.5; spec 19.1 unit layer).
//!
//! Each case under `tests/compile_fail/` is a program that must be rejected
//! by the compiler; the checked-in `.stderr` files pin the diagnostics under
//! the workspace's pinned toolchain (`rust-toolchain.toml`).

#[test]
fn newtypes_are_distinct_compile_time_types() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/*.rs");
}
