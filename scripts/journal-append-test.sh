#!/usr/bin/env bash
# journal-append-test.sh — smoke tests for scripts/journal-append.sh
# (spec §19.13 spirit: append twice and assert ordering + timestamp format;
# malformed invocations leave the file unchanged; shellcheck if installed).

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
append=$script_dir/journal-append.sh
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT
journal=$tmpdir/journal.md

fail() {
  printf 'journal-append-test: FAIL: %s\n' "$1" >&2
  exit 1
}

# Make sure ambient variables cannot leak into the tests.
unset msg ticket pr JOURNAL_DATE 2>/dev/null || true
checks=0

printf '# Test journal\n' > "$journal"

# 1. Two appends land in chronological (append) order with the expected
#    "## YYYY-MM-DD — title" heading format.
JOURNAL_FILE=$journal "$append" 'first entry' > /dev/null
JOURNAL_FILE=$journal JOURNAL_DATE=2099-01-02 "$append" 'second entry' > /dev/null
grep -Eq '^## [0-9]{4}-[0-9]{2}-[0-9]{2} — first entry$' "$journal" \
  || fail 'first entry heading/timestamp format wrong'
grep -Fxq '## 2099-01-02 — second entry' "$journal" \
  || fail 'second entry heading wrong (JOURNAL_DATE override)'
first_line=$(grep -nF ' — first entry' "$journal" | head -n1 | cut -d: -f1)
second_line=$(grep -nF ' — second entry' "$journal" | head -n1 | cut -d: -f1)
[ "$first_line" -lt "$second_line" ] \
  || fail 'entries not in chronological append order (oldest first)'
checks=$((checks + 2))

# 2. Template metadata lines are present; ticket/pr propagate.
JOURNAL_FILE=$journal ticket='TEST-1' pr='#7' "$append" 'metadata entry' > /dev/null
grep -Fxq '**Ticket**: TEST-1' "$journal" || fail 'ticket value not propagated'
grep -Fxq '**PR**: #7' "$journal" || fail 'pr value not propagated'
grep -Fxq '**Ticket**: —' "$journal" || fail 'default Ticket line missing'
checks=$((checks + 1))

# 3. Quoting and multiline bodies survive verbatim.
JOURNAL_FILE=$journal "$append" 'Tricky "quoted" title
Decisions:
- kept `backticks`, $dollars, and '\''single quotes'\'' intact

Open questions:
- (none)' > /dev/null
grep -Fq 'Tricky "quoted" title' "$journal" || fail 'quoted title mangled'
grep -Fxq -e '- kept `backticks`, $dollars, and '\''single quotes'\'' intact' "$journal" \
  || fail 'multiline body mangled'
checks=$((checks + 1))

# 4. Missing / empty / whitespace-only msg errors and leaves the file untouched.
before=$(cat "$journal")
if JOURNAL_FILE=$journal "$append" > /dev/null 2>&1; then
  fail 'missing msg was accepted'
fi
if JOURNAL_FILE=$journal "$append" '' > /dev/null 2>&1; then
  fail 'empty msg was accepted'
fi
if JOURNAL_FILE=$journal "$append" '   ' > /dev/null 2>&1; then
  fail 'whitespace-only msg was accepted'
fi
if JOURNAL_FILE=$tmpdir/does-not-exist.md "$append" 'orphan' > /dev/null 2>&1; then
  fail 'missing journal file was accepted'
fi
[ "$before" = "$(cat "$journal")" ] \
  || fail 'rejected invocation modified the journal'
checks=$((checks + 1))

# 5. Error message is clear (mentions msg and usage).
errout=$(JOURNAL_FILE=$journal "$append" 2>&1 || true)
printf '%s' "$errout" | grep -q 'msg is required' || fail 'error message unclear'
checks=$((checks + 1))

# 6. shellcheck, when available.
if command -v shellcheck > /dev/null 2>&1; then
  shellcheck "$append" "${BASH_SOURCE[0]}" || fail 'shellcheck reported issues'
  checks=$((checks + 1))
else
  printf 'journal-append-test: shellcheck not installed; skipping lint check\n'
fi

printf 'journal-append-test: OK (%d checks passed)\n' "$checks"
