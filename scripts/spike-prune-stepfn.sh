#!/usr/bin/env bash
# Spike for ADR-0002 (OQ-7, spec §26): prove that AWS Step Functions — the
# chosen pruning-workflow orchestrator (spec §15.2, §5) — runs on the pinned
# zero-cost LocalStack community image (§1 non-goals: no production cloud
# spend; §18.1 local dev environment).
#
# The spike starts an ephemeral LocalStack container (SERVICES=stepfunctions,
# port 4567 — deliberately separate from the mtc-local dev env on 4566, whose
# SERVICES list does not yet include stepfunctions), creates a trivial
# pruning-shaped state machine (Plan -> Choice -> Wait -> Succeed), executes
# it, asserts the execution reaches SUCCEEDED, prints the per-state execution
# history, and tears everything down.
#
# Requires: docker. Uses the awslocal CLI shipped inside the LocalStack image,
# so no host-side AWS tooling is needed.
#
# Usage: ./scripts/spike-prune-stepfn.sh
#   SPIKE_KEEP=1 ./scripts/spike-prune-stepfn.sh   # keep the container for inspection
set -euo pipefail

# Keep in sync with deploy/local/docker-compose.yml (pinned community major;
# the 2026 CalVer images require a LocalStack auth token — see the comment
# there).
LOCALSTACK_IMAGE="${LOCALSTACK_IMAGE:-localstack/localstack:4.14.0}"
CONTAINER="mtc-spike-prune-stepfn"
PORT="${SPIKE_PORT:-4567}"
SM_NAME="prune-spike"
# LocalStack does not enforce IAM; any well-formed role ARN is accepted.
ROLE_ARN="arn:aws:iam::000000000000:role/${SM_NAME}"

GREEN=$'\033[32m'
RED=$'\033[31m'
RESET=$'\033[0m'

fail() {
  printf '%s✗ FAIL%s %s\n' "${RED}" "${RESET}" "$1" >&2
  exit 1
}

pass() {
  printf '%s✓ PASS%s %s\n' "${GREEN}" "${RESET}" "$1"
}

cleanup() {
  if [[ "${SPIKE_KEEP:-0}" == "1" ]]; then
    echo "SPIKE_KEEP=1: leaving container '${CONTAINER}' running on port ${PORT}"
    echo "inspect with: docker exec ${CONTAINER} awslocal stepfunctions list-executions --state-machine-arn <arn>"
  else
    docker stop "${CONTAINER}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

awsl() { docker exec "${CONTAINER}" awslocal "$@"; }

echo "== OQ-7 spike: Step Functions on LocalStack community =="

# --- Bring up an ephemeral LocalStack with only stepfunctions enabled --------
docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true
docker run -d --rm --name "${CONTAINER}" \
  -e SERVICES=stepfunctions \
  -e SKIP_SSL_CERT_DOWNLOAD=1 \
  -e DISABLE_EVENTS=1 \
  -p "127.0.0.1:${PORT}:4566" \
  "${LOCALSTACK_IMAGE}" >/dev/null

for _ in $(seq 1 45); do
  health="$(docker exec "${CONTAINER}" curl -sf http://localhost:4566/_localstack/health 2>/dev/null || true)"
  if grep -q '"stepfunctions": "\(available\|running\)"' <<<"${health}"; then
    break
  fi
  sleep 2
done
grep -q '"stepfunctions": "\(available\|running\)"' <<<"${health:-}" \
  || fail "LocalStack (${LOCALSTACK_IMAGE}) did not report stepfunctions healthy"
grep -q '"edition": "community"' <<<"${health}" \
  || fail "expected the community edition (zero-cost constraint, §1 non-goals)"
pass "LocalStack community edition up with stepfunctions enabled"

# --- Create a trivial pruning-shaped state machine ---------------------------
# Mirrors the §15.2 control flow the real workflow (prune-stepfn-workflow)
# will implement — plan, prunable? choice, replication wait, terminal states —
# with Pass states instead of Lambda tasks: this spike proves the orchestrator
# runs locally, not the workers.
DEFINITION='{
  "Comment": "OQ-7 spike: pruning-shaped control flow, no side effects (ADR-0002)",
  "StartAt": "PlanPrunableRange",
  "States": {
    "PlanPrunableRange": {
      "Type": "Pass",
      "Result": { "prunable": true, "range": { "start": 0, "end": 256 }, "tree_size": 1024 },
      "ResultPath": "$.plan",
      "Next": "AnythingPrunable"
    },
    "AnythingPrunable": {
      "Type": "Choice",
      "Choices": [
        { "Variable": "$.plan.prunable", "BooleanEquals": true, "Next": "AwaitReplication" }
      ],
      "Default": "NothingToPrune"
    },
    "AwaitReplication": { "Type": "Wait", "Seconds": 1, "Next": "PruneComplete" },
    "NothingToPrune": { "Type": "Succeed" },
    "PruneComplete": { "Type": "Succeed" }
  }
}'

SM_ARN="$(awsl stepfunctions create-state-machine \
  --name "${SM_NAME}" \
  --role-arn "${ROLE_ARN}" \
  --definition "${DEFINITION}" \
  --query stateMachineArn --output text)" \
  || fail "create-state-machine failed"
pass "state machine created: ${SM_ARN}"

# --- Execute and assert SUCCEEDED --------------------------------------------
EXEC_ARN="$(awsl stepfunctions start-execution \
  --state-machine-arn "${SM_ARN}" \
  --input '{"trigger":"spike"}' \
  --query executionArn --output text)" \
  || fail "start-execution failed"

status="RUNNING"
for _ in $(seq 1 30); do
  status="$(awsl stepfunctions describe-execution \
    --execution-arn "${EXEC_ARN}" --query status --output text)"
  [[ "${status}" == "RUNNING" ]] || break
  sleep 1
done
[[ "${status}" == "SUCCEEDED" ]] \
  || fail "execution ended ${status} (expected SUCCEEDED): ${EXEC_ARN}"
pass "execution SUCCEEDED: ${EXEC_ARN}"

# --- Show the per-state history (the operational-visibility claim) -----------
echo "-- execution history (event types) --"
awsl stepfunctions get-execution-history \
  --execution-arn "${EXEC_ARN}" --query 'events[].type' --output text

echo
printf '%sSpike green:%s Step Functions executes locally on the pinned community image.\n' \
  "${GREEN}" "${RESET}"
