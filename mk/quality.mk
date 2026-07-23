# Code-quality targets (spec §22.12, §22.13, §19.11). Stubs today; implemented
# by the lint-config, license-policy, and testing tickets named below.

.PHONY: fmt lint audit bench

fmt: ## Format all code
	$(call not_implemented,fnd-rust-lint-config)

lint: ## Run all linters
	$(call not_implemented,fnd-rust-lint-config)

audit: ## Run the self-auditor manually
	$(call not_implemented,dev-audit-demo-wiring)

bench: ## Run performance benchmarks
	$(call not_implemented,testing epic (spec §19.11))
