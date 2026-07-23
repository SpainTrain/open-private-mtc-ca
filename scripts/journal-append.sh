#!/usr/bin/env bash
# journal-append.sh — append a timestamped entry to the decision journal
# (docs/journal.md), in the spec §23.7 template form.
#
# Usage:
#   scripts/journal-append.sh "message"        # message as a single argument
#   msg="message" scripts/journal-append.sh    # message via the environment
#   make journal msg="message"                 # via make (see mk/journal.mk)
#
# Optional environment (or make variables, which GNU make auto-exports):
#   ticket=...          value for the "**Ticket**:" line (default: —)
#   pr=...              value for the "**PR**:" line (default: —)
#   JOURNAL_FILE=path   target file (default: <repo root>/docs/journal.md)
#   JOURNAL_DATE=date   override the YYYY-MM-DD stamp (tests only)
#
# Message conventions:
#   - The first line of the message becomes the entry title:
#       ## YYYY-MM-DD — <title>
#   - Any further lines become the entry body verbatim; supply your own
#     "Decisions:" / "Open questions:" sections per the template.
#   - A single-line message gets a minimal template body generated for it.
#
# All validation happens before any write: a bad invocation exits non-zero
# and leaves the journal untouched.

set -euo pipefail

err() {
  printf 'journal-append: error: %s\n' "$1" >&2
  exit 1
}

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
journal_file=${JOURNAL_FILE:-$repo_root/docs/journal.md}

# Message: positional argument wins, then $msg from the environment.
message=${1:-${msg:-}}

# Reject missing / empty / whitespace-only messages before touching anything.
if [ -z "$(printf '%s' "$message" | tr -d '[:space:]')" ]; then
  err 'msg is required and must be non-empty. Usage: make journal msg="..."'
fi

[ -f "$journal_file" ] || err "journal file not found: $journal_file"
[ -w "$journal_file" ] || err "journal file not writable: $journal_file"

date_stamp=${JOURNAL_DATE:-$(date -u +%Y-%m-%d)}
case $date_stamp in
  [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]) ;;
  *) err "invalid JOURNAL_DATE (want YYYY-MM-DD): $date_stamp" ;;
esac

# Split message: first line -> title, remainder -> body.
title=${message%%$'\n'*}
body=''
if [ "$title" != "$message" ]; then
  body=${message#*$'\n'}
  # Trim leading blank lines from the body.
  while [ "${body:0:1}" = $'\n' ]; do
    body=${body:1}
  done
fi

entry="## ${date_stamp} — ${title}

**Ticket**: ${ticket:-—}
**PR**: ${pr:-—}
"

if [ -n "$(printf '%s' "$body" | tr -d '[:space:]')" ]; then
  entry+="
${body}
"
else
  entry+="
Decisions:
- ${title}

Open questions:
- (none)
"
fi

# Append (entries are chronological, oldest first; new entries go at the end).
# Ensure the file ends with a newline, then add a blank-line separator.
if [ -s "$journal_file" ] && [ -n "$(tail -c 1 "$journal_file")" ]; then
  printf '\n' >> "$journal_file"
fi
printf '\n%s' "$entry" >> "$journal_file"

printf 'journal-append: appended "## %s — %s" to %s\n' \
  "$date_stamp" "$title" "$journal_file"
