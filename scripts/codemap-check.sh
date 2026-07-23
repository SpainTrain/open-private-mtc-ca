#!/usr/bin/env bash
# codemap-check.sh — fail if regenerating CODEMAP.md would change the
# committed file (spec §23.6; `make codemap-check`).
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
committed="$repo_root/CODEMAP.md"

if [ ! -f "$committed" ]; then
  printf 'codemap-check: FAIL: %s does not exist — run `make codemap`\n' "$committed" >&2
  exit 1
fi

fresh="$(mktemp)"
trap 'rm -f "$fresh"' EXIT

"$script_dir/codemap-gen.sh" > "$fresh"

if diff -u "$committed" "$fresh" > /dev/null 2>&1; then
  printf 'codemap-check: PASS — CODEMAP.md is up to date\n'
  exit 0
fi

printf 'codemap-check: FAIL — CODEMAP.md is stale; regenerate with `make codemap`\n' >&2
diff -u "$committed" "$fresh" >&2 || true
exit 1
