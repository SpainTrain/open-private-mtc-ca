#!/usr/bin/env bash
# E2E smoke test for the Makefile target skeleton (ticket: dev-make-skeleton).
#
# Asserts that `make help` lists every spec §18.8 target (plus the lifecycle
# companions demo-down / demo-multiregion-down / dev) and that each
# not-yet-implemented stub exits non-zero.
set -euo pipefail

# Resolve the repo root from this script's location so it runs from anywhere.
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
cd "${repo_root}"

# Spec §18.8 targets plus the lifecycle companions. These are all stubs today.
required_targets=(
  demo demo-down demo-multiregion demo-multiregion-down dev
  test test-unit test-prop test-conformance test-chaos test-soak test-e2e
  repl fixture-load fixture-save time-advance partition-region
  api-gen codemap agent-context agent-precheck verify-task journal
  fmt lint audit bench doctor
)

fail=0

# Strip ANSI color codes so grep matches the target names cleanly.
help_out="$(make help | sed 's/\x1b\[[0-9;]*m//g')"

echo "== make help lists all targets =="
for t in "${required_targets[@]}"; do
  if grep -qE "^[[:space:]]+${t}[[:space:]]" <<<"${help_out}"; then
    printf '  ok    %s\n' "${t}"
  else
    printf '  MISSING  %s\n' "${t}"
    fail=1
  fi
done

echo "== stub targets exit non-zero =="
for t in "${required_targets[@]}"; do
  if make "${t}" >/dev/null 2>&1; then
    printf '  UNEXPECTED-EXIT-0  %s\n' "${t}"
    fail=1
  else
    printf '  ok (non-zero)  %s\n' "${t}"
  fi
done

if [[ "${fail}" -ne 0 ]]; then
  echo "make-targets smoke: FAIL"
  exit 1
fi
echo "make-targets smoke: PASS"
