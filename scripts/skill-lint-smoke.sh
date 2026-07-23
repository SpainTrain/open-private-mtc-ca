#!/usr/bin/env bash
# skill-lint-smoke.sh — smoke tests for scripts/skill-lint.sh (spec
# section 19.13 spirit: E2E as plain shell).
#
# Asserts that skill-lint:
#   1. passes on the checked-in .claude/skills/ (including _template.md);
#   2. fails, naming the section, on a fixture with a deleted section;
#   3. fails, naming the path, on a fixture with a dangling file path;
#   4. fails on a bullet with no backticked path;
#   5. fails on an empty skills directory.
# Finally runs shellcheck over both scripts when shellcheck is installed.
#
# Usage: scripts/skill-lint-smoke.sh
# Exit status: 0 when all checks pass, 1 otherwise.

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${repo_root}" ]]; then
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd "${script_dir}/.." && pwd)"
fi

lint="${repo_root}/scripts/skill-lint.sh"
template="${repo_root}/.claude/skills/_template.md"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/skill-lint-smoke.XXXXXX")"
trap 'rm -rf "${tmp}"' EXIT

checks=0
failed=0

# expect_pass <description> <arg...>
expect_pass() {
  local desc="$1"
  shift
  checks=$((checks + 1))
  local out
  if out="$("${lint}" "$@" 2>&1)"; then
    echo "ok: ${desc}"
  else
    failed=$((failed + 1))
    echo "FAILED: ${desc} — expected pass, got failure:" >&2
    echo "${out}" >&2
  fi
}

# expect_fail <description> <expected-substring> <arg...>
expect_fail() {
  local desc="$1" want="$2"
  shift 2
  checks=$((checks + 1))
  local out
  if out="$("${lint}" "$@" 2>&1)"; then
    failed=$((failed + 1))
    echo "FAILED: ${desc} — expected failure, but lint passed:" >&2
    echo "${out}" >&2
  elif [[ "${out}" != *"${want}"* ]]; then
    failed=$((failed + 1))
    echo "FAILED: ${desc} — failed as expected but message lacks '${want}':" >&2
    echo "${out}" >&2
  else
    echo "ok: ${desc}"
  fi
}

# 1. The real skills directory (template included) lints clean.
expect_pass "checked-in .claude/skills/ passes"

# 2. Deleting a required section fails loudly, naming the section.
mkdir -p "${tmp}/missing-section"
grep -v '^## Pattern$' "${template}" >"${tmp}/missing-section/broken.md"
expect_fail "missing '## Pattern' section fails" \
  "missing required section '## Pattern'" "${tmp}/missing-section"

# 3. A dangling "Files involved" path fails loudly, naming the path.
mkdir -p "${tmp}/dangling-path"
# Literal backticks in the sed patterns below — not command substitution.
# shellcheck disable=SC2016
sed 's|`.claude/skills/README.md`|`crates/does-not-exist/src/lib.rs`|' \
  "${template}" >"${tmp}/dangling-path/broken.md"
expect_fail "dangling file path fails" \
  "path does not exist: crates/does-not-exist/src/lib.rs" "${tmp}/dangling-path"

# 4. A bullet without a backticked path fails loudly.
mkdir -p "${tmp}/bare-bullet"
# shellcheck disable=SC2016
sed 's|- `mk/skills.mk` — make targets.*|- mk/skills.mk with no backticks|' \
  "${template}" >"${tmp}/bare-bullet/broken.md"
expect_fail "bullet without backticked path fails" \
  "must start with a backticked repo-relative path" "${tmp}/bare-bullet"

# 5. An empty skills directory fails loudly.
mkdir -p "${tmp}/empty"
expect_fail "empty skills directory fails" \
  "no skill files found" "${tmp}/empty"

# Run shellcheck over the lint tooling itself, when available.
if command -v shellcheck >/dev/null 2>&1; then
  checks=$((checks + 1))
  if shellcheck "${lint}" "${repo_root}/scripts/skill-lint-smoke.sh"; then
    echo "ok: shellcheck clean"
  else
    failed=$((failed + 1))
    echo "FAILED: shellcheck reported issues" >&2
  fi
else
  echo "skip: shellcheck not installed (checked in CI)"
fi

if [[ ${failed} -gt 0 ]]; then
  echo "skill-lint-smoke: ${failed}/${checks} check(s) FAILED" >&2
  exit 1
fi
echo "skill-lint-smoke: OK — ${checks} check(s) passed"
