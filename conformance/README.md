# Spec conformance suite

Spec §19.4; roadmap Phase 1 "Spec conformance suite scaffolding" (ticket
`test-conformance-runner`). An ongoing CI gate proving the `mtc` crate's
wire-format parsing, serialization, and verification behave as this
repository's own implementation intends — a ratchet against silent
regressions in the hand-written TLS-presentation codec (spec §19.3's named
risk area) and the checkpoint/proof verification logic.

- **`vectors/`** — the test-vector fixtures. See
  [`vectors/README.md`](vectors/README.md) for the JSON+hex format and how to
  add a vector.
- **`crates/conformance/`** (`mtc-conformance`, workspace root) — the runner:
  discovers every vector, evaluates it against the real `mtc` types, and
  reports a per-vector pass/fail plus a structural diff on mismatch. See that
  crate's `src/lib.rs` for the architecture (schema / loader / runner
  modules) and `examples/generate_vectors.rs` for how the seed vectors were
  produced.

## Running it

```sh
make test-conformance   # or: make conformance (alias)
```

or directly:

```sh
cargo test -p mtc-conformance --all-features -- --nocapture
```

`--nocapture` is what makes the per-vector `PASS`/`FAIL [kind] id` lines and
the trailing `conformance suite: N passed, M failed, T total` line show up
even on a fully green run (`cargo test` otherwise captures a passing test's
stdout).

## Why this is already a required CI check

`crates/conformance/` is an ordinary workspace member with an ordinary
`#[test]` in `tests/conformance.rs`. The repository's `test` job already runs
`cargo test --workspace --all-features` on every PR (`.github/workflows/ci.yml`,
documented as a required check in `docs/ci.md`) — that command builds and
runs every workspace crate's tests, this one included, with no separate CI
job, workflow edit, or branch-protection change needed. This is what spec
§19.4's AC "Suite runs on every PR under `cargo test --all-features`" means
in practice here: the gate already existed at the workspace level; this
ticket's job was to make sure the conformance suite lives inside it.

## Clean-room vectors vs. draft vectors (scope note)

The vectors seeded here are clean-room — their bytes come from this
repository's own `mtc` serializers (`examples/generate_vectors.rs`), never
hand-invented. Byte-exact conformance against the
`draft-ietf-plants-merkle-tree-certs-03` text itself is a separate, tracked
obligation (bead `mtc-qka.5`); this suite is the harness that will host those
draft-derived vectors once they land — see `vectors/README.md`'s "Clean-room
vectors vs. draft vectors" section for exactly how they are expected to slot
in.

## Out of scope (this ticket)

- Cross-language differential comparison against `bwesterb/mtc` — a separate
  bead (`test-differential-runner` in `docs/planning/epics/testing-infra.json`).
- Vectors for spec structures this repository does not implement yet — added
  by the component tickets that implement them, in this same format.
