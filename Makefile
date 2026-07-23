# MTC-CA developer entry point.
#
# This root Makefile is intentionally generic. It provides `help`, a shared
# helper for not-yet-implemented targets, and includes every fragment in mk/.
#
# DO NOT add targets to this file. Add a target by creating a new
# mk/<name>.mk fragment (or extending a topic fragment you own). This keeps
# parallel work from colliding on one file — the same rationale as the crate
# glob in Cargo.toml. See CONTRIBUTING.md and docs/dev-environment.md.
#
# Every target MUST carry a `## help text` comment on its rule line so
# `make help` can harvest it into the self-documenting catalog.

.DEFAULT_GOAL := help

# All targets live in fragments. New targets go in mk/*.mk, never here.
include mk/*.mk

# Shared helper for targets that are declared but not yet implemented. Prints a
# friendly pointer to the owning beads work and exits non-zero. Later tickets
# replace a stub recipe with a real implementation (delegating to scripts/<name>.sh
# or cargo) in the target's fragment.
#   Usage inside a recipe:  $(call not_implemented,<beads-slug-or-epic>)
define not_implemented
	@printf '\033[33m%s\033[0m is not implemented yet — tracked in beads (%s)\n' '$@' '$(1)' >&2
	@exit 1
endef

.PHONY: help
help: ## Show this help (default target)
	@printf 'MTC-CA developer targets:\n\n'
	@grep -hE '^[a-zA-Z0-9_.-]+:.*?## ' $(MAKEFILE_LIST) \
		| sort \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-26s\033[0m %s\n", $$1, $$2}'
	@printf '\n'
	@printf 'Parameterized targets take name=value args, e.g.:\n'
	@printf '  make fixture-load name=demo    make time-advance days=400    make journal msg="note"\n'
