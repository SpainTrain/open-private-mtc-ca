# Code-quality targets (spec §22.12, §22.13, §19.11). fmt/lint/cargo-deny/
# cargo-audit are live (fnd-rust-lint-config, fnd-license-policy); the rest
# are stubs implemented by the tickets named below. Lint levels: Cargo.toml
# [workspace.lints.*]; knobs: clippy.toml / rustfmt.toml; deviation policy:
# docs/lint-policy.md. License/supply-chain policy: deny.toml,
# .cargo/audit.toml; rationale: docs/license-policy.md.
#
# `audit` (below) is the self-auditor stub (spec §20.2, a different concept
# entirely: independent re-verification of the published log) — `cargo-audit`
# is deliberately a distinct target name so the two are never confused.

.PHONY: fmt fmt-check lint audit bench cargo-deny cargo-audit

fmt: ## Format all code (rustfmt, workspace-wide)
	cargo fmt --all

fmt-check: ## Check formatting without rewriting (the CI gate, spec §22.13)
	cargo fmt --all --check

lint: fmt-check ## Run all linters: rustfmt check + clippy -D warnings (spec §22.12)
	cargo clippy --workspace --all-targets --all-features -- -D warnings

audit: ## Run the self-auditor manually
	$(call not_implemented,dev-audit-demo-wiring)

bench: ## Run performance benchmarks
	$(call not_implemented,testing epic (spec §19.11))

cargo-deny: ## License allow-list + advisory + duplicate-dep + source checks (spec §22.13; deny.toml)
	cargo deny check

cargo-audit: ## RustSec security-advisory scan (spec §22.13; .cargo/audit.toml)
	cargo audit
