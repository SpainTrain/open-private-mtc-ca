#!/usr/bin/env bash
# verify-task.sh — pre-done acceptance gate (spec §23.4).
#
# Run before declaring a task done (`make verify-task`). Steps:
#   1. lint          — rustfmt check + clippy with -D warnings, whole workspace
#   2. fast tests    — cargo test --workspace (unit + integration + doc tests)
#   3. doc/skill/rules lints — the documentation gates, delegated to their
#      owning make targets: rules-lint, skill-lint, lint-runbooks,
#      lint-security-checklist
#
# Every step runs even if an earlier one failed, so one invocation reports
# everything left to fix; any failure makes the final exit status non-zero —
# the task is not done. Runs fully offline on a laptop. `make doctor` owns
# full environment diagnostics.

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

# --- 1. lint ----------------------------------------------------------------
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

# --- 2. fast test suite -----------------------------------------------------
banner 'fast test suite (cargo test --workspace)'
test_rc=0
if command -v cargo > /dev/null 2>&1; then
  cargo test --workspace --quiet || test_rc=1
else
  echo '  cargo not found — cannot run tests (run "make doctor")'
  test_rc=1
fi
step_result 'fast test suite' "$test_rc"

# --- 3. doc/skill/rules lints ----------------------------------------------
# Delegated to the owning make targets so this gate keeps working when their
# implementations move (see mk/rules.mk, mk/skills.mk, mk/runbooks.mk,
# mk/security.mk).
for lint_target in rules-lint skill-lint lint-runbooks lint-security-checklist; do
  banner "doc lint (make $lint_target)"
  make -s "$lint_target"
  step_result "make $lint_target" "$?"
done

# --- summary ----------------------------------------------------------------
banner 'summary'
if [ ${#failed_steps[@]} -eq 0 ]; then
  printf 'verify-task: PASS (%ss) — lint, tests, and doc/skill/rules lints all green\n' "$SECONDS"
  exit 0
fi
printf 'verify-task: FAIL (%ss) — failed step(s): %s\n' "$SECONDS" "${failed_steps[*]}" >&2
exit 1
