# Demo and local-runtime targets (spec §18.1, §18.3, §18.4, §18.7).
# Stubs today; implemented by the local-dev-experience epic tickets named below.

.PHONY: demo demo-down demo-multiregion demo-multiregion-down dev partition-region time-advance

demo: ## Bring up the single-region 60-second demo
	$(call not_implemented,dev-demo-single-region)

demo-down: ## Tear down the single-region demo
	$(call not_implemented,dev-demo-single-region)

demo-multiregion: ## Bring up the three-region simulated environment
	$(call not_implemented,dev-multiregion-harness)

demo-multiregion-down: ## Tear down the three-region environment
	$(call not_implemented,dev-multiregion-harness)

dev: ## Run the CA service with hot reload (cargo-watch)
	$(call not_implemented,dev-hot-reload)

partition-region: ## Simulate a network partition of a region (region=X)
	$(call not_implemented,dev-partition-failover-scenarios)

time-advance: ## Advance the simulated clock (days=N)
	$(call not_implemented,dev-time-advance)
