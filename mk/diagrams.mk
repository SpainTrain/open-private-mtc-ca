# Living architecture diagrams (docs/architecture/) — see spec §23.6.
# Included by the root Makefile via mk/*.mk.

.PHONY: diagrams-check diagrams-check-selftest

## diagrams-check: validate Mermaid syntax in all docs/architecture pages.
## Offline after the first run (tooling installs once into scripts/diagrams-lint).
diagrams-check:
	@bash scripts/check-diagrams.sh

## diagrams-check-selftest: E2E smoke (§19.13 spirit) — the checker must PASS on
## the seeded diagrams and FAIL on a known-broken fixture.
diagrams-check-selftest: diagrams-check
	@echo "--- expecting failure on broken fixture ---"
	@if bash scripts/check-diagrams.sh scripts/diagrams-lint/fixtures/broken-mermaid.md; then \
		echo "SELFTEST FAILED: checker did not reject broken fixture" >&2; \
		exit 1; \
	else \
		echo "SELFTEST OK: checker rejected broken fixture"; \
	fi
