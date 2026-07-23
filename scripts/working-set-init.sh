#!/usr/bin/env bash
# working-set-init.sh — start a task-scoped WORKING_SET.md (spec §23.4) from
# the checked-in template (docs/templates/WORKING_SET.md).
#
# The working copy lives at the repo root and is gitignored (see .gitignore:
# /WORKING_SET.md) — it is task-scoped scratch state, never committed. This
# script refuses to overwrite an existing working copy.
#
# Usage:
#   scripts/working-set-init.sh        # via `make working-set`
#
# Environment:
#   WORKING_SET_FILE=path   target file (default: <repo root>/WORKING_SET.md;
#                           used by the smoke tests)

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
template=$repo_root/docs/templates/WORKING_SET.md
target=${WORKING_SET_FILE:-$repo_root/WORKING_SET.md}

if [ ! -f "$template" ]; then
  printf 'working-set: error: template not found: %s\n' "$template" >&2
  exit 1
fi

if [ -e "$target" ]; then
  printf 'working-set: %s already exists — refusing to overwrite.\n' "$target" >&2
  printf 'working-set: finish or archive the current task first, then delete it.\n' >&2
  exit 1
fi

cp -f "$template" "$target"
printf 'working-set: created %s\n' "$target"
printf 'working-set: fill in Task/Branch/Started and keep it updated as you work.\n'
