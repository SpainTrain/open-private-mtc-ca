#!/usr/bin/env bash
# watch.sh — inner-loop watcher (spec §23.4): lint+test on every save.
#
# Prefers bacon (repo config in bacon.toml; its default job runs clippy with
# -D warnings, then the fast tests). Falls back to cargo-watch with an
# equivalent -x chain. When neither watcher is installed, prints install hints
# and exits non-zero — `make doctor` owns full environment diagnostics.
#
# Usage:
#   scripts/watch.sh            # via `make watch`; Ctrl-C to stop
#   scripts/watch.sh --print    # show the watcher command without running it
#                               # (used by scripts/agent-inner-loop-test.sh)

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
cd "$repo_root"

# Rust tools (cargo, bacon, cargo-watch) live in ~/.cargo/bin.
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

print_only=false
if [ "${1:-}" = '--print' ]; then
  print_only=true
elif [ "$#" -gt 0 ]; then
  printf 'watch: unknown argument: %s (only --print is supported)\n' "$1" >&2
  exit 2
fi

if command -v bacon > /dev/null 2>&1; then
  # bacon.toml sets default_job = "lint-test" (clippy -D warnings, then tests).
  cmd=(bacon)
elif command -v cargo-watch > /dev/null 2>&1; then
  cmd=(cargo watch --clear
    -x 'clippy --workspace --all-targets -- -D warnings'
    -x 'test --workspace --quiet')
else
  cat >&2 << 'EOF'
watch: no file watcher installed. Install one (offline afterwards):

  cargo install --locked bacon         # preferred; repo config in bacon.toml
  cargo install --locked cargo-watch   # fallback

then re-run `make watch`. (`make doctor` diagnoses the full dev environment.)
EOF
  exit 1
fi

if $print_only; then
  printf 'watch: would run:'
  printf ' %q' "${cmd[@]}"
  printf '\n'
  exit 0
fi

printf 'watch: starting %s — lint+test on every save (Ctrl-C to stop)\n' "${cmd[0]}"
exec "${cmd[@]}"
