#!/usr/bin/env bash
# Smoke test for the LocalStack + SoftHSM2 local dev environment (spec §19.1).
# Asserts the §8 data model and §14 HSM provisioning are actually in place:
#   - LocalStack healthy
#   - log bucket: Object Lock enabled + versioning enabled (§8.1)
#   - coordination table: PK/SK schema, ACTIVE (§8.2)
#   - SoftHSM2: token visible via pkcs11-tool, ECDSA P-256 key present (§14.1)
#
# Usage: ./scripts/dev-env-smoke.sh   (from anywhere; paths are self-locating)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
COMPOSE=(docker compose -f "${REPO_ROOT}/deploy/local/docker-compose.yml")

BUCKET="${MTC_LOG_BUCKET:-mtc-log-local}"
TABLE="${MTC_COORDINATION_TABLE:-mtc-log-coordination}"
ENDPOINT="${AWS_ENDPOINT_URL:-http://127.0.0.1:4566}"
TOKEN_LABEL="${MTC_PKCS11_TOKEN_LABEL:-mtc-dev}"
PIN="${MTC_PKCS11_PIN:-1234}"
KEY_LABEL="${MTC_PKCS11_KEY_LABEL:-checkpoint-signing}"
MODULE="${MTC_PKCS11_MODULE_PATH:-/usr/lib/softhsm/libsofthsm2.so}"

GREEN=$'\033[32m'
RED=$'\033[31m'
RESET=$'\033[0m'
FAILURES=0

# check_grep DESC PATTERN CMD... — run CMD, require PATTERN in its output.
check_grep() {
  local desc="$1" pattern="$2"
  shift 2
  local out
  if out="$("$@" 2>&1)" && grep -q "${pattern}" <<<"${out}"; then
    printf '%s✓ PASS%s %s\n' "${GREEN}" "${RESET}" "${desc}"
  else
    printf '%s✗ FAIL%s %s\n' "${RED}" "${RESET}" "${desc}"
    printf '        command : %s\n' "$*"
    printf '        expected: %s\n' "${pattern}"
    printf '        got     : %.400s\n' "${out}"
    FAILURES=$((FAILURES + 1))
  fi
}

echo "== MTC local dev environment smoke test =="

# --- LocalStack --------------------------------------------------------------
check_grep "LocalStack health endpoint (${ENDPOINT})" \
  '"s3"' \
  curl -sf "${ENDPOINT}/_localstack/health"

check_grep "log bucket '${BUCKET}': Object Lock enabled (§8.1)" \
  '"ObjectLockEnabled": "Enabled"' \
  "${COMPOSE[@]}" exec -T localstack \
  awslocal s3api get-object-lock-configuration --bucket "${BUCKET}"

check_grep "log bucket '${BUCKET}': Object Lock default retention COMPLIANCE" \
  '"Mode": "COMPLIANCE"' \
  "${COMPOSE[@]}" exec -T localstack \
  awslocal s3api get-object-lock-configuration --bucket "${BUCKET}"

check_grep "log bucket '${BUCKET}': versioning enabled (§8.1)" \
  '"Status": "Enabled"' \
  "${COMPOSE[@]}" exec -T localstack \
  awslocal s3api get-bucket-versioning --bucket "${BUCKET}"

check_grep "coordination table '${TABLE}': ACTIVE with PK/SK schema (§8.2)" \
  '^ACTIVE[[:space:]]*PK[[:space:]]*SK$' \
  "${COMPOSE[@]}" exec -T localstack \
  awslocal dynamodb describe-table --table-name "${TABLE}" \
  --query 'Table.[TableStatus, KeySchema[?KeyType==`HASH`].AttributeName | [0], KeySchema[?KeyType==`RANGE`].AttributeName | [0]]' \
  --output text

# --- SoftHSM2 ----------------------------------------------------------------
check_grep "SoftHSM2 token '${TOKEN_LABEL}' visible via pkcs11-tool (§14)" \
  "token label.*${TOKEN_LABEL}" \
  "${COMPOSE[@]}" exec -T softhsm \
  pkcs11-tool --module "${MODULE}" --list-slots

check_grep "SoftHSM2 keypair '${KEY_LABEL}' present (public key)" \
  "label:[[:space:]]*${KEY_LABEL}" \
  "${COMPOSE[@]}" exec -T softhsm \
  pkcs11-tool --module "${MODULE}" --token-label "${TOKEN_LABEL}" \
  --list-objects --type pubkey

# pkcs11-tool prints the curve as an OID; 1.2.840.10045.3.1.7 == prime256v1 (P-256).
check_grep "SoftHSM2 key '${KEY_LABEL}' is ECDSA P-256 (§14.1 v1)" \
  "1\.2\.840\.10045\.3\.1\.7\|prime256v1\|secp256r1" \
  "${COMPOSE[@]}" exec -T softhsm \
  pkcs11-tool --module "${MODULE}" --token-label "${TOKEN_LABEL}" \
  --list-objects --type pubkey

check_grep "SoftHSM2 private key usable (login + list, §14)" \
  "Private Key Object" \
  "${COMPOSE[@]}" exec -T softhsm \
  pkcs11-tool --module "${MODULE}" --token-label "${TOKEN_LABEL}" \
  --login --pin "${PIN}" --list-objects --type privkey

echo
if [[ "${FAILURES}" -eq 0 ]]; then
  printf '%sAll checks green.%s\n' "${GREEN}" "${RESET}"
else
  printf '%s%d check(s) failed.%s\n' "${RED}" "${FAILURES}" "${RESET}"
  exit 1
fi
