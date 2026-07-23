# update-codemap-on-structure-change

> Spec: §23.6 (Scaling beyond context windows: code map).

## Rule

Run `make codemap` after creating or moving crates (or otherwise changing
workspace structure), and commit the regenerated `CODEMAP.md` in the same PR.

## Rationale

§23.6: `make codemap` generates `CODEMAP.md` — a single-file summary of all
packages, their exports, and dependencies — checked into the repo so agents can
navigate without loading all files. The repo will grow beyond any single
context window; the code map is a primary navigation affordance. A stale map is
worse than none: an agent trusting it will look for crates that moved and miss
crates that exist, burning tokens re-discovering structure the map was meant to
provide.

## Compliant example

```console
$ cargo new crates/tile-cache --lib     # new crate added to the workspace
$ make codemap                          # regenerate
$ git add crates/tile-cache CODEMAP.md  # map committed with the change
```

`CODEMAP.md` now contains the §23.6-format entry:

```text
CRATE: tile-cache
  PURPOSE: LRU tile cache for proof serving
  PUBLIC API: ...
  DEPENDS ON: types
  USED BY: ca-service
```

## Non-compliant example

```console
$ git mv crates/storage crates/storage-aws   # crate moved
$ git commit -am "rename storage crate"      # CODEMAP.md still lists 'storage'
```

## Enforcement

- **CI gate**: `CODEMAP.md` is regenerated in CI and diffed; drift between the
  committed map and the workspace fails the check (§23.6 — updated
  automatically; checked into repo).
- **Review**: structure-changing PRs without a `CODEMAP.md` diff are sent
  back.
