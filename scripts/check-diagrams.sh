#!/usr/bin/env bash
# Validate Mermaid syntax in markdown files (default: docs/architecture/*.md).
#
# Usage:
#   scripts/check-diagrams.sh                 # check all docs/architecture pages
#   scripts/check-diagrams.sh file.md [...]   # check specific markdown files
#
# First run installs pinned tooling (mermaid + jsdom) into
# scripts/diagrams-lint/node_modules — network needed once. Subsequent runs are
# fully offline. No browser/Chromium download involved.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lint_dir="${repo_root}/scripts/diagrams-lint"

if ! command -v node >/dev/null 2>&1; then
  echo "error: node is required (>= 18). Install Node.js and re-run." >&2
  exit 2
fi

if [ ! -d "${lint_dir}/node_modules/mermaid" ]; then
  echo "Installing mermaid lint tooling (one-time; offline afterwards)..." >&2
  npm --prefix "${lint_dir}" install --no-audit --no-fund --loglevel=error
fi

if [ "$#" -gt 0 ]; then
  files=("$@")
else
  shopt -s nullglob
  files=("${repo_root}"/docs/architecture/*.md)
  shopt -u nullglob
  if [ "${#files[@]}" -eq 0 ]; then
    echo "error: no markdown files found in docs/architecture/" >&2
    exit 1
  fi
fi

exec node "${lint_dir}/check.mjs" "${files[@]}"
