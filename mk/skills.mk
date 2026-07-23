# mk/skills.mk — agent skill tooling (spec section 23.1).
# Included by the root Makefile via `include mk/*.mk`.

.PHONY: skill-lint skill-lint-smoke

skill-lint: ## Validate .claude/skills/ files: required sections present, "Files involved" paths exist
	@scripts/skill-lint.sh

skill-lint-smoke: ## Smoke-test skill-lint itself: passes on template, fails on broken fixtures; shellcheck
	@scripts/skill-lint-smoke.sh
