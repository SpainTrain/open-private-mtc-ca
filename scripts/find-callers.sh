#!/usr/bin/env bash
# find-callers.sh — find call sites for a symbol (spec §23.6).
#
# Usage: scripts/find-callers.sh (via `make find-callers symbol=Y`)
#
# Searches for all call sites of a given symbol using ripgrep.
# Emits file:line-prefixed output.

set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# Extract the symbol parameter from environment or positional args.
symbol="${symbol:-${1:-}}"

if [ -z "$symbol" ]; then
  echo "usage: make find-callers symbol=SYMBOL" >&2
  echo "       finds all call sites for SYMBOL" >&2
  exit 1
fi

# Locate rg (ripgrep) in common places.
if command -v rg > /dev/null 2>&1; then
  RG=rg
elif [ -x /home/spain/.gemini/tmp/bin/rg ]; then
  RG=/home/spain/.gemini/tmp/bin/rg
else
  echo "error: ripgrep (rg) not found; install with: cargo install ripgrep" >&2
  exit 1
fi

# Escape special regex characters for rg.
escaped_symbol=$(printf '%s\n' "$symbol" | sed 's/[[\.*^$/]/\\&/g')

# Search for function/method calls: symbol(...) or symbol::method()
# Also catch method calls on the result.
$RG -n "$escaped_symbol\s*\(" crates/ || true
