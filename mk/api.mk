# API codegen target (spec §17.2). Stub today; implemented by the OpenAPI
# codegen pipeline ticket.

.PHONY: api-gen

api-gen: ## Regenerate API code from the OpenAPI spec
	$(call not_implemented,openapi-codegen-pipeline)
