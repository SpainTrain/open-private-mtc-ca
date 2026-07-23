#!/usr/bin/env bash
# doctor.sh — dev environment diagnostics (spec §18.8).
#
# Diagnose the dev environment and print actionable remediation so onboarding
# failures are self-explaining instead of mysterious. Exits 0 only when all
# hard checks pass; WARN-only exits 0.
#
# Checks:
# - Docker daemon reachable
# - Docker Compose v2 present
# - Required ports free (LocalStack 4566, admin UI 8080)
# - Rust toolchain (rustc/cargo) matches rust-toolchain.toml
# - cargo-watch and evcxr_repl installed
# - SoftHSM2 library present
# - LocalStack image pulled
# - Sufficient free disk space (2GB)
#
# Usage: scripts/doctor.sh (via `make doctor`)

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
cd "$repo_root"

# Rust tools (cargo, rustc) live in ~/.cargo/bin.
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# Color codes for output.
PASS_COLOR='\033[32m'  # green
FAIL_COLOR='\033[31m'  # red
WARN_COLOR='\033[33m'  # yellow
RESET_COLOR='\033[0m'

# Track overall status.
any_hard_failed=false
any_soft_failed=false

# Helper to print a check result.
# Args: status (PASS, FAIL, WARN), message, [fix_command]
report_check() {
  local status="$1"
  local message="$2"
  local fix_command="${3:-}"

  case "$status" in
    PASS)
      printf '%b%-6s%b %s\n' "$PASS_COLOR" "✓ PASS" "$RESET_COLOR" "$message"
      ;;
    FAIL)
      printf '%b%-6s%b %s\n' "$FAIL_COLOR" "✗ FAIL" "$RESET_COLOR" "$message"
      if [ -n "$fix_command" ]; then
        printf '         Fix: %s\n' "$fix_command"
      fi
      any_hard_failed=true
      ;;
    WARN)
      printf '%b%-6s%b %s\n' "$WARN_COLOR" "⚠ WARN" "$RESET_COLOR" "$message"
      if [ -n "$fix_command" ]; then
        printf '         Fix: %s\n' "$fix_command"
      fi
      ;;
  esac
}

printf '\n%sDev Environment Diagnostics%s\n' "$PASS_COLOR" "$RESET_COLOR"
printf '%s=================================\n' "$PASS_COLOR" && printf '%s\n' "$RESET_COLOR"

# Check 1: Docker daemon reachable.
if command -v docker > /dev/null 2>&1 && docker ps > /dev/null 2>&1; then
  report_check "PASS" "Docker daemon is reachable"
else
  report_check "FAIL" "Docker daemon not reachable" \
    "Start Docker (systemctl start docker or open Docker Desktop)"
fi

# Check 2: Docker Compose v2 present.
if command -v docker-compose > /dev/null 2>&1 || docker compose version > /dev/null 2>&1; then
  report_check "PASS" "Docker Compose v2 is available"
else
  report_check "FAIL" "Docker Compose v2 not found" \
    "Install Docker Compose: see https://docs.docker.com/compose/install/"
fi

# Check 3: Required ports free (skip if no tool available).
# Try to check with nc if available; otherwise skip as a WARN.
ports_checked=false
port_4566_free=true
port_8080_free=true

if command -v nc > /dev/null 2>&1; then
  ports_checked=true
  # Use timeout to prevent hanging.
  if timeout 1 nc -z 127.0.0.1 4566 2>/dev/null; then
    port_4566_free=false
  fi
  if timeout 1 nc -z 127.0.0.1 8080 2>/dev/null; then
    port_8080_free=false
  fi
fi

if [ "$ports_checked" = true ]; then
  if [ "$port_4566_free" = true ] && [ "$port_8080_free" = true ]; then
    report_check "PASS" "Required ports are free (4566, 8080)"
  else
    ports_busy=""
    [ "$port_4566_free" = false ] && ports_busy="4566"
    [ "$port_8080_free" = false ] && ports_busy="$ports_busy 8080"
    report_check "FAIL" "Required ports are in use: $ports_busy" \
      "Stop services using these ports (make demo-down may help)"
  fi
else
  report_check "WARN" "Port check skipped (nc not available)" \
    "Make sure ports 4566 and 8080 are free before running 'make demo'"
fi

# Check 4: Rust toolchain matches rust-toolchain.toml.
if command -v rustc > /dev/null 2>&1; then
  expected_version=$(grep channel rust-toolchain.toml 2>/dev/null | grep -o '[0-9.]*' | head -1)
  actual_version=$(rustc --version | grep -o '[0-9.]*' | head -1)
  if [ "$expected_version" = "$actual_version" ]; then
    report_check "PASS" "Rust toolchain (rustc/cargo) matches spec ($actual_version)"
  else
    report_check "WARN" "Rust version mismatch: expected $expected_version, found $actual_version" \
      "rustup default ${expected_version} (or: rustup update)"
  fi
else
  report_check "FAIL" "Rust toolchain (rustc/cargo) not found" \
    "Install Rust: curl https://sh.rustup.rs | sh"
fi

# Check 5: cargo-watch installed.
if command -v cargo-watch > /dev/null 2>&1; then
  report_check "PASS" "cargo-watch is installed"
else
  report_check "WARN" "cargo-watch not installed (used by 'make watch')" \
    "cargo install --locked cargo-watch"
fi

# Check 6: evcxr_repl installed.
if command -v evcxr_repl > /dev/null 2>&1; then
  report_check "PASS" "evcxr_repl is installed"
else
  report_check "WARN" "evcxr_repl not installed (used by 'make repl')" \
    "cargo install --locked evcxr_repl"
fi

# Check 7: SoftHSM2 library present.
if pkg-config --exists libsofthsm2 2>/dev/null; then
  report_check "PASS" "SoftHSM2 library is available"
else
  report_check "WARN" "SoftHSM2 library not found (softhsm-backend needs this)" \
    "apt-get install softhsm2 (or: brew install softhsm)"
fi

# Check 8: LocalStack image is pulled.
if command -v docker > /dev/null 2>&1; then
  if docker image inspect localstack/localstack:latest > /dev/null 2>&1; then
    report_check "PASS" "LocalStack image is pulled"
  else
    report_check "WARN" "LocalStack image not pulled (will be fetched on first demo run)" \
      "docker pull localstack/localstack:latest (optional; auto-downloads)"
  fi
else
  report_check "WARN" "Cannot check LocalStack image (Docker not running)"
fi

# Check 9: Sufficient free disk space (2GB).
if command -v df > /dev/null 2>&1; then
  free_space_kb=$(df "$repo_root" | awk 'NR==2 {print $4}')
  free_space_gb=$((free_space_kb / 1024 / 1024))
  if [ "$free_space_gb" -ge 2 ]; then
    report_check "PASS" "Sufficient disk space ($free_space_gb GB free)"
  else
    report_check "FAIL" "Insufficient disk space ($free_space_gb GB free, need 2 GB)" \
      "Free up disk space on $repo_root or use another volume"
  fi
fi

printf '\n%s=================================\n' "$PASS_COLOR" && printf '%s\n' "$RESET_COLOR"

# Summary.
if [ "$any_hard_failed" = true ]; then
  printf '%bSummary: Some checks failed. Fix the items above, then re-run.%s\n\n' "$FAIL_COLOR" "$RESET_COLOR"
  exit 1
else
  printf '%bSummary: All hard checks passed. You are ready to run `make demo`.%s\n\n' "$PASS_COLOR" "$RESET_COLOR"
  exit 0
fi
