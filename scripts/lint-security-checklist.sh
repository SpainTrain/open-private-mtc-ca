#!/usr/bin/env bash
# Lint the Phase 8 security review checklist (docs/security/review-checklist.md).
#
# Enforces the machine-checkable contract of ticket mtc-ve8j
# (ops-security-review-checklist):
#   - every checklist row has six non-empty cells:
#       ID | Statement | Spec | Verify | Evidence | Status
#   - Status is one of: unreviewed | pass | finding | N.A.
#   - Verify is one of: test | code inspection | config check | n/a (scoped out)
#   - every item cites at least one spec section (a `§` reference)
#   - IDs are unique
#   - all eight §19.8 adversarial scenarios (SEC-ADV-01..08) are present, and
#     each SEC-ADV row's Evidence is a test pointer (backtick/path) or an
#     explicit "GAP:" marker
#
# Usage: lint-security-checklist.sh [path-to-checklist.md]
# Exit code 0 iff the checklist passes all checks.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CHECKLIST="${1:-${REPO_ROOT}/docs/security/review-checklist.md}"

if [[ ! -f "${CHECKLIST}" ]]; then
    echo "ERROR: checklist not found: ${CHECKLIST}" >&2
    exit 1
fi

awk '
function trim(s) {
    gsub(/^[[:space:]]+/, "", s)
    gsub(/[[:space:]]+$/, "", s)
    return s
}

function err(msg) {
    printf "ERROR: %s:%d: %s\n", FILENAME, FNR, msg > "/dev/stderr"
    failed = 1
}

BEGIN {
    FS = "|"
    valid_status["unreviewed"] = 1
    valid_status["pass"] = 1
    valid_status["finding"] = 1
    valid_status["N.A."] = 1
    valid_verify["test"] = 1
    valid_verify["code inspection"] = 1
    valid_verify["config check"] = 1
    valid_verify["n/a (scoped out)"] = 1
    n_required_adv = 8
}

# Checklist item rows: markdown table rows whose first cell is a SEC id.
/^\|[[:space:]]*SEC-[A-Z]+-[0-9]+[[:space:]]*\|/ {
    # A six-column row "| a | b | c | d | e | f |" splits into 8 fields
    # (leading and trailing empties).
    if (NF != 8) {
        err("row has " (NF - 2) " cells, expected 6 (ID, Statement, Spec, Verify, Evidence, Status)")
        next
    }

    id = trim($2); stmt = trim($3); spec = trim($4)
    verify = trim($5); evidence = trim($6); status = trim($7)
    total++

    if (id in seen) {
        err("duplicate id " id " (first seen at line " seen[id] ")")
    } else {
        seen[id] = FNR
    }

    if (stmt == "" || stmt == "—") err(id ": empty Statement cell")
    if (spec !~ /§/) err(id ": Spec cell must cite at least one spec section (§N)")
    if (!(verify in valid_verify)) {
        err(id ": invalid Verify value \"" verify "\" (allowed: test, code inspection, config check, n/a (scoped out))")
    }
    if (evidence == "") err(id ": empty Evidence cell (use — if pending review)")
    if (status == "") {
        err(id ": missing Status")
    } else if (!(status in valid_status)) {
        err(id ": invalid Status \"" status "\" (allowed: unreviewed, pass, finding, N.A.)")
    } else {
        by_status[status]++
    }

    if (id ~ /^SEC-ADV-/) {
        # §19.8 scenarios must point at a covering test or carry an explicit
        # gap marker.
        if (evidence !~ /^(GAP:|`)/) {
            err(id ": Evidence must be a test pointer (backticked path) or start with \"GAP:\"")
        }
    }
}

END {
    for (i = 1; i <= n_required_adv; i++) {
        req = sprintf("SEC-ADV-%02d", i)
        if (!(req in seen)) {
            printf "ERROR: %s: missing required §19.8 adversarial scenario item %s\n", FILENAME, req > "/dev/stderr"
            failed = 1
        }
    }
    if (total == 0) {
        printf "ERROR: %s: no checklist items found (expected rows starting with \"| SEC-\")\n", FILENAME > "/dev/stderr"
        failed = 1
    }
    if (failed) exit 1
    printf "OK: %d checklist items validated", total
    sep = " ("
    for (s in by_status) { printf "%s%s: %d", sep, s, by_status[s]; sep = ", " }
    print ")"
}
' "${CHECKLIST}"
