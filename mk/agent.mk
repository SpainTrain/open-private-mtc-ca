# Agent-harnessing targets (spec §23).
#
# Inner-loop tooling (§23.4) — agent-precheck, watch, verify-task, working-set
# — implemented by ticket agent-inner-loop-targets; recipes delegate to
# scripts/. codemap and agent-context remain stubs owned by their own tickets.
# The `journal` target lives in mk/journal.mk.

.PHONY: codemap agent-context agent-precheck verify-task watch working-set agent-inner-loop-test

codemap: ## Generate the repo code map
	$(call not_implemented,agent-harnessing epic (spec §23.6))

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
