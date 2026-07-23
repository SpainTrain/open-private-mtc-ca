#!/usr/bin/env bash
# agent-inner-loop-test.sh — smoke tests for the §23.4 inner-loop tooling
# (ticket agent-inner-loop-targets; spec §19.13 spirit).
#
# Covers:
#   - recent-journal.sh entry-extraction logic against a fixture journal
#   - working-set-init.sh create / refuse-to-overwrite behavior (sandboxed)
#   - agent-precheck failing loudly when a required tool is missing
#   - watch.sh tool selection (--print) or its graceful missing-tool message
#   - the real `make agent-precheck` and `make verify-task` gates passing on
#     the current tree (slow-ish: compiles + full lint; warm-cache friendly)
#   - shellcheck over the inner-loop scripts, when installed

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

fail() {
  printf 'agent-inner-loop-test: FAIL: %s\n' "$1" >&2
  exit 1
}

# Make sure ambient variables cannot leak into the tests.
unset JOURNAL_FILE WORKING_SET_FILE PRECHECK_TOOLS PRECHECK_JOURNAL_ENTRIES 2> /dev/null || true
checks=0

# 1. recent-journal.sh: last-N extraction against a fixture journal.
fixture=$tmpdir/journal.md
printf '# Fixture journal\n\npreamble text\n' > "$fixture"
for i in 1 2 3 4 5; do
  printf '\n## 2099-01-0%s — entry %s\n\nbody %s\n' "$i" "$i" "$i" >> "$fixture"
done

out=$(JOURNAL_FILE=$fixture "$script_dir/recent-journal.sh")
grep -Fq 'entry 3' <<< "$out" || fail 'default N=3 missing 3rd-from-last entry'
grep -Fq 'entry 5' <<< "$out" || fail 'default N=3 missing last entry'
grep -Fq 'entry 2' <<< "$out" && fail 'default N=3 printed a 4th entry'
checks=$((checks + 1))

out=$(JOURNAL_FILE=$fixture "$script_dir/recent-journal.sh" 1)
grep -Fq 'entry 5' <<< "$out" || fail 'N=1 missing last entry'
grep -Fq 'entry 4' <<< "$out" && fail 'N=1 printed more than one entry'
checks=$((checks + 1))

out=$(JOURNAL_FILE=$fixture "$script_dir/recent-journal.sh" 99)
grep -Fq 'entry 1' <<< "$out" || fail 'N larger than entry count should print all entries'
checks=$((checks + 1))

printf '# Empty journal, no entries\n' > "$tmpdir/empty.md"
out=$(JOURNAL_FILE=$tmpdir/empty.md "$script_dir/recent-journal.sh")
grep -Fq 'no entries yet' <<< "$out" || fail 'empty journal not reported'
checks=$((checks + 1))

JOURNAL_FILE=$fixture "$script_dir/recent-journal.sh" bogus > /dev/null 2>&1 \
  && fail 'non-numeric N was accepted'
JOURNAL_FILE=$tmpdir/absent.md "$script_dir/recent-journal.sh" > /dev/null 2>&1 \
  && fail 'missing journal file was accepted'
checks=$((checks + 1))

# 2. working-set-init.sh: creates from template; refuses to overwrite.
ws=$tmpdir/WORKING_SET.md
WORKING_SET_FILE=$ws "$script_dir/working-set-init.sh" > /dev/null
[ -f "$ws" ] || fail 'working-set-init did not create the working copy'
grep -Fq '## Files touched' "$ws" || fail 'working copy missing "Files touched" section'
grep -Fq '## Decisions made' "$ws" || fail 'working copy missing "Decisions made" section'
grep -Fq '## Open questions' "$ws" || fail 'working copy missing "Open questions" section'
checks=$((checks + 1))

printf 'sentinel' >> "$ws"
before=$(cat "$ws")
if WORKING_SET_FILE=$ws "$script_dir/working-set-init.sh" > /dev/null 2>&1; then
  fail 'working-set-init overwrote an existing working copy'
fi
[ "$before" = "$(cat "$ws")" ] || fail 'refused overwrite still modified the file'
checks=$((checks + 1))

# 3. The root working copy is gitignored (the convention, enforced).
if git -C "$repo_root" check-ignore -q WORKING_SET.md; then
  checks=$((checks + 1))
else
  fail 'WORKING_SET.md at the repo root is not gitignored'
fi

# 4. agent-precheck: fails loudly (non-zero, MISSING named) on a missing tool.
if out=$(PRECHECK_TOOLS='definitely-not-a-real-tool-xyz' \
  "$script_dir/agent-precheck.sh" 2>&1); then
  fail 'agent-precheck passed despite a missing required tool'
fi
grep -Fq 'MISSING' <<< "$out" || fail 'agent-precheck did not name the missing tool'
grep -Fq 'make doctor' <<< "$out" || fail 'agent-precheck did not point at make doctor'
checks=$((checks + 1))

# 5. watch.sh: picks a watcher (--print) when one is installed; otherwise
#    degrades gracefully with install hints and a non-zero exit.
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
if command -v bacon > /dev/null 2>&1 || command -v cargo-watch > /dev/null 2>&1; then
  out=$("$script_dir/watch.sh" --print)
  grep -Eq 'bacon|cargo watch' <<< "$out" || fail 'watch --print did not name a watcher'
else
  if out=$("$script_dir/watch.sh" --print 2>&1); then
    fail 'watch exited zero with no watcher installed'
  fi
  grep -Fq 'cargo install' <<< "$out" || fail 'watch missing-tool message lacks install hint'
fi
"$script_dir/watch.sh" --bogus-flag > /dev/null 2>&1 && fail 'watch accepted an unknown flag'
checks=$((checks + 1))

# 6. The real gates pass on the current tree (invoked via make, as agents do).
make -C "$repo_root" -s agent-precheck > /dev/null || fail 'make agent-precheck failed on this tree'
checks=$((checks + 1))
make -C "$repo_root" -s verify-task > /dev/null || fail 'make verify-task failed on this tree'
checks=$((checks + 1))

# 7. shellcheck, when available.
if command -v shellcheck > /dev/null 2>&1; then
  shellcheck \
    "$script_dir/agent-precheck.sh" \
    "$script_dir/verify-task.sh" \
    "$script_dir/watch.sh" \
    "$script_dir/recent-journal.sh" \
    "$script_dir/working-set-init.sh" \
    "${BASH_SOURCE[0]}" \
    || fail 'shellcheck reported issues'
  checks=$((checks + 1))
else
  printf 'agent-inner-loop-test: shellcheck not installed; skipping lint check\n'
fi

printf 'agent-inner-loop-test: OK (%d checks passed)\n' "$checks"
