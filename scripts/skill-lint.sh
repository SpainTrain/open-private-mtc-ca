#!/usr/bin/env bash
# skill-lint.sh — validate .claude/skills/ skill files (spec section 23.1).
#
# Lints every skill file — `.claude/skills/*.md` except README.md, plus the
# Claude Code native layout `.claude/skills/*/SKILL.md` — for:
#
#   1. Required sections: `## Goal`, `## Files involved`, `## Pattern`,
#      `## Common pitfalls`.
#   2. Every "Files involved" bullet starts with a backticked repo-relative
#      path, and every such path exists in the repo.
#   3. At least one file pointer (zero is a failure); a count outside the
#      3-5 target (section 23.1 / 23.8 context budget) is a warning.
#
# Optional YAML frontmatter (a leading `--- ... ---` block) is ignored.
#
# Usage: scripts/skill-lint.sh [skills-dir]
#   skills-dir defaults to <repo-root>/.claude/skills. Path existence is
#   always checked relative to the repo root, so fixture dirs elsewhere
#   (e.g. under /tmp in the smoke test) lint exactly like the real thing.
#
# Exit status: 0 clean, 1 on any violation. One line per violation:
#   skill-lint: FAIL <file>: <reason>

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${repo_root}" ]]; then
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd "${script_dir}/.." && pwd)"
fi

skills_dir="${1:-${repo_root}/.claude/skills}"

if [[ ! -d "${skills_dir}" ]]; then
  echo "skill-lint: FAIL: skills directory not found: ${skills_dir}" >&2
  exit 1
fi

required_sections=("Goal" "Files involved" "Pattern" "Common pitfalls")
bullet_start_re='^[[:space:]]*[-*][[:space:]]'
# Literal backticks, not command substitution — this is a regex.
# shellcheck disable=SC2016
bullet_path_re='^[[:space:]]*[-*][[:space:]]+`([^`]+)`'

failures=0
warnings=0
checked=0

fail() { # <file> <reason>
  echo "skill-lint: FAIL ${1}: ${2}" >&2
  failures=$((failures + 1))
}

warn() { # <file> <reason>
  echo "skill-lint: warn ${1}: ${2}" >&2
  warnings=$((warnings + 1))
}

# Collect skill files (sorted, stable).
skill_files=()
while IFS= read -r f; do
  skill_files+=("${f}")
done < <(
  {
    find "${skills_dir}" -maxdepth 1 -type f -name '*.md' ! -name 'README.md'
    find "${skills_dir}" -mindepth 2 -maxdepth 2 -type f -name 'SKILL.md'
  } | sort
)

if [[ ${#skill_files[@]} -eq 0 ]]; then
  echo "skill-lint: FAIL: no skill files found in ${skills_dir}" >&2
  exit 1
fi

for file in "${skill_files[@]}"; do
  checked=$((checked + 1))
  rel="${file#"${repo_root}"/}"

  # Strip optional YAML frontmatter so headings are matched on the body only.
  body="$(awk 'NR==1 && $0=="---" {fm=1; next}
               fm==1 {if ($0=="---") fm=0; next}
               {print}' "${file}")"

  have_files_section=1
  for section in "${required_sections[@]}"; do
    if ! grep -qiE "^##[[:space:]]+${section}[[:space:]]*$" <<<"${body}"; then
      fail "${rel}" "missing required section '## ${section}'"
      if [[ "${section}" == "Files involved" ]]; then
        have_files_section=0
      fi
    fi
  done

  # Path checks only make sense when the section exists.
  if [[ ${have_files_section} -eq 0 ]]; then
    continue
  fi

  # Extract the body of the "Files involved" section (up to the next `##`).
  files_section="$(awk '
    tolower($0) ~ /^##[[:space:]]+files involved[[:space:]]*$/ {insec=1; next}
    insec && /^##[[:space:]]/ {insec=0}
    insec {print}' <<<"${body}")"

  path_count=0
  while IFS= read -r line; do
    [[ "${line}" =~ ${bullet_start_re} ]] || continue
    if [[ "${line}" =~ ${bullet_path_re} ]]; then
      path="${BASH_REMATCH[1]}"
      path_count=$((path_count + 1))
      if [[ "${path}" == /* ]]; then
        fail "${rel}" "'Files involved' path must be repo-relative, not absolute: ${path}"
      elif [[ ! -e "${repo_root}/${path}" ]]; then
        fail "${rel}" "'Files involved' path does not exist: ${path}"
      fi
    else
      fail "${rel}" "'Files involved' bullet must start with a backticked repo-relative path: ${line}"
    fi
  done <<<"${files_section}"

  if [[ ${path_count} -eq 0 ]]; then
    fail "${rel}" "'Files involved' has no file pointers (need 3-5 backticked paths)"
  elif [[ ${path_count} -lt 3 || ${path_count} -gt 5 ]]; then
    warn "${rel}" "'Files involved' has ${path_count} pointers (target is 3-5, spec section 23.1)"
  fi
done

if [[ ${failures} -gt 0 ]]; then
  echo "skill-lint: ${failures} violation(s) across ${checked} skill file(s)" >&2
  exit 1
fi

echo "skill-lint: OK — ${checked} skill file(s) checked, ${warnings} warning(s)"
