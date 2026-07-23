#!/usr/bin/env bash
# test-lint-runbooks.sh — pass/fail fixture tests for scripts/lint-runbooks.sh.
#
# The pass fixture lives at scripts/testdata/lint-runbooks/pass/. Fail cases
# are derived from it: each test copies the fixture to a temp dir, applies one
# structural mutation, and asserts the lint fails with a message naming the
# problem. Also asserts the real docs/runbooks/ tree lints clean.
#
# Usage: test-lint-runbooks.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LINT="$SCRIPT_DIR/lint-runbooks.sh"
PASS_FIXTURE="$SCRIPT_DIR/testdata/lint-runbooks/pass"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

passed=0
failed=0

ok()   { passed=$((passed + 1)); echo "PASS: $1"; }
bad()  { failed=$((failed + 1)); echo "FAIL: $1 — $2" >&2; }

# expect_lint_ok NAME DIR
expect_lint_ok() {
  local name="$1" dir="$2" out
  if out="$(bash "$LINT" "$dir" 2>&1)"; then
    ok "$name"
  else
    bad "$name" "expected lint to pass, got: $out"
  fi
}

# expect_lint_fail NAME DIR NEEDLE — lint must exit non-zero and mention NEEDLE.
expect_lint_fail() {
  local name="$1" dir="$2" needle="$3" out
  if out="$(bash "$LINT" "$dir" 2>&1)"; then
    bad "$name" "expected lint to fail, but it passed"
  elif ! grep -qF "$needle" <<<"$out"; then
    bad "$name" "lint failed but output lacks '$needle'; got: $out"
  else
    ok "$name"
  fi
}

# mutate NAME — fresh copy of the pass fixture at $TMP/NAME, echoes the path.
mutate() {
  local dir="$TMP/$1"
  rm -rf "$dir"
  cp -r "$PASS_FIXTURE" "$dir"
  echo "$dir"
}

# --- pass cases ------------------------------------------------------------

expect_lint_ok "pass fixture lints clean" "$PASS_FIXTURE"
expect_lint_ok "seeded docs/runbooks lints clean" "$REPO_ROOT/docs/runbooks"

# --- fail cases ------------------------------------------------------------

d="$(mutate missing-section)"
sed -i '/^## Recovery procedure$/d' "$d/crr-stall.md"
expect_lint_fail "missing required section" "$d" \
  "missing required section '## Recovery procedure'"

d="$(mutate duplicate-section)"
printf '\n## Detection\n\nDuplicate.\n' >> "$d/adapter-flood.md"
expect_lint_fail "duplicate section" "$d" "duplicate section '## Detection'"

d="$(mutate out-of-order)"
sed -i '/^## Detection$/d' "$d/primary-failure.md"
printf '\n## Detection\n\nMoved to the end.\n' >> "$d/primary-failure.md"
expect_lint_fail "sections out of order" "$d" "sections out of order"

d="$(mutate missing-title)"
sed -i '/^# Runbook:/d' "$d/pruning-failure.md"
expect_lint_fail "missing title heading" "$d" "title heading"

d="$(mutate template-broken)"
sed -i '/^## Mitigation steps$/d' "$d/TEMPLATE.md"
expect_lint_fail "TEMPLATE.md missing section" "$d" \
  "TEMPLATE.md: missing required section '## Mitigation steps'"

d="$(mutate postmortem-broken)"
sed -i '/^## Action items$/d' "$d/POSTMORTEM.md"
expect_lint_fail "POSTMORTEM.md missing section" "$d" \
  "missing required section '## Action items'"

d="$(mutate postmortem-absent)"
rm "$d/POSTMORTEM.md"
expect_lint_fail "POSTMORTEM.md missing" "$d" "POSTMORTEM.md is missing"

d="$(mutate index-entry-missing)"
sed -i '/](hsm-unavailability\.md)/d' "$d/README.md"
rm "$d/hsm-unavailability.md"
expect_lint_fail "required runbook missing from index" "$d" \
  "missing index entry for required runbook 'hsm-unavailability.md'"

d="$(mutate index-bad-status)"
sed -i 's/^\(| \[emergency-revocation\].*\) stub \(.*\)$/\1 in-progress \2/' "$d/README.md"
expect_lint_fail "index status not stub/complete" "$d" \
  "lacks a status of 'stub' or 'complete'"

d="$(mutate unindexed-file)"
cp "$d/crr-stall.md" "$d/extra-runbook.md"
expect_lint_fail "runbook file absent from index" "$d" \
  "'extra-runbook.md' exists but is not listed in the index"

d="$(mutate dead-link)"
rm "$d/self-auditor-anomaly.md"  # README row remains, so the link is now dead
expect_lint_fail "index links to missing file" "$d" \
  "links to 'self-auditor-anomaly.md' which does not exist"

d="$(mutate readme-absent)"
rm "$d/README.md"
expect_lint_fail "README.md missing" "$d" "README.md is missing"

# --- verdict ---------------------------------------------------------------

echo
echo "test-lint-runbooks: $passed passed, $failed failed"
(( failed == 0 )) || exit 1
