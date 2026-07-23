#!/bin/bash
# LocalStack ready-hook: create the MTC data model (spec §8.1/§8.2).
# Runs inside the LocalStack container once the gateway is ready.
# Idempotent: safe if resources already exist (container restart).
set -euo pipefail

BUCKET="${MTC_LOG_BUCKET:-mtc-log-local}"
TABLE="${MTC_COORDINATION_TABLE:-mtc-log-coordination}"

echo "[mtc-init] creating log bucket '${BUCKET}' (versioning + Object Lock, spec §8.1)"
if awslocal s3api head-bucket --bucket "${BUCKET}" 2>/dev/null; then
  echo "[mtc-init] bucket already exists, skipping create"
else
  awslocal s3api create-bucket \
    --bucket "${BUCKET}" \
    --object-lock-enabled-for-bucket
fi

# Object Lock implies versioning, but the spec lists both invariants — be explicit.
awslocal s3api put-bucket-versioning \
  --bucket "${BUCKET}" \
  --versioning-configuration Status=Enabled

# Compliance mode = true append-only (spec §8 "S3 invariants", §15.3).
# Short default retention for local dev; reset wipes the container anyway.
awslocal s3api put-object-lock-configuration \
  --bucket "${BUCKET}" \
  --object-lock-configuration \
  '{"ObjectLockEnabled": "Enabled", "Rule": {"DefaultRetention": {"Mode": "COMPLIANCE", "Days": 1}}}'

echo "[mtc-init] creating coordination table '${TABLE}' (spec §8.2)"
if awslocal dynamodb describe-table --table-name "${TABLE}" >/dev/null 2>&1; then
  echo "[mtc-init] table already exists, skipping create"
else
  # Single-table design: PK = "log#{logId}", SK = item kind (counter, lease,
  # latest-checkpoint, batch#..., audit#...). Attributes beyond the key are
  # schemaless per DynamoDB.
  awslocal dynamodb create-table \
    --table-name "${TABLE}" \
    --attribute-definitions \
      AttributeName=PK,AttributeType=S \
      AttributeName=SK,AttributeType=S \
    --key-schema \
      AttributeName=PK,KeyType=HASH \
      AttributeName=SK,KeyType=RANGE \
    --billing-mode PAY_PER_REQUEST
  awslocal dynamodb wait table-exists --table-name "${TABLE}"
fi

touch /tmp/mtc_init_done
echo "[mtc-init] done"
