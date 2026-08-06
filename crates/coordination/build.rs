//! Declares the `kani` cfg so the `#[cfg(kani)]` no-panic proof harness
//! (`proofs/lease_epoch.rs`) doesn't trip the `unexpected_cfgs` lint in normal
//! builds. Under `cargo kani`, the `kani` cfg is set and the harness compiles;
//! otherwise it is excluded (mirrors `crates/clock/build.rs` for `loom`).

fn main() {
    println!("cargo::rustc-check-cfg=cfg(kani)");
}
