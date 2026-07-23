#!/usr/bin/env bash
# find-impl.sh — find implementations of a trait (spec §23.6).
#
# Usage: scripts/find-impl.sh (via `make find-impl iface=Z`)
#
# Searches for all implementations of a given trait using ripgrep.
# Emits file:line-prefixed output.

set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# Extract the iface parameter from environment or positional args.
iface="${iface:-${1:-}}"

if [ -z "$iface" ]; then
  echo "usage: make find-impl iface=TRAIT" >&2
  echo "       finds all implementations of TRAIT" >&2
  exit 1
fi

# Locate rg (ripgrep).
if ! command -v rg > /dev/null 2>&1; then
  echo "error: ripgrep (rg) not found; install with: cargo install ripgrep" >&2
  exit 1
fi

# Escape special regex characters for rg.
escaped_iface=$(printf '%s\n' "$iface" | sed 's/[[\.*^$/]/\\&/g')

# Search for impl blocks: impl Trait or impl<T> Trait or impl Trait for Type
# Use -H to ensure file:line prefix.
rg -H -n "impl\s+[^{]*$escaped_iface" crates/ || true
