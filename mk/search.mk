# Agent navigation: pre-built search affordances (spec §23.6).
#
# Ripgrep-based targets for locating tests, callers, implementations, and TODOs
# without burning tokens reading files. All targets emit file:line-prefixed
# output agents can consume directly. Implemented by the agent-search-affordances
# ticket.

.PHONY: find-tests find-callers find-impl find-todo

find-tests: ## Find tests related to a path (usage: make find-tests path=X)
	@scripts/find-tests.sh

find-callers: ## Find call sites for a symbol (usage: make find-callers symbol=Y)
	@scripts/find-callers.sh

find-impl: ## Find implementations of a trait (usage: make find-impl iface=Z)
	@scripts/find-impl.sh

find-todo: ## Find all TODO/FIXME comments with file:line
	@scripts/find-todo.sh
