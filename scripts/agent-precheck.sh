#!/usr/bin/env bash
# agent-precheck.sh — pre-task verification gate (spec §23.4).
#
# Run before starting any task (`make agent-precheck`). Steps:
#   1. environment  — required tools present (Rust toolchain, docker, beads CLI)
#   2. context      — recent decisions from docs/journal.md
#   3. lint         — rustfmt check + clippy with -D warnings, whole workspace
#   4. fast tests   — workspace unit tests (lib targets only, for speed)
#
# Every step runs even if an earlier one failed, so one invocation reports the
# full baseline; any failure makes the final exit status non-zero. Runs fully
# offline and targets ~60s on a warm laptop. When a tool is missing you get a
# one-line pointer — `make doctor` owns full diagnostics.
#
# Environment overrides (used by scripts/agent-inner-loop-test.sh):
#   PRECHECK_TOOLS            space-separated required tools
#                             (default: "cargo rustc docker bd")
#   PRECHECK_JOURNAL_ENTRIES  how many recent journal entries to print (default: 3)
#   JOURNAL_FILE              journal path (passed through to recent-journal.sh)

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
cd "$repo_root" || exit 1

# Rust tools (cargo, rustfmt, clippy) live in ~/.cargo/bin.
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

failed_steps=()
step_start=$SECONDS

banner() {
  printf '\n== %s ==\n' "$1"
}

# step_result <name> <rc> — record and print one step's outcome.
step_result() {
  local elapsed=$((SECONDS - step_start))
  if [ "$2" -eq 0 ]; then
    printf '   PASS  %s (%ss)\n' "$1" "$elapsed"
  else
    printf '   FAIL  %s (%ss)\n' "$1" "$elapsed"
    failed_steps+=("$1")
  fi
  step_start=$SECONDS
}

# --- 1. environment: required tools ----------------------------------------
banner 'environment'
env_rc=0
for tool in ${PRECHECK_TOOLS:-cargo rustc docker bd}; do
  if command -v "$tool" > /dev/null 2>&1; then
    version=$("$tool" --version 2> /dev/null | head -n 1 || true)
    printf '  ok       %-8s %s\n' "$tool" "${version:-$(command -v "$tool")}"
  else
    printf '  MISSING  %-8s not on PATH — run "make doctor" for diagnostics\n' "$tool"
    env_rc=1
  fi
done
step_result 'environment (required tools)' "$env_rc"

# --- 2. context: recent decisions ------------------------------------------
banner 'recent decisions (docs/journal.md)'
"$script_dir/recent-journal.sh" "${PRECHECK_JOURNAL_ENTRIES:-3}"
step_result 'recent journal entries' "$?"

# --- 3. lint: current workspace state --------------------------------------
banner 'lint (cargo fmt --check + clippy -D warnings)'
lint_rc=0
if command -v cargo > /dev/null 2>&1; then
  cargo fmt --all --check || lint_rc=1
  cargo clippy --workspace --all-targets --quiet -- -D warnings || lint_rc=1
else
  echo '  cargo not found — cannot lint (run "make doctor")'
  lint_rc=1
fi
step_result 'workspace lint' "$lint_rc"

# --- 4. fast unit tests -----------------------------------------------------
banner 'fast unit tests (cargo test --workspace --lib)'
test_rc=0
if command -v cargo > /dev/null 2>&1; then
  cargo test --workspace --lib --quiet || test_rc=1
else
  echo '  cargo not found — cannot run tests (run "make doctor")'
  test_rc=1
fi
step_result 'fast unit tests' "$test_rc"

# --- summary ----------------------------------------------------------------
banner 'summary'
if [ ${#failed_steps[@]} -eq 0 ]; then
  printf 'agent-precheck: PASS (%ss) — environment, journal, lint, fast tests all green\n' "$SECONDS"
  exit 0
fi
printf 'agent-precheck: FAIL (%ss) — failed step(s): %s\n' "$SECONDS" "${failed_steps[*]}" >&2
exit 1
