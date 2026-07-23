# mk/runbooks.mk — runbook documentation tooling (spec §21.2).
# Included by the root Makefile via mk/*.mk.

RUNBOOKS_DIR ?= docs/runbooks

.PHONY: lint-runbooks
lint-runbooks: ## Lint docs/runbooks/ structure (five §21.2 sections, postmortem template, index completeness)
	scripts/lint-runbooks.sh $(RUNBOOKS_DIR)

.PHONY: test-lint-runbooks
test-lint-runbooks: ## Run pass/fail fixture tests for the runbook structure lint
	scripts/test-lint-runbooks.sh
