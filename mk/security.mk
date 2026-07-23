# Security review checklist tooling (ticket mtc-ve8j, spec §24 Phase 8).
# Included by the root Makefile via `include mk/*.mk`.

SECURITY_CHECKLIST := docs/security/review-checklist.md

.PHONY: lint-security-checklist
lint-security-checklist: ## Lint the security review checklist (every item has a valid status)
	scripts/lint-security-checklist.sh $(SECURITY_CHECKLIST)

.PHONY: test-security-checklist-lint
test-security-checklist-lint: ## Unit-test the checklist lint script against fixtures
	scripts/test-lint-security-checklist.sh
