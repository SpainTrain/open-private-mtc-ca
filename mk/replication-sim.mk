# dev-replicator: local S3 CRR + DynamoDB Global Tables replication simulator
# (ticket dev-crr-replication-sim, spec §18.3). One `dev-replicator` process
# instance is one directed link between two LocalStack endpoints; these
# targets stand up a second LocalStack instance for a two-region link and
# drive/inspect a running replicator over its control endpoint.
#
# NOT the three-region `make demo-multiregion` topology — that is
# `dev-multiregion-harness`'s compose profile, which layers its own per-region
# CA-service services the same way `docker-compose.replication-sim.yml` layers
# `localstack-b` here. This file only proves the replicator substrate works.

REPL_SIM_COMPOSE := docker compose \
	-f deploy/local/docker-compose.yml \
	-f deploy/local/docker-compose.replication-sim.yml

.PHONY: replication-sim-up replication-sim-down replication-sim-run replication-sim-test

replication-sim-up: ## Bring up two LocalStack instances for the replication simulator (region A: 4566, region B: 4567)
	$(REPL_SIM_COMPOSE) up -d --wait

replication-sim-down: ## Tear down the two-instance replication simulator environment
	$(REPL_SIM_COMPOSE) down --volumes --remove-orphans

replication-sim-run: ## Run dev-replicator A->B with lag=5s (ctrl-C to stop; requires replication-sim-up)
	REPL_LINK_NAME=region-a-to-region-b \
	REPL_SOURCE_ENDPOINT_URL=http://127.0.0.1:4566 \
	REPL_TARGET_ENDPOINT_URL=http://127.0.0.1:4567 \
	REPL_S3_BUCKET=mtc-log-local \
	REPL_DDB_TABLE=mtc-log-coordination \
	REPL_LAG_MS=5000 \
	cargo run -p dev-replicator

replication-sim-test: ## Run the dev-replicator integration suite against a two-instance environment (brings up/tears down)
	@tests/e2e/replication-sim-demo.sh
