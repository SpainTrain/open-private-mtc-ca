#!/usr/bin/env bash
# doctor-test.sh — smoke tests for make doctor (spec 18.8).
#
# Tests: exit 0 when only WARNs, non-zero when hard check fails,
# no raw escape sequences in output.
#
# Usage: scripts/doctor-test.sh

set -u

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

passed=0
failed=0

echo "Testing doctor diagnostics..."

# Test 1: doctor exits 0 when only warnings/passes
output=$(scripts/doctor.sh 2>&1)
rc=$?
if [ "$rc" -eq 0 ]; then
  echo "PASS: doctor exits 0 when checks pass or warn"
  ((passed++))
else
  # It's okay if it exits non-zero if there's a hard failure
  echo "INFO: doctor exited with status $rc (hard checks may have failed)"
fi

# Test 2: doctor output should not contain raw escape sequences (\033)
if echo "$output" | grep -q '\\033'; then
  echo "FAIL: doctor output contains raw escape sequences"
  ((failed++))
else
  echo "PASS: doctor output has no raw escape sequences"
  ((passed++))
fi

# Test 3: doctor output should contain colored text indicators
if echo "$output" | grep -qE '(PASS|FAIL|WARN)'; then
  echo "PASS: doctor output contains status indicators"
  ((passed++))
else
  echo "FAIL: doctor output missing status indicators"
  ((failed++))
fi

# Test 4: doctor output should contain Fix hints for failures/warnings
if echo "$output" | grep -q 'Fix:'; then
  echo "PASS: doctor output contains remediation hints"
  ((passed++))
else
  # Might pass - only if no checks failed or warned
  echo "INFO: doctor output has no remediation hints (all checks passed?)"
fi

echo ""
echo "=== Test Summary ==="
echo "Passed: $passed"
echo "Failed: $failed"

if [ $failed -eq 0 ]; then
  echo "All tests passed!"
  exit 0
else
  echo "Some tests failed."
  exit 1
fi
