# Agent-harnessing targets (spec §23).
#
# Inner-loop tooling (§23.4) — agent-precheck, watch, verify-task, working-set
# — implemented by ticket agent-inner-loop-targets; recipes delegate to
# scripts/. codemap is implemented by ticket agent-codemap-generator
# (scripts/codemap-gen.sh, from `cargo metadata` — spec §23.6);
# agent-context remains a stub owned by its own ticket. The `journal` target
# lives in mk/journal.mk.

.PHONY: codemap codemap-check codemap-test agent-context agent-precheck verify-task watch working-set agent-inner-loop-test

codemap: ## Regenerate CODEMAP.md from cargo metadata (spec §23.6)
	@scripts/codemap-gen.sh > CODEMAP.md

codemap-check: ## Fail if regenerating CODEMAP.md would change the committed file (spec §23.6)
	@scripts/codemap-check.sh

codemap-test: ## Sandboxed smoke test for the codemap generator against a fixture workspace
	@bash scripts/codemap-smoke-test.sh

agent-context: ## Generate an agent context summary
	$(call not_implemented,agent-context-summary (spec §23.8))

agent-precheck: ## Pre-task gate: tool check, recent journal, lint, fast unit tests (spec §23.4)
	@scripts/agent-precheck.sh

verify-task: ## Pre-done gate: lint, fast test suite, doc/skill/rules lints (spec §23.4)
	@scripts/verify-task.sh

watch: ## Run lint+test on every save via bacon or cargo-watch (spec §23.4; Ctrl-C to stop)
	@scripts/watch.sh

working-set: ## Start ./WORKING_SET.md from docs/templates/WORKING_SET.md (never overwrites)
	@scripts/working-set-init.sh

agent-inner-loop-test: ## Smoke-test the inner-loop tooling (plus shellcheck when installed)
	@scripts/agent-inner-loop-test.sh
