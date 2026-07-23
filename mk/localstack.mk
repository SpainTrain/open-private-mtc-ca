# Local dev environment: LocalStack + SoftHSM2 (spec §18.1).
# Compose file and docs live in deploy/local/; scripts in scripts/.

DEV_ENV_COMPOSE_FILE := deploy/local/docker-compose.yml
DEV_ENV_COMPOSE := docker compose -f $(DEV_ENV_COMPOSE_FILE)

.PHONY: dev-env-up dev-env-down dev-env-reset dev-env-smoke dev-env-logs dev-env-config

dev-env-up: ## Start LocalStack + SoftHSM2 and wait until healthy (bucket/table/token provisioned)
	$(DEV_ENV_COMPOSE) up -d --build --wait

dev-env-down: ## Stop the local dev environment (SoftHSM token state preserved)
	$(DEV_ENV_COMPOSE) down

dev-env-reset: ## Tear down the local dev environment and wipe all state (clean slate)
	./scripts/dev-env-reset.sh

dev-env-smoke: ## Smoke-test the running local dev environment (Object Lock, DDB schema, PKCS#11 token)
	./scripts/dev-env-smoke.sh

dev-env-logs: ## Tail logs from the local dev environment containers
	$(DEV_ENV_COMPOSE) logs -f

dev-env-config: ## Validate and print the resolved local dev compose config
	$(DEV_ENV_COMPOSE) config
