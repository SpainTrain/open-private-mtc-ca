#!/usr/bin/env bash
# Reset the LocalStack + SoftHSM2 local dev environment to a clean slate:
# stops containers and removes named volumes (SoftHSM token state).
# LocalStack itself is stateless across restarts, so down + volume wipe
# guarantees the next `make dev-env-up` re-provisions everything from scratch.
#
# Usage: ./scripts/dev-env-reset.sh   (from anywhere; paths are self-locating)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

docker compose -f "${REPO_ROOT}/deploy/local/docker-compose.yml" \
  down --volumes --remove-orphans

echo "Local dev environment reset to a clean slate."
echo "Start fresh with: make dev-env-up"
