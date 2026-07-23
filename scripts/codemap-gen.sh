#!/usr/bin/env bash
# codemap-gen.sh — generate CODEMAP.md from `cargo metadata` (spec §23.6).
#
# Data source is `cargo metadata --format-version 1` — NOT hand-parsed
# Cargo.toml/lib.rs text. Two things come straight from cargo's own model:
#   - PURPOSE: each workspace package's Cargo.toml `description` field
#   - DEPENDS ON / USED BY: the resolve graph (`.resolve.nodes[].deps`),
#     restricted to workspace-internal packages; USED BY is the graph
#     inverted, so it always reflects real dependent crate names (never a
#     placeholder like "see cargo tree...").
#
# The one thing cargo metadata does not model is a crate's *public API
# surface*, so PUBLIC API is extracted by reading each crate's `src/lib.rs`
# only (top-level surface, per §23.6 "strict module privacy" — internal
# modules are not walked). Grouped re-exports
# (`pub use mod::{A, B, C};`, including ones split across lines) are expanded
# to every symbol they name; a best-effort lookup in the target module file
# labels each symbol's kind (struct/enum/trait/fn/const/static/type) where it
# can be found, and falls back to the bare name otherwise.
#
# Output is deterministic: crates are sorted by name, DEPENDS ON/USED BY are
# sorted+deduped, and PUBLIC API items are listed in lib.rs source order (a
# property of the file, not of any hash/iteration order) — running this twice
# against an unchanged workspace produces byte-identical output.
#
# Usage: codemap-gen.sh [WORKSPACE_ROOT]
#   WORKSPACE_ROOT defaults to the repo this script lives in; overriding it
#   lets scripts/codemap-smoke-test.sh point the generator at a throwaway
#   fixture workspace. Output goes to stdout.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
workspace_root=$(cd -- "${1:-$repo_root}" && pwd)
manifest="$workspace_root/Cargo.toml"

# Rust tools (cargo) live in ~/.cargo/bin.
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

need() {
  command -v "$1" > /dev/null 2>&1 || {
    printf 'codemap-gen: error: %s is required (%s)\n' "$1" "$2" >&2
    exit 1
  }
}
need cargo "install via rustup (https://rustup.rs); run \`make doctor\`"
need jq "JSON processor used to parse cargo metadata output"

[ -f "$manifest" ] || {
  printf 'codemap-gen: error: no Cargo.toml at %s\n' "$manifest" >&2
  exit 1
}

# --- jq: cargo-metadata -> one compact JSON object per workspace crate -----
# Restricting DEPENDS ON / USED BY to workspace-internal packages matches the
# §23.6 example ("DEPENDS ON: cloud-types, types, clock") — external crates
# (tokio, serde, ...) are not part of this repo's navigable crate graph.
jq_filter="$script_dir/codemap-crates.jq"

metadata_json=$(cargo metadata --format-version 1 --manifest-path "$manifest")

crates_ndjson=$(printf '%s' "$metadata_json" | jq -c -f "$jq_filter")

# --- Rust-source parsing: PUBLIC API from src/lib.rs ------------------------
#
# emit_item KIND NAME CFG — print one "    - " PUBLIC API line. Tracks its
# own emission count (codemap_item_count) so extract_public_api can tell
# "genuinely no public items" apart from "every pub line was unrecognized"
# without threading a flag through every call site.
codemap_item_count=0
emit_item() {
  local kind="$1" name="$2" cfg="$3" line
  if [ -n "$kind" ]; then
    line="$kind $name"
  else
    line="$name"
  fi
  if [ -n "$cfg" ]; then
    line="$line (cfg: $cfg)"
  fi
  printf '    - %s\n' "$line"
  codemap_item_count=$((codemap_item_count + 1))
}

# resolve_modfile SRC_DIR MODPATH — echo the source file a `pub use`
# module-path resolves to (module.rs or module/mod.rs), or nothing if it
# cannot be found; used only for a best-effort kind label, never to walk
# further re-exports.
resolve_modfile() {
  local src_dir="$1" modpath="$2"
  modpath="${modpath#crate::}"
  modpath="${modpath#self::}"
  local relpath="${modpath//::/\/}"
  if [ -f "$src_dir/$relpath.rs" ]; then
    printf '%s/%s.rs\n' "$src_dir" "$relpath"
  elif [ -f "$src_dir/$relpath/mod.rs" ]; then
    printf '%s/%s/mod.rs\n' "$src_dir" "$relpath"
  fi
}

# resolve_kind FILE NAME — echo struct/enum/trait/fn/const/static/type/union
# if a declaration for NAME is found in FILE, else nothing.
resolve_kind() {
  local file="$1" name="$2"
  [ -n "$file" ] && [ -f "$file" ] || return 0
  grep -m1 -oE "(struct|enum|trait|fn|const|static|type|union)[[:space:]]+${name}\b" "$file" 2> /dev/null \
    | awk '{print $1}' | head -n1
}

# split_top_level_commas TEXT — print each top-level (brace-depth 0) segment
# of TEXT on its own line; segments inside nested {..} groups stay intact so
# a caller can recurse into them.
split_top_level_commas() {
  local text="$1" depth=0 seg="" ch i len
  len=${#text}
  for ((i = 0; i < len; i++)); do
    ch="${text:i:1}"
    case "$ch" in
      '{') depth=$((depth + 1)); seg+="$ch" ;;
      '}') depth=$((depth - 1)); seg+="$ch" ;;
      ',')
        if [ "$depth" -eq 0 ]; then
          printf '%s\n' "$seg"
          seg=""
        else
          seg+="$ch"
        fi
        ;;
      *) seg+="$ch" ;;
    esac
  done
  [ -n "${seg// /}" ] && printf '%s\n' "$seg"
}

# emit_use_group SRC_DIR MODPATH BODY CFG — expand one `mod::{ ... }` group
# (BODY is the text between the braces), recursing one level into any nested
# group so `mod::{a, sub::{b, c}}` still yields every symbol.
emit_use_group() {
  local src_dir="$1" modpath="$2" body="$3" cfg="$4"
  while IFS= read -r item; do
    item="$(printf '%s' "$item" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
    [ -n "$item" ] || continue
    if [ "$item" = "self" ]; then
      emit_item "mod" "${modpath##*::}" "$cfg"
      continue
    fi
    if printf '%s' "$item" | grep -qE '::\{'; then
      local sub_mod="${item%%::\{*}"
      local sub_body="${item#*::\{}"
      sub_body="${sub_body%\}}"
      emit_use_group "$src_dir" "$modpath::$sub_mod" "$sub_body" "$cfg"
      continue
    fi
    local orig="$item" display="$item"
    if printf '%s' "$item" | grep -qE ' as '; then
      orig="$(printf '%s' "$item" | sed -E 's/^([A-Za-z_][A-Za-z0-9_]*)[[:space:]]+as[[:space:]]+([A-Za-z_][A-Za-z0-9_]*)$/\1/')"
      display="$(printf '%s' "$item" | sed -E 's/^([A-Za-z_][A-Za-z0-9_]*)[[:space:]]+as[[:space:]]+([A-Za-z_][A-Za-z0-9_]*)$/\2/')"
    fi
    local file kind
    file="$(resolve_modfile "$src_dir" "$modpath")"
    kind="$(resolve_kind "$file" "$orig")"
    emit_item "$kind" "$display" "$cfg"
  done < <(split_top_level_commas "$body")
}

# process_pub_use SRC_DIR STATEMENT CFG — STATEMENT is a full, single-line
# `pub use ...;` (continuation lines already joined by the caller).
process_pub_use() {
  local src_dir="$1" stmt="$2" cfg="$3"
  local body="${stmt#pub use }"
  body="${body%;}"
  body="$(printf '%s' "$body" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
  if printf '%s' "$body" | grep -qE '::\{'; then
    local modpath group_body
    modpath="${body%%::\{*}"
    group_body="${body#*::\{}"
    group_body="${group_body%\}}"
    emit_use_group "$src_dir" "$modpath" "$group_body" "$cfg"
  else
    # No group: `pub use path::Item;` or `pub use path::Item as Alias;`
    local orig="$body" display item
    if printf '%s' "$body" | grep -qE ' as '; then
      orig="$(printf '%s' "$body" | sed -E 's/^.*::([A-Za-z_][A-Za-z0-9_]*)[[:space:]]+as[[:space:]]+([A-Za-z_][A-Za-z0-9_]*)$/\1/')"
      display="$(printf '%s' "$body" | sed -E 's/^.*[[:space:]]as[[:space:]]+([A-Za-z_][A-Za-z0-9_]*)$/\1/')"
    else
      orig="${body##*::}"
      display="$orig"
    fi
    local modpath file kind
    modpath="${body%::*}"
    if [ "$modpath" = "$body" ]; then
      modpath=""
    fi
    item="$orig"
    file="$(resolve_modfile "$src_dir" "$modpath")"
    kind="$(resolve_kind "$file" "$item")"
    emit_item "$kind" "$display" "$cfg"
  fi
}

# parse_direct_item LINE CFG — LINE is `pub [modifiers] KEYWORD NAME ...`
# declared directly in lib.rs (not a `pub use`/`pub mod`), e.g.
# `pub const fn marker() -> &'static str {` or `pub const API_VERSION: &str`.
parse_direct_item() {
  local line="$1" cfg="$2"
  local rest="${line#pub}"
  rest="${rest# }"
  local -a toks
  read -ra toks <<< "$rest"
  local i=0 kw="" n=${#toks[@]}
  while [ "$i" -lt "$n" ]; do
    local t="${toks[$i]}"
    case "$t" in
      async | unsafe | default)
        i=$((i + 1))
        ;;
      extern)
        i=$((i + 1))
        if [ "$i" -lt "$n" ] && [[ "${toks[$i]}" == \"*\" ]]; then
          i=$((i + 1))
        fi
        ;;
      const)
        if [ $((i + 1)) -lt "$n" ] && [ "${toks[$((i + 1))]}" = "fn" ]; then
          kw="fn"
          i=$((i + 2))
        else
          kw="const"
          i=$((i + 1))
        fi
        break
        ;;
      fn | struct | enum | trait | static | type | union | mod)
        kw="$t"
        i=$((i + 1))
        break
        ;;
      *)
        return 0
        ;;
    esac
  done
  [ -n "$kw" ] || return 0
  local name_tok="${toks[$i]:-}"
  [[ "$name_tok" =~ ^([A-Za-z_][A-Za-z0-9_]*) ]] || return 0
  emit_item "$kw" "${BASH_REMATCH[1]}" "$cfg"
}

# extract_public_api LIB_RS — the top-level-surface PUBLIC API scan.
# Reads only LIB_RS (never submodule files, beyond the single-line kind
# lookups above) — the §23.6 "strict module privacy" surface.
extract_public_api() {
  local lib_rs="$1"
  local src_dir
  src_dir="$(dirname -- "$lib_rs")"
  local before=$codemap_item_count
  local pending_cfg="" buf="" depth=0 in_use=0
  while IFS= read -r line || [ -n "$line" ]; do
    if [ -z "$line" ]; then
      pending_cfg=""
      continue
    fi
    case "$line" in
      //*)
        continue
        ;;
    esac
    if [[ "$line" == '#!['* ]]; then
      continue
    fi
    if [[ "$line" =~ ^#\[cfg\((.*)\)\]$ ]]; then
      pending_cfg="${BASH_REMATCH[1]}"
      continue
    fi
    if [[ "$line" == '#['* ]]; then
      continue
    fi
    if [ "$in_use" -eq 1 ]; then
      buf="$buf $line"
      local opens="${line//[^\{]/}" closes="${line//[^\}]/}"
      depth=$((depth + ${#opens} - ${#closes}))
      if [ "$depth" -le 0 ]; then
        in_use=0
        process_pub_use "$src_dir" "$buf" "$pending_cfg"
        pending_cfg=""
        buf=""
      fi
      continue
    fi
    if [[ "$line" == 'pub use '* ]]; then
      buf="$line"
      local opens="${line//[^\{]/}" closes="${line//[^\}]/}"
      depth=$((${#opens} - ${#closes}))
      if [ "$depth" -le 0 ]; then
        process_pub_use "$src_dir" "$buf" "$pending_cfg"
        pending_cfg=""
        buf=""
      else
        in_use=1
      fi
      continue
    fi
    if [[ "$line" == 'pub mod '* ]]; then
      local name="${line#pub mod }"
      name="${name%%[\;\{ ]*}"
      emit_item "mod" "$name" "$pending_cfg"
      pending_cfg=""
      continue
    fi
    if [[ "$line" == 'pub '* ]]; then
      # Best-effort: parse_direct_item silently no-ops on a `pub` line it
      # doesn't recognize (e.g. a re-exported macro_rules!); the pending cfg
      # is still consumed either way so it can't leak onto an unrelated item.
      parse_direct_item "$line" "$pending_cfg"
      pending_cfg=""
      continue
    fi
    pending_cfg=""
  done < "$lib_rs"
  [ "$codemap_item_count" -gt "$before" ]
}

# --- Emit CODEMAP.md ---------------------------------------------------------

cat << 'EOF'
# CODEMAP.md

Auto-generated by `make codemap` (scripts/codemap-gen.sh) from `cargo
metadata` — do not hand-edit. Summarizes every workspace crate's purpose,
top-level public API surface (from `src/lib.rs`), and workspace-internal
dependencies, per spec §23.6. Run `make codemap` to regenerate; `make
codemap-check` fails if regeneration would change this file.

EOF

while IFS= read -r row; do
  name=$(jq -r '.name' <<< "$row")
  purpose=$(jq -r '.purpose' <<< "$row")
  lib_rs=$(jq -r '.src_lib' <<< "$row")
  depends_on=$(jq -r '.depends_on | if length == 0 then "(none)" else join(", ") end' <<< "$row")
  used_by=$(jq -r '.used_by | if length == 0 then "(none)" else join(", ") end' <<< "$row")

  [ -n "$purpose" ] || purpose="(no description in Cargo.toml)"

  printf 'CRATE: %s\n' "$name"
  printf '  PURPOSE: %s\n' "$purpose"
  printf '  PUBLIC API:\n'
  if [ -f "$lib_rs" ]; then
    extract_public_api "$lib_rs" || printf '    (none)\n'
  else
    printf '    (no src/lib.rs)\n'
  fi
  printf '  DEPENDS ON: %s\n' "$depends_on"
  printf '  USED BY: %s\n' "$used_by"
  printf '\n'
done <<< "$crates_ndjson"
