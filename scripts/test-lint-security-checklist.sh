#!/usr/bin/env bash
# Unit tests for scripts/lint-security-checklist.sh (ticket mtc-ve8j).
#
# Builds fixture checklists in a temp dir and asserts the linter accepts valid
# input and rejects each class of invalid input with a matching error message.
# Also asserts the real checklist at docs/security/review-checklist.md passes.
#
# Usage: test-lint-security-checklist.sh
# Exit code 0 iff all tests pass.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
LINT="${SCRIPT_DIR}/lint-security-checklist.sh"

TMPDIR_TESTS="$(mktemp -d)"
trap 'rm -rf "${TMPDIR_TESTS}"' EXIT

pass_count=0
fail_count=0

# assert_lint <expected-exit: ok|fail> <test-name> <fixture-file> [expected-stderr-substring]
assert_lint() {
    local expected="$1" name="$2" fixture="$3" want_msg="${4:-}"
    local out err rc=0
    out="$("${LINT}" "${fixture}" 2>"${TMPDIR_TESTS}/stderr")" || rc=$?
    err="$(cat "${TMPDIR_TESTS}/stderr")"

    local ok=1
    if [[ "${expected}" == "ok" && ${rc} -ne 0 ]]; then ok=0; fi
    if [[ "${expected}" == "fail" && ${rc} -eq 0 ]]; then ok=0; fi
    if [[ -n "${want_msg}" && "${err}" != *"${want_msg}"* ]]; then ok=0; fi

    if [[ ${ok} -eq 1 ]]; then
        echo "PASS: ${name}"
        pass_count=$((pass_count + 1))
    else
        echo "FAIL: ${name} (rc=${rc}, expected ${expected}${want_msg:+, wanted stderr containing: ${want_msg}})"
        [[ -n "${err}" ]] && echo "${err}" | sed 's/^/  stderr: /'
        [[ -n "${out}" ]] && echo "${out}" | sed 's/^/  stdout: /'
        fail_count=$((fail_count + 1))
    fi
}

# --- Fixtures -----------------------------------------------------------------

VALID="${TMPDIR_TESTS}/valid.md"
cat > "${VALID}" <<'EOF'
# Fixture checklist

| ID | Statement | Spec | Verify | Evidence | Status |
|---|---|---|---|---|---|
| SEC-KH-01 | Keys never leave the HSM boundary. | §14.1 | code inspection | — | unreviewed |
| SEC-AO-01 | Log objects are immutable once written. | §8.1 | config check | — | pass |
| SEC-OOS-01 | Cosigners not applicable: single trust boundary. | §1, §3 | n/a (scoped out) | — | N.A. |
| SEC-ADV-01 | Malformed tiles rejected. | §19.8 | test | GAP: planned fuzz target | unreviewed |
| SEC-ADV-02 | Stalled CRR blocks promotion. | §19.8 | test | GAP: planned chaos-crr-stall | unreviewed |
| SEC-ADV-03 | Bad HSM signatures detected. | §19.8 | test | `tests/adversarial/hsm_bad_sig.rs` | finding |
| SEC-ADV-04 | Clock skew handled. | §19.8 | test | GAP: planned chaos-clock-skew | unreviewed |
| SEC-ADV-05 | Duplicate index allocation impossible. | §19.8 | test | GAP: planned Kani proof | unreviewed |
| SEC-ADV-06 | Stale-epoch writes rejected. | §19.8 | test | GAP: planned chaos-split-brain | unreviewed |
| SEC-ADV-07 | Replayed checkpoints detected. | §19.8 | test | GAP: planned self-auditor test | unreviewed |
| SEC-ADV-08 | Key-hash collisions ineffective. | §19.8 | test | GAP: planned invariant test | unreviewed |
EOF

mutate() { # mutate <name> <sed-expr> -> prints fixture path
    local name="$1" expr="$2"
    local f="${TMPDIR_TESTS}/${name}.md"
    sed "${expr}" "${VALID}" > "${f}"
    echo "${f}"
}

# --- Tests --------------------------------------------------------------------

assert_lint ok "valid fixture passes" "${VALID}"

assert_lint fail "missing file fails" "${TMPDIR_TESTS}/does-not-exist.md" "checklist not found"

f="$(mutate empty-status 's/| SEC-KH-01 \(.*\)| unreviewed |/| SEC-KH-01 \1|  |/')"
assert_lint fail "empty status fails" "${f}" "SEC-KH-01: missing Status"

f="$(mutate bad-status 's/| SEC-AO-01 \(.*\)| pass |/| SEC-AO-01 \1| done |/')"
assert_lint fail "invalid status value fails" "${f}" 'invalid Status "done"'

f="$(mutate bad-verify 's/| code inspection | — | unreviewed |/| vibes | — | unreviewed |/')"
assert_lint fail "invalid verification method fails" "${f}" 'invalid Verify value "vibes"'

f="$(mutate dup-id 's/SEC-AO-01/SEC-KH-01/')"
assert_lint fail "duplicate id fails" "${f}" "duplicate id SEC-KH-01"

f="$(mutate missing-adv '/^| SEC-ADV-05 /d')"
assert_lint fail "missing §19.8 scenario fails" "${f}" "missing required §19.8 adversarial scenario item SEC-ADV-05"

f="$(mutate adv-no-pointer 's/| GAP: planned fuzz target |/| will get to it |/')"
assert_lint fail "ADV row without test pointer or GAP marker fails" "${f}" 'SEC-ADV-01: Evidence must be a test pointer'

f="$(mutate no-spec-cite 's/| §14.1 |/| n\/a |/')"
assert_lint fail "missing spec citation fails" "${f}" "SEC-KH-01: Spec cell must cite"

f="$(mutate wrong-cols 's/| SEC-KH-01 | Keys never leave the HSM boundary. |/| SEC-KH-01 |/')"
assert_lint fail "wrong column count fails" "${f}" "expected 6"

printf '# empty\nno table here\n' > "${TMPDIR_TESTS}/no-items.md"
assert_lint fail "checklist with no items fails" "${TMPDIR_TESTS}/no-items.md" "no checklist items found"

assert_lint ok "real checklist passes" "${REPO_ROOT}/docs/security/review-checklist.md"

# --- Summary ------------------------------------------------------------------

echo
echo "${pass_count} passed, ${fail_count} failed"
[[ ${fail_count} -eq 0 ]]
