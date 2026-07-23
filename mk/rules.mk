# Agent-harness rule linting (ticket agent-claude-rules-seed).

.PHONY: rules-lint

## rules-lint: check every .claude/rules/ file has the required sections
rules-lint:
	@sh .claude/rules/lint-rules.sh
