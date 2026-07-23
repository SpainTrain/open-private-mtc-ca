#!/usr/bin/env bash
# find-todo.sh — find TODO and FIXME comments (spec §23.6).
#
# Usage: scripts/find-todo.sh (via `make find-todo`)
#
# Searches for all TODO and FIXME comments in the codebase using ripgrep.
# Emits file:line-prefixed output.

set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# Locate rg (ripgrep) in common places.
if command -v rg > /dev/null 2>&1; then
  RG=rg
elif [ -x /home/spain/.gemini/tmp/bin/rg ]; then
  RG=/home/spain/.gemini/tmp/bin/rg
else
  echo "error: ripgrep (rg) not found; install with: cargo install ripgrep" >&2
  exit 1
fi

# Search for TODO and FIXME comments in Rust source files.
# Also search in Makefiles, shell scripts, and markdown docs.
$RG -n '\b(TODO|FIXME)\b' crates/ mk/ scripts/ docs/ || true
