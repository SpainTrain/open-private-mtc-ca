# codemap-crates.jq — cargo-metadata (--format-version 1, WITH the resolve
# graph) reduced to one compact JSON object per workspace crate, consumed by
# scripts/codemap-gen.sh.
#
# DEPENDS ON / USED BY come from `.resolve.nodes[].deps` — the real resolved
# dependency graph, not a re-parse of each Cargo.toml — restricted to
# workspace-internal packages (external crates like tokio/serde are not part
# of the navigable CODEMAP graph). USED BY is DEPENDS ON inverted, so it is
# always real dependent-crate names, never a placeholder.
. as $meta
| ($meta.workspace_members) as $wsids
| ($meta.packages | map(select(.id as $id | $wsids | index($id) != null))) as $wspkgs
| (reduce $wspkgs[] as $p ({}; .[$p.id] = $p.name)) as $idname
| ($meta.resolve.nodes | map(select(.id as $id | $wsids | index($id) != null))) as $wsnodes
| (reduce $wsnodes[] as $n ({};
     .[$n.id] = ([$n.deps[] | select(.pkg as $p | $wsids | index($p) != null) | $idname[.pkg]] | unique)
   )) as $deps_by_id
| (reduce ($deps_by_id | to_entries[]) as $e ({};
     reduce $e.value[] as $depname (.;
       .[$depname] = ((.[$depname] // []) + [$idname[$e.key]])
     )
   )) as $usedby_by_name
| [ $wspkgs[] | {
    name: .name,
    purpose: ((.description // "") | gsub("\\s+"; " ") | ltrimstr(" ") | rtrimstr(" ")),
    src_lib: ((.manifest_path | rtrimstr("Cargo.toml")) + "src/lib.rs"),
    depends_on: ($deps_by_id[.id] // []),
    used_by: (($usedby_by_name[.name] // []) | unique)
  } ]
| sort_by(.name)
| .[]
