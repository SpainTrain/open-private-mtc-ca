#!/usr/bin/env bash
# codemap-smoke-test.sh — sandboxed fixture-workspace tests for
# scripts/codemap-gen.sh (spec §19.13 spirit; `make codemap-test`).
#
# Runs entirely against a throwaway `mktemp -d` copy of
# scripts/testdata/codemap/fixture-workspace/ — never against the real
# workspace's Cargo.toml/Cargo.lock — following the sandbox pattern in
# scripts/adr-smoke-test.sh / scripts/test-lint-runbooks.sh.
#
# Covers the three QA findings that failed the prior codemap-generator
# attempt:
#   1. cargo metadata is the actual data source (the generator has no other
#      way to learn the crate graph; if this test passes at all, metadata
#      parsing worked).
#   2. Grouped `pub use mod::{A, B, C};` re-exports — including split across
#      multiple lines with a renamed (`as`) export — are fully preserved,
#      not mangled into a content-free bullet.
#   3. USED BY lists real dependent-crate names from the resolve graph
#      (fixture-consumer depends on fixture-base; fixture-base's USED BY
#      must name it exactly), not a placeholder.
# Plus: deterministic output (run twice, byte-identical) and the
# `codemap-check`-style staleness check (mutate the fixture, confirm the
# regenerated output actually differs).
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
fixture_src="$script_dir/testdata/codemap/fixture-workspace"
gen="$script_dir/codemap-gen.sh"

sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT
cp -r "$fixture_src/." "$sandbox/"

pass=0
fail() {
  printf 'codemap-smoke-test: FAIL: %s\n' "$1" >&2
  exit 1
}
ok() {
  pass=$((pass + 1))
  printf 'ok: %s\n' "$1"
}

out="$("$gen" "$sandbox")"

# --- 1 & 2. Grouped pub-use (single-line and multi-line-with-alias) is fully
# preserved, per-symbol, not collapsed into an empty/mangled bullet. ---------

base_block="$(awk '/^CRATE: fixture-base$/{p=1} p{print} /^CRATE: fixture-consumer$/{exit}' <<< "$out")"

for sym in 'struct Gadget' 'struct Widget' 'const GIZMO_VERSION'; do
  grep -qF -- "- $sym" <<< "$base_block" \
    || fail "single-line grouped pub-use lost '$sym'"
done
ok "single-line grouped pub-use (alpha::{Gadget, Widget, GIZMO_VERSION}) preserved"

for sym in 'fn frobnicate' 'enum FrobError' 'struct Frobnicator'; do
  grep -qF -- "- $sym" <<< "$base_block" \
    || fail "multi-line grouped pub-use lost '$sym'"
done
ok "multi-line grouped pub-use (beta::{frobnicate, FrobError, Frobnicator, ...}) preserved"

grep -qF -- '- struct Aliased' <<< "$base_block" \
  || fail "multi-line grouped pub-use lost the renamed export 'Renamed as Aliased'"
grep -qF -- 'Renamed' <<< "$base_block" \
  && fail "renamed export leaked its original name 'Renamed' instead of 'Aliased'"
ok "renamed ('as') export in a grouped pub-use preserved under its public name"

# The prior attempt's bug (`sed 's/{.*//'`) turns the whole multi-line
# statement into a single content-free "- beta" (or similar) bullet. Guard
# against any regression back to that shape.
grep -qE -- '^\s*-\s+(mod\s+)?beta\s*$' <<< "$base_block" \
  && fail "grouped pub-use regressed to a mangled bare 'beta' bullet"
ok "no mangled content-free bullet from the grouped pub-use"

# Strict module privacy (§23.6): the public `alpha` module is listed; the
# private `beta` module (only its selected items are re-exported) is not.
grep -qF -- '- mod alpha' <<< "$base_block" || fail "public module 'alpha' missing from PUBLIC API"
grep -qF -- '- mod beta' <<< "$base_block" && fail "private module 'beta' leaked into PUBLIC API"
ok "strict module privacy: public module listed, private module's re-exports only"

# --- 3. USED BY / DEPENDS ON are real crate names from the resolve graph, ---
# not a placeholder, and the edge is correct in both directions. ------------

grep -qF -- 'USED BY: fixture-consumer' <<< "$base_block" \
  || fail "fixture-base USED BY does not name real dependent crate 'fixture-consumer'"
grep -qF -- 'USED BY: (none)' <<< "$out" \
  || fail "expected at least one crate with no dependents to print 'USED BY: (none)'"
ok "USED BY lists real reverse-dependency crate names from the resolve graph"

consumer_block="$(awk '/^CRATE: fixture-consumer$/{p=1} p{print} /^CRATE: fixture-leaf$/{exit}' <<< "$out")"
grep -qF -- 'DEPENDS ON: fixture-base' <<< "$consumer_block" \
  || fail "fixture-consumer DEPENDS ON does not name 'fixture-base'"
ok "DEPENDS ON lists real workspace-internal dependency crate names"

# --- Empty-crate path: no public items, no deps, no dependents. ------------

leaf_block="$(awk '/^CRATE: fixture-leaf$/{p=1} p{print}' <<< "$out")"
grep -qF -- '(none)' <<< "$leaf_block" || fail "fixture-leaf PUBLIC API did not report '(none)'"
grep -qF -- 'DEPENDS ON: (none)' <<< "$leaf_block" || fail "fixture-leaf DEPENDS ON did not report '(none)'"
grep -qF -- 'USED BY: (none)' <<< "$leaf_block" || fail "fixture-leaf USED BY did not report '(none)'"
ok "crate with no public items/deps/dependents renders all three as '(none)'"

# --- Determinism: running twice against an unchanged workspace is ----------
# byte-identical (AC: "Deterministic, stably ordered output"). -------------

out2="$("$gen" "$sandbox")"
[ "$out" = "$out2" ] || fail "two runs against the same unchanged workspace produced different output"
ok "determinism: two runs produce byte-identical output"

# --- codemap-check-style staleness detection: mutating the fixture must ----
# change the regenerated output (what `make codemap-check` diffs against the
# committed CODEMAP.md to decide pass/fail). --------------------------------

printf '\npub const ADDED_AFTER_SNAPSHOT: u32 = 2;\n' >> "$sandbox/crates/base/src/lib.rs"
out3="$("$gen" "$sandbox")"
if [ "$out" = "$out3" ]; then
  fail "regenerating after a structural change produced identical output (codemap-check would never fire)"
fi
grep -qF -- '- const ADDED_AFTER_SNAPSHOT' <<< "$out3" \
  || fail "newly added pub item did not appear after regeneration"
ok "codemap-check-style staleness: a structural change changes the regenerated output"

# --- shellcheck, when available. --------------------------------------------

if command -v shellcheck > /dev/null 2>&1; then
  shellcheck "$gen" "$script_dir/codemap-check.sh" "${BASH_SOURCE[0]}" \
    || fail "shellcheck reported issues"
  pass=$((pass + 1))
else
  printf 'codemap-smoke-test: shellcheck not installed; skipping lint check\n'
fi

printf 'codemap-smoke-test: OK (%d checks passed)\n' "$pass"
