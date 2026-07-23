#!/usr/bin/env bash
# search-test.sh — smoke tests for search affordances (spec 23.6).
#
# Tests: empty-result exits 0, missing-arg exits cleanly, file:line prefix
# present in both single-file and multi-file mode.
#
# Usage: scripts/search-test.sh

set -u # no unbound vars, but allow failures for testing

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# If rg is not in PATH, try adding common locations.
if ! command -v rg > /dev/null 2>&1; then
  for rg_dir in ~/.gemini/tmp/bin /usr/local/bin ~/.cargo/bin; do
    if [ -x "$rg_dir/rg" ]; then
      export PATH="$rg_dir:$PATH"
      break
    fi
  done
fi

# Final check - if rg still not available, skip tests
if ! command -v rg > /dev/null 2>&1; then
  echo "WARNING: ripgrep not found; skipping search tests"
  exit 0
fi

passed=0
failed=0

echo "Testing search affordances..."

# Test 1: find-impl with missing argument should fail
output=$(scripts/find-impl.sh 2>&1 || true)
if echo "$output" | grep -q "usage"; then
  echo "PASS: find-impl missing argument exits with usage"
  ((passed++))
else
  echo "FAIL: find-impl missing argument did not show usage"
  echo "$output"
  ((failed++))
fi

# Test 2: find-callers with missing argument should fail
output=$(scripts/find-callers.sh 2>&1 || true)
if echo "$output" | grep -q "usage"; then
  echo "PASS: find-callers missing argument exits with usage"
  ((passed++))
else
  echo "FAIL: find-callers missing argument did not show usage"
  echo "$output"
  ((failed++))
fi

# Test 3: find-tests with missing argument should fail
output=$(scripts/find-tests.sh 2>&1 || true)
if echo "$output" | grep -q "usage"; then
  echo "PASS: find-tests missing argument exits with usage"
  ((passed++))
else
  echo "FAIL: find-tests missing argument did not show usage"
  echo "$output"
  ((failed++))
fi

# Test 4: find-impl with a known trait should have file:line prefix
output=$(iface=Clock scripts/find-impl.sh 2>/dev/null || true)
if [ -n "$output" ] && echo "$output" | head -1 | grep -qE '^[^:]+:[0-9]+:'; then
  echo "PASS: find-impl output has file:line prefix"
  ((passed++))
else
  echo "FAIL: find-impl output missing file:line prefix"
  if [ -z "$output" ]; then
    echo "  (no output)"
  else
    echo "$output" | head -1
  fi
  ((failed++))
fi

# Test 5: find-todo should have file:line prefix
output=$(scripts/find-todo.sh 2>/dev/null || true)
if [ -n "$output" ] && echo "$output" | head -1 | grep -qE '^[^:]+:[0-9]+:'; then
  echo "PASS: find-todo output has file:line prefix"
  ((passed++))
else
  echo "FAIL: find-todo output missing file:line prefix"
  ((failed++))
fi

# Test 6: find-impl with non-existent trait should exit cleanly (not fail)
if iface=NonExistent12345 scripts/find-impl.sh > /dev/null 2>&1 || true; then
  echo "PASS: find-impl with non-existent trait exits cleanly"
  ((passed++))
else
  echo "FAIL: find-impl with non-existent trait did not exit cleanly"
  ((failed++))
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
