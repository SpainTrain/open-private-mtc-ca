# Code-quality targets (spec §22.12, §22.13, §19.11). fmt/lint are live
# (fnd-rust-lint-config); the rest are stubs implemented by the tickets named
# below. Lint levels: Cargo.toml [workspace.lints.*]; knobs: clippy.toml /
# rustfmt.toml; deviation policy: docs/lint-policy.md.

.PHONY: fmt fmt-check lint audit bench

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
