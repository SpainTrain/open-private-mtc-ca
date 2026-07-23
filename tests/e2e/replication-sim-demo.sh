#!/usr/bin/env bash
# E2E demo/smoke test for dev-replicator (ticket dev-crr-replication-sim,
# spec §18.3). Brings up two LocalStack instances, runs the crate's
# `#[ignore]`-gated integration suite against them (real S3/DynamoDB traffic,
# FakeClock-driven lag so it stays fast), and tears down.
#
# This is both the ticket's literal "Demo" (§18.3: "Start two LocalStacks +
# replicator with 5s lag; aws s3 cp to A; watch the object appear in B five
# seconds later" — exercised precisely by
# `s3_object_appears_in_target_only_after_lag_elapses`) and its Testing AC
# ("Integration: two LocalStack containers ... pause halts propagation").
#
# Usage: tests/e2e/replication-sim-demo.sh   (from anywhere; paths are self-locating)
#     or: make replication-sim-test
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
cd "${REPO_ROOT}"

# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

COMPOSE=(docker compose -f deploy/local/docker-compose.yml -f deploy/local/docker-compose.replication-sim.yml)

cleanup() {
  echo "== tearing down replication-sim environment =="
  "${COMPOSE[@]}" down --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "== bringing up two LocalStack instances (region A: 4566, region B: 4567) =="
"${COMPOSE[@]}" up -d --wait

echo "== running dev-replicator integration suite =="
cargo test -p dev-replicator --test integration -- --ignored --test-threads=1

echo
echo "replication-sim-demo: PASS"
