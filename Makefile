# Makefile — repo automation targets.
# Agent-harness targets from docs/mtc-architecture-spec.md land here as their
# tickets complete (agent-precheck, codemap, verify-task, journal, ...).

.PHONY: rules-lint

## rules-lint: check every .claude/rules/ file has the required sections
rules-lint:
	@sh .claude/rules/lint-rules.sh
