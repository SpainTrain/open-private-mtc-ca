#!/usr/bin/env sh
# Lint .claude/rules/: every rule file must contain the required sections
# (rule, rationale, compliant + non-compliant example, enforcement) per the
# agent-claude-rules-seed acceptance criteria. Invoked via `make rules-lint`.
set -eu

rules_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
required_headings='## Rule
## Rationale
## Compliant example
## Non-compliant example
## Enforcement'

expected_count=16
fail=0
count=0

for f in "$rules_dir"/*.md; do
    base="$(basename "$f")"
    [ "$base" = "README.md" ] && continue
    count=$((count + 1))

    # Every rule file starts with a title heading.
    if ! head -n 1 "$f" | grep -q '^# '; then
        echo "FAIL: $base: missing top-level '# <rule-name>' title" >&2
        fail=1
    fi

    printf '%s\n' "$required_headings" | while IFS= read -r heading; do
        if ! grep -qx "$heading" "$f"; then
            echo "FAIL: $base: missing required heading '$heading'" >&2
            echo "$base $heading" >> "$rules_dir/.lint-failures"
        fi
    done
done

if [ -f "$rules_dir/.lint-failures" ]; then
    rm -f "$rules_dir/.lint-failures"
    fail=1
fi

if [ "$count" -ne "$expected_count" ]; then
    echo "FAIL: expected $expected_count rule files, found $count" >&2
    fail=1
fi

# Basic markdown lint if a linter is available; heading check above is the gate.
if command -v markdownlint >/dev/null 2>&1; then
    markdownlint "$rules_dir"/*.md || fail=1
elif command -v mdl >/dev/null 2>&1; then
    mdl "$rules_dir"/*.md || fail=1
else
    echo "note: no markdown linter found (markdownlint/mdl); skipped"
fi

if [ "$fail" -ne 0 ]; then
    echo "rules-lint: FAILED" >&2
    exit 1
fi
echo "rules-lint: OK ($count rule files, all required headings present)"
