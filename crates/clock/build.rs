//! Declares the `loom` cfg so `--cfg loom` builds (concurrency model tests)
//! don't trip the `unexpected_cfgs` lint in normal builds.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(loom)");
}
