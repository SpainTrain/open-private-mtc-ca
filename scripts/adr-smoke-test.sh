#!/usr/bin/env bash
# adr-smoke-test.sh — script smoke tests for scripts/adr-new.sh (§19.13 spirit).
# Runs against a throwaway sandbox copy of docs/adr; never touches the repo's
# real ADRs. Exits non-zero on the first failing assertion.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
adr_new="${repo_root}/scripts/adr-new.sh"

sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT
cp -f "${repo_root}/docs/adr/_template.md" "$sandbox/"
cat >"${sandbox}/README.md" <<'EOF'
# ADR index (test fixture)

<!-- adr-index-begin -->
| ADR | Title | Status | Summary |
|-----|-------|--------|---------|
<!-- adr-index-end -->
EOF

pass=0
fail() {
  echo "FAIL: $1" >&2
  exit 1
}
ok() {
  pass=$((pass + 1))
  echo "ok: $1"
}

# 1. First scaffold gets number 0001 and an index row.
ADR_DIR="$sandbox" "$adr_new" "First decision" >/dev/null
[ -f "${sandbox}/0001-first-decision.md" ] || fail "0001-first-decision.md not created"
grep -q '^# ADR-0001: First decision$' "${sandbox}/0001-first-decision.md" \
  || fail "title not substituted into ADR-0001"
grep -q "$(date +%F)" "${sandbox}/0001-first-decision.md" \
  || fail "date not substituted into ADR-0001"
grep -qF '| [ADR-0001](0001-first-decision.md) | First decision | Proposed |' "${sandbox}/README.md" \
  || fail "index row for ADR-0001 missing"
ok "scaffolds ADR-0001 with index row"

# 2. Second scaffold is sequentially numbered; slug handles punctuation.
ADR_DIR="$sandbox" "$adr_new" "Second Decision, with punctuation!" >/dev/null
[ -f "${sandbox}/0002-second-decision-with-punctuation.md" ] \
  || fail "0002 not created with kebab-case slug"
grep -qF '[ADR-0002](0002-second-decision-with-punctuation.md)' "${sandbox}/README.md" \
  || fail "index row for ADR-0002 missing"
ok "sequential numbering and slugging"

# 3. Index row lands inside the markers, ADR-0002 after ADR-0001.
awk '/adr-index-begin/,/adr-index-end/' "${sandbox}/README.md" \
  | grep -q 'ADR-0002' || fail "ADR-0002 row not inside index markers"
[ "$(grep -n 'ADR-0001' "${sandbox}/README.md" | head -1 | cut -d: -f1)" \
  -lt "$(grep -n 'ADR-0002' "${sandbox}/README.md" | head -1 | cut -d: -f1)" ] \
  || fail "index rows out of order"
ok "index rows ordered inside markers"

# 4. Missing title fails and changes nothing.
before="$(cat "${sandbox}/README.md")"
if ADR_DIR="$sandbox" "$adr_new" 2>/dev/null; then
  fail "missing title should exit non-zero"
fi
[ "$before" = "$(cat "${sandbox}/README.md")" ] || fail "failed run modified the index"
if compgen -G "${sandbox}/0003-*" >/dev/null; then
  fail "failed run created a file"
fi
ok "missing title: clean failure, no changes"

# 5. Broken index (no markers) fails before creating the ADR file.
sed -i 's/<!-- adr-index-end -->//' "${sandbox}/README.md"
if ADR_DIR="$sandbox" "$adr_new" "Third decision" 2>/dev/null; then
  fail "missing marker should exit non-zero"
fi
[ ! -e "${sandbox}/0003-third-decision.md" ] || fail "marker failure still created a file"
ok "missing index marker: clean failure, no ADR file"

echo "adr smoke tests passed (${pass} checks)"
