# mk/journal.mk — decision journal targets (docs/journal.md, spec §23.5/§23.7).
# Included by the root Makefile via mk/*.mk.
#
# `msg` (and optional `ticket`/`pr`) reach scripts/journal-append.sh through
# the environment — GNU make auto-exports command-line variables — so quotes
# and multiline text survive intact. Make-level caveat: make expands `$`, so
# write literal dollar signs as `$$`, or call scripts/journal-append.sh
# directly for text make would mangle.

.PHONY: journal journal-test

journal: ## Append a timestamped entry to docs/journal.md (required: msg="..."; optional: ticket=..., pr=...)
	@scripts/journal-append.sh

journal-test: ## Run smoke tests for the journal append tooling (plus shellcheck when installed)
	@scripts/journal-append-test.sh
