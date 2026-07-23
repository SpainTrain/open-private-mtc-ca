# Interactive REPL and fixture targets (spec §18.5, §18.6). Stubs today;
# implemented by the local-dev-experience epic tickets named below.

.PHONY: repl fixture-load fixture-save

repl: ## Launch the interactive Rust REPL (evcxr)
	$(call not_implemented,dev-repl-evcxr)

fixture-load: ## Load a named fixture (name=X)
	$(call not_implemented,dev-fixture-targets)

fixture-save: ## Snapshot current state as a named fixture (name=X)
	$(call not_implemented,dev-fixture-targets)
