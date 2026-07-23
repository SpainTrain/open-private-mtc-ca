#!/usr/bin/env bash
# adr-new.sh — scaffold the next-numbered ADR from the template and add an
# index row to docs/adr/README.md. Spec: §23.5 (ADRs), §23.6 (ADR index).
#
# Usage: scripts/adr-new.sh "Title of the decision"
# Normally invoked as: make adr title="Title of the decision"
#
# ADR_DIR may be overridden (used by the smoke tests); it defaults to
# <repo-root>/docs/adr.
set -euo pipefail

usage() {
  echo "usage: $0 \"Title of the decision\"" >&2
  echo "       (or: make adr title=\"Title of the decision\")" >&2
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
adr_dir="${ADR_DIR:-${repo_root}/docs/adr}"
template="${adr_dir}/_template.md"
index="${adr_dir}/README.md"
begin_marker='<!-- adr-index-begin -->'
end_marker='<!-- adr-index-end -->'

title="${1:-}"
if [ "$#" -ne 1 ] || [ -z "${title// /}" ]; then
  echo "error: missing ADR title" >&2
  usage
  exit 1
fi

for f in "$template" "$index"; do
  if [ ! -f "$f" ]; then
    echo "error: required file not found: $f" >&2
    exit 1
  fi
done
for marker in "$begin_marker" "$end_marker"; do
  if ! grep -qF "$marker" "$index"; then
    echo "error: index marker '$marker' not found in $index" >&2
    exit 1
  fi
done

# Next sequential four-digit number: max existing + 1, starting at 0001.
max=0
for f in "$adr_dir"/[0-9][0-9][0-9][0-9]-*.md; do
  [ -e "$f" ] || continue
  n="${f##*/}"
  n="${n%%-*}"
  n=$((10#$n))
  if [ "$n" -gt "$max" ]; then max="$n"; fi
done
num="$(printf '%04d' $((max + 1)))"

# Kebab-case slug from the title.
slug="$(printf '%s' "$title" \
  | tr '[:upper:]' '[:lower:]' \
  | tr -cs 'a-z0-9' '-' \
  | sed -e 's/^-*//' -e 's/-*$//')"
if [ -z "$slug" ]; then
  echo "error: title produced an empty slug: '$title'" >&2
  exit 1
fi

adr_file="${adr_dir}/${num}-${slug}.md"
if [ -e "$adr_file" ]; then
  echo "error: refusing to overwrite existing file: $adr_file" >&2
  exit 1
fi

# Build the updated index in a temp file first so a failure leaves the
# index untouched (no half-applied state).
row_title="${title//|/\\|}"
row="| [ADR-${num}](${num}-${slug}.md) | ${row_title} | Proposed | TODO: one-line summary |"
tmp_index="$(mktemp "${index}.XXXXXX")"
trap 'rm -f "$tmp_index"' EXIT
awk -v row="$row" -v marker="$end_marker" '
  index($0, marker) { print row }
  { print }
' "$index" >"$tmp_index"

# Instantiate the template. Bash ${var//pat/rep} keeps titles with sed/awk
# metacharacters safe.
content="$(<"$template")"
content="${content//ADR-NNNN: TITLE/ADR-${num}: ${title}}"
content="${content//YYYY-MM-DD/$(date +%F)}"
printf '%s\n' "$content" >"$adr_file"

mv -f "$tmp_index" "$index"
trap - EXIT

echo "created ${adr_file}"
echo "indexed in ${index} (status Proposed; fill in the ADR, then update its status and summary row)"
