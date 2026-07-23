# mk/adr.mk — ADR (Architecture Decision Record) tooling, spec §23.5–23.6.
# Included by the root Makefile via `include mk/*.mk`.

.PHONY: adr adr-test

adr: ## Scaffold the next-numbered ADR and index row: make adr title="My decision"
	@scripts/adr-new.sh "$(title)"

adr-test: ## Run the ADR tooling smoke tests (sandboxed; touches no real ADRs)
	@bash scripts/adr-smoke-test.sh
