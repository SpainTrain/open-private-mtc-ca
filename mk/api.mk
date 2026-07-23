# API codegen targets (spec §17.2, §17.5). `api-gen` is the one regeneration
# entrypoint: it lints api/admin.openapi.yaml and regenerates the axum
# server-stub crate (crates/admin-api-server) and the Rust client crate
# (crates/admin-api-client). Generated code is committed; rerunning must be a
# no-op unless the spec changed. The TypeScript client + HTML docs pipeline is
# a separate ticket (fnd-openapi-ts-client-docs).

.PHONY: api-gen api-lint

api-gen: ## Regenerate API code from the OpenAPI spec
	@scripts/api-gen.sh

api-lint: ## Lint api/admin.openapi.yaml without regenerating
	@scripts/api-gen.sh --lint-only
