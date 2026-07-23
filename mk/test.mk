# Test-suite targets (spec §19). Stubs today; implemented by the testing epic.

.PHONY: test test-unit test-prop test-conformance test-chaos test-soak test-e2e

test: ## Run all tests
	$(call not_implemented,testing epic (spec §19))

test-unit: ## Run unit tests only
	$(call not_implemented,testing epic (spec §19.1))

test-prop: ## Run property-based tests with extended runs
	$(call not_implemented,testing epic (spec §19.2))

test-conformance: ## Run the spec conformance suite
	$(call not_implemented,testing epic (spec §19.4))

test-chaos: ## Run chaos-engineering scenarios
	$(call not_implemented,testing epic (spec §19.9))

test-soak: ## Run the long-running soak test
	$(call not_implemented,testing epic (spec §19.7))

test-e2e: ## Run end-to-end tests via the CLI
	$(call not_implemented,testing epic (spec §19.13))
