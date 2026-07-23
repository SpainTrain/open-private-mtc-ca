# Example fragment. This is the template every other fragment follows: declare
# your targets .PHONY, give each a `## help text` comment, and either implement
# the recipe (delegating to scripts/<name>.sh or cargo) or leave it as a
# not-implemented stub via $(call not_implemented,<slug>). Copy this file to
# mk/<your-topic>.mk to add targets — never edit the root Makefile.

.PHONY: hello
hello: ## Print a friendly greeting (example fragment)
	@echo "hello from mtc-ca"
