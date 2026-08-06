# cloud-aws: S3ObjectStore / S3ObjectLock over aws-sdk-s3, exercised against
# LocalStack (ticket aws-backend, spec §9.3). Reuses the same LocalStack
# container as `make dev-env-up` (deploy/local/docker-compose.yml); this
# crate's own tests/support/mod.rs provisions a dedicated, freshly-named test
# bucket per run inside it -- see crates/cloud-aws/src/lib.rs.

.PHONY: cloud-aws-integration-test

cloud-aws-integration-test: ## Run cloud-aws's LocalStack-backed conformance suites (brings up LocalStack if needed)
	docker compose -f deploy/local/docker-compose.yml up -d --wait localstack
	cargo test -p cloud-aws --features integration -- --test-threads=1
