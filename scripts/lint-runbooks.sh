#!/usr/bin/env bash
# lint-runbooks.sh — structure lint for docs/runbooks/ (spec §21.2).
#
# Enforces:
#   * TEMPLATE.md, POSTMORTEM.md, and README.md exist.
#   * Every runbook (all *.md except README.md/POSTMORTEM.md, plus
#     TEMPLATE.md itself) starts with a '# ' title and contains the five
#     mandated '## ' sections — Detection, Initial assessment, Mitigation
#     steps, Recovery procedure, Postmortem template — exactly once each,
#     in that order.
#   * POSTMORTEM.md contains Timeline, Impact, Root cause, Action items.
#   * README.md indexes each of the eight required runbooks (§21.2) with a
#     status of 'stub' or 'complete', indexes every runbook file present,
#     and contains no dead relative .md links.
#
# Usage: lint-runbooks.sh [RUNBOOKS_DIR]   (default: docs/runbooks)
# Exits non-zero if any check fails.
set -euo pipefail

RUNBOOKS_DIR="${1:-docs/runbooks}"

RUNBOOK_SECTIONS=(
  "Detection"
  "Initial assessment"
  "Mitigation steps"
  "Recovery procedure"
  "Postmortem template"
)
POSTMORTEM_SECTIONS=(
  "Timeline"
  "Impact"
  "Root cause"
  "Action items"
)
# The eight required runbooks, spec §21.2.
REQUIRED_RUNBOOKS=(
  primary-failure
  hsm-unavailability
  crr-stall
  self-auditor-anomaly
  emergency-revocation
  pruning-failure
  suspected-key-compromise
  adapter-flood
)

errors=0
err() {
  echo "lint-runbooks: ERROR: $*" >&2
  errors=$((errors + 1))
}

if [[ ! -d "$RUNBOOKS_DIR" ]]; then
  echo "lint-runbooks: ERROR: runbooks directory not found: $RUNBOOKS_DIR" >&2
  exit 1
fi

for special in TEMPLATE.md POSTMORTEM.md README.md; do
  [[ -f "$RUNBOOKS_DIR/$special" ]] || err "$RUNBOOKS_DIR/$special is missing"
done

# --- helpers ---------------------------------------------------------------

check_title() {
  local file="$1" first
  first="$(grep -m1 -v '^[[:space:]]*$' "$file" || true)"
  if [[ "$first" != "# "* ]]; then
    err "$file: first non-blank line must be a '# ' title heading"
  fi
}

# check_sections FILE SECTION...
# Requires each SECTION as a '## ' heading exactly once, in the given order.
check_sections() {
  local file="$1"
  shift
  local expected=("$@") found=() sec heading count all_present_once=1

  mapfile -t found < <(sed -n 's/^## \(.*[^[:space:]]\)[[:space:]]*$/\1/p' "$file")

  for sec in "${expected[@]}"; do
    count=0
    for heading in "${found[@]}"; do
      [[ "$heading" == "$sec" ]] && count=$((count + 1))
    done
    if (( count == 0 )); then
      err "$file: missing required section '## $sec'"
      all_present_once=0
    elif (( count > 1 )); then
      err "$file: duplicate section '## $sec' (must appear exactly once)"
      all_present_once=0
    fi
  done

  # Order check only when each expected section appears exactly once.
  if (( all_present_once )); then
    local sequence=()
    for heading in "${found[@]}"; do
      for sec in "${expected[@]}"; do
        [[ "$heading" == "$sec" ]] && sequence+=("$heading")
      done
    done
    if [[ "$(printf '%s\n' "${sequence[@]}")" != "$(printf '%s\n' "${expected[@]}")" ]]; then
      err "$file: sections out of order (required: ${expected[*]})"
    fi
  fi
}

# --- per-file structure ----------------------------------------------------

runbook_files=()
while IFS= read -r file; do
  case "$(basename "$file")" in
    README.md|POSTMORTEM.md) continue ;;
  esac
  runbook_files+=("$file")
done < <(find "$RUNBOOKS_DIR" -maxdepth 1 -name '*.md' | sort)

for file in "${runbook_files[@]}"; do
  check_title "$file"
  check_sections "$file" "${RUNBOOK_SECTIONS[@]}"
done

if [[ -f "$RUNBOOKS_DIR/POSTMORTEM.md" ]]; then
  check_title "$RUNBOOKS_DIR/POSTMORTEM.md"
  check_sections "$RUNBOOKS_DIR/POSTMORTEM.md" "${POSTMORTEM_SECTIONS[@]}"
fi

# --- index (README.md) -----------------------------------------------------

readme="$RUNBOOKS_DIR/README.md"
if [[ -f "$readme" ]]; then
  # Each required runbook has exactly one index row, with a stub/complete status.
  for slug in "${REQUIRED_RUNBOOKS[@]}"; do
    row_count="$(grep -c "]($slug\.md)" "$readme" || true)"
    if (( row_count == 0 )); then
      err "$readme: missing index entry for required runbook '$slug.md' (§21.2)"
    elif (( row_count > 1 )); then
      err "$readme: multiple index entries for '$slug.md'"
    else
      row="$(grep "]($slug\.md)" "$readme")"
      if ! grep -qE '\|[[:space:]]*(stub|complete)[[:space:]]*\|' <<<"$row"; then
        err "$readme: index entry for '$slug.md' lacks a status of 'stub' or 'complete'"
      fi
    fi
  done

  # Every runbook file present is listed in the index.
  for file in "${runbook_files[@]}"; do
    base="$(basename "$file")"
    [[ "$base" == "TEMPLATE.md" ]] && continue
    if ! grep -q "]($base)" "$readme"; then
      err "$readme: runbook file '$base' exists but is not listed in the index"
    fi
  done

  # No dead relative .md links anywhere in README.md.
  while IFS= read -r ref; do
    [[ -f "$RUNBOOKS_DIR/$ref" ]] || err "$readme: links to '$ref' which does not exist"
  done < <(grep -oE '\]\([A-Za-z0-9._-]+\.md\)' "$readme" | sed -E 's/^\]\((.*)\)$/\1/' | sort -u)
fi

# --- verdict ---------------------------------------------------------------

if (( errors > 0 )); then
  echo "lint-runbooks: FAIL — $errors error(s) in $RUNBOOKS_DIR" >&2
  exit 1
fi

echo "lint-runbooks: OK — $((${#runbook_files[@]})) runbook file(s) structurally valid in $RUNBOOKS_DIR"
