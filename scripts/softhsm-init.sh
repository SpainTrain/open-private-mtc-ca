#!/usr/bin/env bash
# Provision a host-side SoftHSM2 token for the cloud-softhsm PKCS#11 backend
# (spec §4 HSM dev row, §14.1). A process on the host cannot dlopen a module
# running inside the docker `softhsm` container, so the in-process Rust HSM
# tests need a *host* SoftHSM2 install (deploy/local/README.md). This script is
# the host counterpart of deploy/local/softhsm/entrypoint.sh and provisions the
# identical token: label `mtc-dev`, user PIN `1234`, one ECDSA P-256 key
# `checkpoint-signing`.
#
# Idempotent: re-running is a no-op once the token exists.
#
# Usage:
#   ./scripts/softhsm-init.sh
#   cargo test -p cloud-softhsm --features integration -- --test-threads=1
#
# Honors the MTC_PKCS11_* env contract (deploy/local/local.env) plus:
#   MTC_PKCS11_MODULE_PATH  PKCS#11 module (default: platform auto-detect)
#   MTC_PKCS11_TOKEN_LABEL  token label   (default: mtc-dev)
#   MTC_PKCS11_PIN          user PIN      (default: 1234)
#   MTC_PKCS11_KEY_LABEL    key label     (default: checkpoint-signing)
#   SOFTHSM_SO_PIN          SO PIN        (default: 0102030405060708)
#   SOFTHSM2_CONF           config path   (default: user config, see below)
set -euo pipefail

LABEL="${MTC_PKCS11_TOKEN_LABEL:-mtc-dev}"
PIN="${MTC_PKCS11_PIN:-1234}"
SO_PIN="${SOFTHSM_SO_PIN:-0102030405060708}"
KEY_LABEL="${MTC_PKCS11_KEY_LABEL:-checkpoint-signing}"

fail() {
  echo "[softhsm-init] error: $*" >&2
  exit 1
}

command -v softhsm2-util >/dev/null 2>&1 \
  || fail "softhsm2-util not found — install SoftHSM2 (Debian/Ubuntu: 'sudo apt-get install -y softhsm2 opensc'; macOS: 'brew install softhsm opensc')"
command -v pkcs11-tool >/dev/null 2>&1 \
  || fail "pkcs11-tool not found — install OpenSC ('sudo apt-get install -y opensc' / 'brew install opensc')"

# Resolve the PKCS#11 module: explicit env wins, else probe the usual paths.
MODULE="${MTC_PKCS11_MODULE_PATH:-}"
if [ -z "${MODULE}" ]; then
  for candidate in \
    /usr/lib/softhsm/libsofthsm2.so \
    /usr/lib/x86_64-linux-gnu/softhsm/libsofthsm2.so \
    /usr/local/lib/softhsm/libsofthsm2.so \
    /opt/homebrew/lib/softhsm/libsofthsm2.so \
    /usr/local/opt/softhsm/lib/softhsm/libsofthsm2.so; do
    if [ -f "${candidate}" ]; then
      MODULE="${candidate}"
      break
    fi
  done
fi
[ -n "${MODULE}" ] && [ -f "${MODULE}" ] \
  || fail "libsofthsm2.so not found — set MTC_PKCS11_MODULE_PATH to its location"

# Ensure a discoverable SoftHSM2 config with a writable, user-owned token dir,
# so this needs no root. SoftHSM2's search order is: SOFTHSM2_CONF env, then
# ~/.config/softhsm2/softhsm2.conf, then the system config. We only create the
# user config when nothing else is configured, so existing setups win.
if [ -z "${SOFTHSM2_CONF:-}" ] && [ ! -f "${HOME}/.config/softhsm2/softhsm2.conf" ]; then
  TOKENDIR="${SOFTHSM_TOKENDIR:-${HOME}/.local/share/softhsm/tokens}"
  mkdir -p "${TOKENDIR}" "${HOME}/.config/softhsm2"
  cat >"${HOME}/.config/softhsm2/softhsm2.conf" <<EOF
# Written by scripts/softhsm-init.sh — user-local SoftHSM2 store (dev only).
directories.tokendir = ${TOKENDIR}
objectstore.backend = file
log.level = ERROR
EOF
  echo "[softhsm-init] wrote ${HOME}/.config/softhsm2/softhsm2.conf (tokendir ${TOKENDIR})"
fi

# --show-slots pads the label to a 32-char field with trailing spaces.
if softhsm2-util --show-slots 2>/dev/null | grep -qE "Label:[[:space:]]*${LABEL}[[:space:]]*\$"; then
  echo "[softhsm-init] token '${LABEL}' already initialized, skipping"
else
  echo "[softhsm-init] initializing token '${LABEL}'"
  softhsm2-util --init-token --free \
    --label "${LABEL}" \
    --so-pin "${SO_PIN}" \
    --pin "${PIN}"

  echo "[softhsm-init] generating ECDSA P-256 key '${KEY_LABEL}' (spec §14.1 v1)"
  pkcs11-tool --module "${MODULE}" \
    --token-label "${LABEL}" \
    --login --pin "${PIN}" \
    --keypairgen --key-type EC:prime256v1 \
    --label "${KEY_LABEL}" --id 01
fi

echo "[softhsm-init] ready — module: ${MODULE}, token: ${LABEL}, key: ${KEY_LABEL}"
echo "[softhsm-init] run: MTC_PKCS11_MODULE_PATH=${MODULE} cargo test -p cloud-softhsm --features integration -- --test-threads=1"
