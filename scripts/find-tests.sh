#!/usr/bin/env bash
# find-tests.sh — find test files/functions relevant to a path (spec §23.6).
#
# Usage: scripts/find-tests.sh (via `make find-tests path=X`)
#
# Searches for test functions and test files related to a given path using
# ripgrep heuristics. Emits file:line-prefixed output.

set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# Extract the path parameter from environment or positional args.
path="${path:-${1:-}}"

if [ -z "$path" ]; then
  echo "usage: make find-tests path=PATH" >&2
  echo "       finds test functions and test files related to PATH" >&2
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

# If path matches a file, search in that file; otherwise treat as a module/crate name.
if [ -f "$path" ]; then
  # Emit all test functions in the file
  $RG -n '^\s*#\[(tokio::)?test\]' "$path"
else
  # Search for test files that match the module name
  $RG -l '#\[(tokio::)?test\]' crates/ | $RG "$path"
  # Also search for test functions that mention the path
  $RG -n '#\[(tokio::)?test\]' crates/ | $RG "$path"
fi
