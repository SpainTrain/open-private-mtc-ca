#!/bin/sh
# Provision the SoftHSM2 dev token (spec §4 HSM dev row, §14.1):
#   - initialize a token (label: mtc-dev by default)
#   - generate an ECDSA P-256 keypair for checkpoint signing
# Idempotent: skips provisioning if the token already exists (persisted volume).
set -eu

LABEL="${SOFTHSM_TOKEN_LABEL:-mtc-dev}"
SO_PIN="${SOFTHSM_SO_PIN:-0102030405060708}"
PIN="${SOFTHSM_USER_PIN:-1234}"
KEY_LABEL="${SOFTHSM_KEY_LABEL:-checkpoint-signing}"
MODULE="${MTC_PKCS11_MODULE_PATH:-/usr/lib/softhsm/libsofthsm2.so}"

# Note: --show-slots pads the label field with trailing spaces (32-char field).
if softhsm2-util --show-slots | grep -q "Label:[[:space:]]*${LABEL}[[:space:]]*\$"; then
  echo "[softhsm] token '${LABEL}' already initialized, skipping provisioning"
else
  echo "[softhsm] initializing token '${LABEL}'"
  softhsm2-util --init-token --free \
    --label "${LABEL}" \
    --so-pin "${SO_PIN}" \
    --pin "${PIN}"

  echo "[softhsm] generating ECDSA P-256 keypair '${KEY_LABEL}' (spec §14.1 v1)"
  pkcs11-tool --module "${MODULE}" \
    --token-label "${LABEL}" \
    --login --pin "${PIN}" \
    --keypairgen --key-type EC:prime256v1 \
    --label "${KEY_LABEL}" --id 01
fi

echo "[softhsm] ready — module: ${MODULE}, token: ${LABEL}, key: ${KEY_LABEL}"
touch /tmp/softhsm_ready

# Keep the container alive for `docker compose exec` (smoke tests, pkcs11-tool).
exec tail -f /dev/null
