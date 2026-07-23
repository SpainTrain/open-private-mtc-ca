#!/usr/bin/env bash
# recent-journal.sh — print the most recent entries of the decision journal
# (docs/journal.md). Entries are dated `## YYYY-MM-DD — title` headings (the
# spec §23.7 template), chronological oldest-first, so "recent" means the tail
# of the file. Non-entry `##` headings (the Conventions preamble) and headings
# inside fenced code blocks (the template example) are not counted as entries.
#
# Usage:
#   scripts/recent-journal.sh [N]     # last N entries (default: 3)
#
# Environment:
#   JOURNAL_FILE=path   journal to read (default: <repo root>/docs/journal.md)
#
# Used by `make agent-precheck` (spec §23.4) to surface recent decisions
# before a task starts; also usable standalone.

set -euo pipefail

err() {
  printf 'recent-journal: error: %s\n' "$1" >&2
  exit 2
}

n=${1:-3}
case $n in
  '' | *[!0-9]*) err "N must be a positive integer, got: $n" ;;
esac
[ "$n" -ge 1 ] || err "N must be >= 1, got: $n"

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
journal_file=${JOURNAL_FILE:-$repo_root/docs/journal.md}

[ -f "$journal_file" ] || err "journal file not found: $journal_file"

# Line number of the N-th-from-last dated entry heading (or the earliest one,
# when the journal has fewer than N entries). Headings inside ``` fences are
# ignored so the preamble's template example is not mistaken for an entry.
start=$(awk '
  /^```/ { fence = !fence; next }
  !fence && /^## [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9] / { print NR }
' "$journal_file" | tail -n "$n" | head -n 1 || true)

if [ -z "$start" ]; then
  printf '(journal has no entries yet — see %s)\n' "$journal_file"
else
  tail -n +"$start" "$journal_file"
fi
