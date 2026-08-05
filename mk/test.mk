# Test-suite targets (spec §19). Stubs today; implemented by the testing epic.

.PHONY: test test-unit test-prop test-conformance conformance test-chaos test-soak test-e2e

test: ## Run all tests
	$(call not_implemented,testing epic (spec §19))

test-unit: ## Run unit tests only
	$(call not_implemented,testing epic (spec §19.1))

test-prop: ## Run property-based tests with extended runs
	$(call not_implemented,testing epic (spec §19.2))

# test-conformance-runner ticket (spec §19.4). --nocapture: the suite's value
# is the per-vector PASS/FAIL lines and the trailing pass/fail/total summary
# (ticket demo), which `cargo test` would otherwise swallow on a fully green
# run. This is plain `cargo test -p mtc-conformance`, so it also runs
# unmodified under the workspace-wide `cargo test --all-features` required CI
# check (spec §22.13) with no separate CI wiring — see
# crates/conformance/src/lib.rs and conformance/README.md.
test-conformance: ## Run the spec conformance suite
	cargo test -p mtc-conformance --all-features -- --nocapture

conformance: test-conformance ## Alias for test-conformance (spec §19.4 ticket demo command)

test-chaos: ## Run chaos-engineering scenarios
	$(call not_implemented,testing epic (spec §19.9))

test-soak: ## Run the long-running soak test
	$(call not_implemented,testing epic (spec §19.7))

test-e2e: ## Run end-to-end tests via the CLI
	$(call not_implemented,testing epic (spec §19.13))
